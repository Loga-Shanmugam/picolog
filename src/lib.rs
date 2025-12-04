mod util;
use crate::{
    global::next_seq_id,
    page::Page,
    util::{get_blksize, get_file_handler},
    worker::LogWorker,
};
use crossbeam_channel::Sender;
use std::cell::UnsafeCell;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use std::{io::Error, path::PathBuf};
mod errors;
mod global;
mod page;
mod worker;

#[repr(C)]
#[derive(Clone, Default)]
pub struct LogMessage<T> {
    pub seq_id: u64,
    pub data: T,
}

struct PageManager<T> {
    pages: Vec<Page<T>>,
    active_idx: usize,
    pending_status: Vec<bool>,
}

impl<T> PageManager<T> {
    pub fn new(page_size: usize, count: usize) -> Self {
        let mut pages = Vec::with_capacity(count);
        let mut pending_status = Vec::with_capacity(count);
        for _ in 0..count {
            pages.push(Page::init(page_size));
            pending_status.push(false);
        }
        Self {
            pages,
            active_idx: 0,
            pending_status,
        }
    }

    pub fn get_active_page(&mut self) -> &mut Page<T> {
        &mut self.pages[self.active_idx]
    }

    pub fn advance(&mut self) -> usize {
        let prev = self.active_idx;
        self.active_idx = (self.active_idx + 1) % self.pages.len();
        prev
    }
}

struct LogBuffer<T> {
    inner: Vec<UnsafeCell<LogMessage<T>>>,
}

unsafe impl<T: Send + Sync> Sync for LogBuffer<T> {}
unsafe impl<T: Send + Sync> Send for LogBuffer<T> {}

pub struct Logger<T> {
    data_buffer: Arc<LogBuffer<T>>,
    sender: Option<Sender<usize>>,
    worker_handle: Option<thread::JoinHandle<()>>,
    capacity: usize,
}

impl<T: Send + Sync + Default + Copy + 'static> Logger<T> {
    pub fn new(
        logpath: String,
        capacity: usize,
        flush_interval: u64,
    ) -> Result<Self, Error> {
        let mut raw_vec = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            raw_vec.push(UnsafeCell::new(LogMessage::default()));
        }

        let data_buffer = Arc::new(LogBuffer { inner: raw_vec });

        let (sender, receiver) = crossbeam_channel::bounded::<usize>(capacity);

        let path = PathBuf::from(logpath);
        let blk_size = get_blksize(&path) as usize;

        let worker_buffer = data_buffer.clone();

        let page_manager = PageManager::new(blk_size, 128);

        let file = get_file_handler(&path)?;
        let handle = thread::spawn(move || {
            let ring = io_uring::IoUring::new(256).expect("failed to init io_uring");
            let mut worker = LogWorker {
                receiver,
                pages: page_manager,
                data_buffer: worker_buffer,
                last_flush: Instant::now(),
                flush_interval: Duration::from_nanos(flush_interval),
                logfile: &file,
                ring,
                pending_writes: 0,
            };
            worker.run();
        });
        Ok(Self {
            data_buffer,
            sender: Some(sender),
            worker_handle: Some(handle),
            capacity,
        })
    }
    pub fn log(&mut self, data: T) -> Option<u64> {
        if let Some(sender) = &self.sender {
            let seq_id = next_seq_id();
            let index = (seq_id as usize) % self.capacity;

            unsafe {
                let ptr = self.data_buffer.inner[index].get();
                (*ptr).seq_id = seq_id;
                (*ptr).data = data;
            }

            let _ = sender.send(index);
            return Some(seq_id);
        }
        return None;
    }

    pub fn get_last_flushed_entry() -> u64 {
        global::get_ack_number()
    }
}

impl<T> Drop for Logger<T> {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            drop(sender);
        }

        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}
