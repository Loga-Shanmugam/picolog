use crate::errors::PicoError;
use crate::global::{next_page_id, set_ack_number};
use crate::{LogBuffer, PageManager};
use crossbeam_channel::{Receiver, RecvTimeoutError};
use io_uring::{IoUring, opcode, types};
use std::fs::File;
use std::os::unix::io::AsRawFd;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct LogWorker<'a, T> {
    pub receiver: Receiver<usize>,
    pub data_buffer: Arc<LogBuffer<T>>,
    pub pages: PageManager<T>,
    pub last_flush: Instant,
    pub flush_interval: Duration,
    pub poll_interval: Duration,
    pub logfile: &'a File,
    pub ring: IoUring,
    pub pending_writes: usize,
}

impl<'a, T> LogWorker<'a, T> {
    pub fn run(&mut self) {
        loop {
            self.process_completions();

            if self.last_flush.elapsed() >= self.flush_interval {
                self.flush_current_page();
            }

            let time_since_flush = self.last_flush.elapsed();

            let time_until_flush = if time_since_flush > self.flush_interval {
                Duration::ZERO
            } else {
                self.flush_interval - time_since_flush
            };

            let timeout = std::cmp::min(time_until_flush, self.poll_interval);

            match self.receiver.recv_timeout(timeout) {
                Ok(msg) => self.handle_message(msg),
                Err(RecvTimeoutError::Timeout) => {
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.flush_remaining();
                    break;
                }
            }
        }
    }

    fn handle_message(&mut self, idx: usize) {
        let log_msg = unsafe { &*self.data_buffer.inner[idx].get() };

        if let Err(PicoError::PageFull { .. }) =
            self.pages.get_active_page().append(log_msg.seq_id, &log_msg.data)
        {
            self.flush_current_page();
            let _ = self.pages.get_active_page().append(log_msg.seq_id, &log_msg.data);
        }
    }

    fn flush_current_page(&mut self) {
        let page_idx = self.pages.active_idx;
        let page = &self.pages.pages[page_idx];
        
        if page.is_empty() {
            self.last_flush = Instant::now();
            return;
        }

        self.pages.pending_status[page_idx] = true;

        let page_id = next_page_id();
        let offset = page_id * (page.get_page_content().len() as u64);
        let buf = page.get_page_content();

        let seq_id = page.get_last_entry();
        let user_data = ((page_idx as u64) << 56) | (seq_id & 0x00FF_FFFF_FFFF_FFFF);

        let write_e = opcode::Write::new(
            types::Fd(self.logfile.as_raw_fd()),
            buf.as_ptr(),
            buf.len() as _,
        )
        .offset(offset)
        .build()
        .user_data(user_data);

        unsafe {
            if self.ring.submission().push(&write_e).is_err() {
                self.ring.submit().expect("Fail to submit to clear SQ");

                self.ring
                    .submission()
                    .push(&write_e)
                    .expect("SQ full even after submit");
            }
        }

        let _ = self.ring.submit(); 
        self.pending_writes += 1;


        let _ = self.pages.advance();
        
        self.wait_if_next_page_pending();
        self.pages.get_active_page().reset();

        self.last_flush = Instant::now();
    }

    fn wait_if_next_page_pending(&mut self) {
        let idx = self.pages.active_idx;
        while self.pages.pending_status[idx] {
            self.ring.submit_and_wait(1).expect("failed to wait");
            self.process_completions();
        }
    }

    fn process_completions(&mut self) {
        let mut cq = self.ring.completion();
        while let Some(cqe) = cq.next() {
            if self.pending_writes > 0 {
                self.pending_writes -= 1;
            }
            if cqe.result() >= 0 {
                let user_data = cqe.user_data();
                let page_idx = (user_data >> 56) as usize;
                let seq_id = user_data & 0x00FF_FFFF_FFFF_FFFF;
                
                if page_idx < self.pages.pending_status.len() {
                    self.pages.pending_status[page_idx] = false;
                }
                
                set_ack_number(seq_id);
            } else {
                eprintln!("Async write failed: {}", cqe.result());
            }
        }
    }

    fn flush_remaining(&mut self) {
        self.flush_current_page();
        while self.pending_writes > 0 {
            self.ring.submit_and_wait(1).expect("failed to wait");
            self.process_completions();
        }
    }
}
