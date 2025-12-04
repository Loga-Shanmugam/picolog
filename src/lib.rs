mod util;
use crate::{
    global::next_seq_id,
    page::{Page, EntryHeader},
    util::{get_blksize, get_file_handler},
    worker::LogWorker,
};
use crossbeam_channel::Sender;
use std::cell::UnsafeCell;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use std::{io::Error, path::PathBuf};
use std::io::Read;
use std::ptr;

mod errors;
mod global;
mod page;
mod worker;

#[repr(C)]
#[derive(Clone, Default)]
/// A wrapper struct for log data that includes a sequence ID.
pub struct LogMessage<T> {
    /// Unique sequence identifier for the log message.
    pub seq_id: u64,
    /// The actual log data payload.
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

/// The main logger struct responsible for handling log writing and reading operations.
/// It uses a ring buffer and a background worker thread for asynchronous logging.
pub struct Logger<T> {
    data_buffer: Option<Arc<LogBuffer<T>>>,
    sender: Option<Sender<usize>>,
    worker_handle: Option<thread::JoinHandle<()>>,
    capacity: usize,
    logpath: Option<String>,
    flush_interval: Option<u64>,
    poll_interval: Option<u64>,
}

impl<T: Send + Sync + Default + Copy + 'static> Logger<T> {
    /// Creates a new instance of `Logger` with default (empty) configuration.
    pub fn new() -> Self {
        Self {
            data_buffer: None,
            sender: None,
            worker_handle: None,
            capacity: 0,
            logpath: None,
            flush_interval: None,
            poll_interval: None,
        }
    }

    /// Configures the logger for writing logs.
    ///
    /// # Arguments
    ///
    /// * `logpath` - Path to the log file.
    /// * `capacity` - Size of the ring buffer.
    /// * `flush_interval` - Interval in nanoseconds to flush logs to disk.
    /// * `poll_interval` - Interval in nanoseconds to poll for uring completions.
    pub fn with_write_config(mut self, logpath: String, capacity: usize, flush_interval: u64, poll_interval: u64) -> Self {
        self.logpath = Some(logpath);
        self.capacity = capacity;
        self.flush_interval = Some(flush_interval);
        self.poll_interval = Some(poll_interval);
        self
    }

    /// Initializes the internal components (buffer, worker thread) and starts the logging process.
    ///
    /// # Returns
    ///
    /// * `Result<(), Error>` - Ok if started successfully, Err if configuration is missing.
    pub fn start(&mut self) -> Result<(), Error> {
        if let (Some(logpath), Some(flush_interval), Some(poll_interval)) = (&self.logpath, self.flush_interval, self.poll_interval) {
            let capacity = self.capacity;
            let mut raw_vec = Vec::with_capacity(capacity);
            for _ in 0..capacity {
                raw_vec.push(UnsafeCell::new(LogMessage::default()));
            }

            let data_buffer = Arc::new(LogBuffer { inner: raw_vec });

            let (sender, receiver) = crossbeam_channel::bounded::<usize>(capacity);

            let path = PathBuf::from(logpath);
            let blk_size = get_blksize(&path) as usize;

            let worker_buffer = data_buffer.clone();

            let page_manager = PageManager::new(blk_size, 256);

            let file = get_file_handler(&path)?;
            let flush_interval_duration = flush_interval;
            let poll_interval_duration = poll_interval;

            let handle = thread::spawn(move || {
                let ring = io_uring::IoUring::new(256).expect("failed to init io_uring");
                let mut worker = LogWorker {
                    receiver,
                    pages: page_manager,
                    data_buffer: worker_buffer,
                    last_flush: Instant::now(),
                    flush_interval: Duration::from_nanos(flush_interval_duration),
                    poll_interval: Duration::from_nanos(poll_interval_duration),
                    logfile: &file,
                    ring,
                    pending_writes: 0,
                };
                worker.run();
            });

            self.data_buffer = Some(data_buffer);
            self.sender = Some(sender);
            self.worker_handle = Some(handle);
            
            Ok(())
        } else {
             Err(Error::new(std::io::ErrorKind::InvalidInput, "Config missing"))
        }
    }

    /// Configures the logger for reading logs.
    ///
    /// # Arguments
    ///
    /// * `logpath` - Path to the log file to read from.
    pub fn with_read_config(mut self, logpath: String) -> Self {
        self.logpath = Some(logpath);
        self
    }

    /// Reads all log entries from the configured log file.
    ///
    /// # Returns
    ///
    /// * `Result<Vec<T>, Error>` - A vector of log data if successful, or an error.
    pub fn read(&self) -> Result<Vec<T>, Error> {
        let logpath = self.logpath.as_ref().ok_or(Error::new(std::io::ErrorKind::NotFound, "Log path not configured"))?;
        let mut file = std::fs::File::open(logpath)?;
        let mut vec = Vec::new();
        let path = PathBuf::from(logpath);
        let blk_size = get_blksize(&path) as usize;
        
        let mut buffer = vec![0u8; blk_size];
        
        loop {
            let bytes_read = file.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            
            let mut cursor = 0;
            while cursor < bytes_read {
                if cursor + std::mem::size_of::<EntryHeader>() > bytes_read {
                    break; 
                }
                
                let header_ptr = unsafe { buffer.as_ptr().add(cursor) as *const EntryHeader };
                let header = unsafe { ptr::read_unaligned(header_ptr) };
                
                if header.len == 0 {
                    break;
                }
                
                let msg_size = header.len as usize;
                let header_size = std::mem::size_of::<EntryHeader>();
                let total_size = header_size + msg_size;
                let aligned_size = (total_size + 7) & !7;
                
                if cursor + total_size > bytes_read {
                    break;
                }
                
                let data_ptr = unsafe { buffer.as_ptr().add(cursor + header_size) as *const T };
                let data = unsafe { ptr::read_unaligned(data_ptr) };
                vec.push(data);
                
                cursor += aligned_size;
            }
        }
        Ok(vec)
    }

    /// Adds a new log entry to the buffer.
    ///
    /// # Arguments
    ///
    /// * `data` - The log data to be written.
    ///
    /// # Returns
    ///
    /// * `Option<u64>` - The sequence ID of the log entry if successful, or `None` if the logger is not started.
    pub fn log(&mut self, data: T) -> Option<u64> {
        if let Some(sender) = &self.sender {
            let seq_id = next_seq_id();
            let index = (seq_id as usize) % self.capacity;

            if let Some(data_buffer) = &self.data_buffer {
                unsafe {
                    let ptr = data_buffer.inner[index].get();
                    (*ptr).seq_id = seq_id;
                    (*ptr).data = data;
                }
            }

            let _ = sender.send(index);
            return Some(seq_id);
        }
        return None;
    }

    /// Retrieves the sequence ID of the last log entry that was successfully flushed to disk.
    ///
    /// # Returns
    ///
    /// * `u64` - The sequence ID.
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
