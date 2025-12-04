use std::{
    alloc::{Layout, alloc, dealloc},
    marker::PhantomData,
    ptr::{self, NonNull},
    slice,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::errors::PicoError;

#[repr(C, packed)]
struct EntryHeader {
    seq_id: u64,
    ts_nanos: u64,
    len: u16,
    _pad: [u8; 6],
}

pub struct Page<T> {
    pub ptr: NonNull<u8>,
    layout: Layout,
    block_size: usize,
    cursor: usize,
    last_entry: u64,
    _frankenstein: PhantomData<T>,
}

fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

unsafe impl<T: Send> Send for Page<T> {}

impl<T> Page<T> {
    pub fn init(block_size: usize) -> Self {
        let layout = Layout::from_size_align(block_size, block_size).unwrap();
        let ptr = unsafe { alloc(layout) };
        let ptr = NonNull::new(ptr).expect("Mem alloc failed");
        unsafe {
            ptr.as_ptr().write_bytes(0, block_size);
        }
        Self {
            ptr,
            layout,
            block_size,
            cursor: 0,
            last_entry: 0,
            _frankenstein: PhantomData,
        }
    }

    pub fn append(&mut self, seq_id: u64, data: &T) -> Result<(), PicoError> {
        let msg_size = std::mem::size_of::<T>();
        let header_size = std::mem::size_of::<EntryHeader>();
        let total_size = header_size + msg_size;
        let aligned_size = align_up(total_size, 8);

        if self.cursor + total_size > self.block_size {
            return Err(PicoError::PageFull {});
        }

        //TODO: Use a faster method to fetch monotonic val
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;

        let header = EntryHeader {
            seq_id,
            ts_nanos: now,
            len: msg_size as u16,
            _pad: [0; 6],
        };

        unsafe {
            let dest_ptr = self.ptr.as_ptr().add(self.cursor);
            ptr::write(dest_ptr as *mut EntryHeader, header);

            ptr::copy_nonoverlapping(
                data as *const T as *const u8,
                dest_ptr.add(header_size),
                msg_size,
            );

            let padding_bytes = aligned_size - total_size;
            if padding_bytes > 0 {
                ptr::write_bytes(dest_ptr.add(total_size), 0, padding_bytes);
            }

            self.cursor += aligned_size;
        }
        self.last_entry = seq_id;
        Ok(())
    }

    pub fn reset(&mut self) {
        unsafe {
            self.ptr.as_ptr().write_bytes(0, self.block_size);
        }
        self.cursor = 0;
        self.last_entry = 0;
    }

    pub fn is_empty(&self) -> bool {
        self.cursor == 0
    }

    pub fn get_last_entry(&self) -> u64 {
        self.last_entry
    }

    pub fn get_page_content(&self) -> &[u8] {
        unsafe {
            let raw_ptr = self.ptr.as_ptr();
            slice::from_raw_parts(raw_ptr, self.block_size)
        }
    }
}

impl<T> Drop for Page<T> {
    fn drop(&mut self) {
        unsafe { dealloc(self.ptr.as_ptr(), self.layout) }
    }
}
