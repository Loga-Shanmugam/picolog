use std::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE_ID: AtomicU64 = AtomicU64::new(0);
static ACK_NUMBER: AtomicU64 = AtomicU64::new(0);
static PAGE_ID: AtomicU64 = AtomicU64::new(0);

pub fn next_seq_id() -> u64 {
    SEQUENCE_ID.fetch_add(1, Ordering::Relaxed)
}


pub fn get_ack_number() -> u64 {
    ACK_NUMBER.load(Ordering::Acquire)
}

pub fn set_ack_number(val: u64) {
    ACK_NUMBER.fetch_max(val, Ordering::Release);
}

pub fn next_page_id() -> u64 {
    PAGE_ID.fetch_add(1, Ordering::Relaxed)
}
