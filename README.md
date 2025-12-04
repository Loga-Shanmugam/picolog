# Picolog: Crash-Consistent HFT Logger

A zero-copy, lock-free, log sequencer for high-frequency trading engines.


## Overview

Picolog is a specialized Write-Ahead Log (WAL) designed for financial systems where data loss is unacceptable and latency is critical. Unlike general-purpose loggers (Log4j, spdlog) that buffer data in userspace or kernel page cache, Picolog enforces Strict Durability using O_DIRECT and io_uring while maintaining nanosecond-level producer latency.

It serves as the persistence backbone for deterministic trading sequencers (like Raft or Aeron), decoupling the Matching Engine (Hot Path) from the NVMe SSD (Cold Path).

⚠️ Under active development

## Key Features

1. Zero-Copy Serialization: Uses repr(C) casting to write structs directly to page buffers, eliminating heap allocations (malloc) in the hot path.

2. Crash Consistency: Bypasses OS Page Cache (O_DIRECT). Data is physically committed to NVMe before an ACK is issued.

3. Double-Buffered Async I/O: Leverages io_uring to flush pages asynchronously. The trading thread never blocks on disk I/O unless the ring buffer is saturated.

4. Lock-Free SPSC: Single-Producer-Single-Consumer architecture using atomic cursors for thread-safe, lock-free communication.

5. Page-Aligned Memory: Custom slab allocator ensures all writes are aligned for block size to meet strict kernel Direct I/O requirements.

## Architecture

1. Picolog decouples the Application Thread (Producer) from the Persistence Thread (Consumer) using a shared memory ring buffer.

2. Producer (Hot Path): The trading engine writes a trade to the LogBuffer. This is a memcpy operation.

3. Consumer (Cold Path): The background thread polls the buffer, batches messages into 4KB pages, and submits them to the Linux Kernel via io_uring.

4. Persistence: The kernel performs a DMA transfer directly from the user-space buffer to the NVMe controller (Zero Copy).

5. Ack: Once the disk confirms the write, an Atomic High-Water Mark is updated. The trading engine polls this mark to confirm trades to clients (Group Commit).


## Benchmarks

Benchmarks performed on an AWS i3en.xlarge instance (4 vCPUs, 32GB RAM, 2 TB NVMe SSD).

### Scenario: High Throughput Burst

* Payload: 100 Bytes (Typical SBE Trade Message)
* Buffer: 4KB
* flush interval: 1ms
* poll interval: 10µs
* Total Ops: 8,698,537
* Duration: 20.01s

| Metric | Result | Description |
| :--- | :--- | :--- |
| **Throughput** | **434,704.22 Op/s** | Sustained durable write rate. |
| **P50 Submission** | **87.00 ns** | Time to copy data to ring buffer. |
| **P95 Submission** | **476.00 ns** | Tail latency for submission. |
| **P99 Submission** | **2.87 µs** | Worst-case blocking time for Trading Engine. |
| **P50 Disk Latency** | **13.00 ms** | Median time to physical NVMe persistence. |
| **P95 Disk Latency** | **21.78 ms** | 95th percentile disk persistence time. |
| **P99 Disk Latency** | **25.82 ms** | Worst-case time to physical NVMe persistence (Async). |

Analysis: The system successfully decouples the trading engine from the disk. While the disk takes ~13ms to persist the batch under load, the trading engine is only blocked for nanoseconds (P50: 87ns), allowing the strategy to continue processing market data without stalling.

## Usage


1. Installation
    Add to Cargo.toml:

    ```
    [dependencies]
    picologv2 = { path = "../picologv2" }
    ```


2. The Log Struct

    Define your log entry format. Must be repr(C) and Copy (POD) to ensure zero-copy safety.

    ```
    #[derive(Clone, Copy, Default)]
    #[repr(C)]
    struct Trade {
        price: u64,
        qty: u64,
        symbol_id: u32,
        side: u8,
        strategy_id: u32,
    }
    ```

3. Initialization

    Initialize the logger with a path, ring capacity, and flush policy.

    ```
    use picologv2::Logger;

    fn main() -> Result<(), Box<dyn std::error::Error>> {
        // 4096 slot ring buffer, flush every 10us, poll every 1us
        let mut logger = Logger::<Trade>::new()
            .with_write_config(
                "trades.wal".to_string(), 
                4096, 
                10_000,
                1_000
            );
        
        logger.start()?;

        let trade = Trade { price: 10050, qty: 100, symbol_id: 1, side: 1, strategy_id: 8};
        
        // Returns Some(seq_id) if successful, None if buffer full
        if let Some(seq) = logger.log(trade) {
            println!("Trade sequenced: {}", seq);
        }
        
        Ok(())
    }

    ```

4. Reading the Log

    Recover state by reading the log from the beginning.

    ```rust
    let logger = Logger::<Trade>::new()
        .with_read_config("trades.wal".to_string());

    let trades = logger.read()?;
    
    for trade in trades {
        println!("Recovered trade: price={}, qty={}", trade.price, trade.qty);
    }
    ```

## Design Decisions & Trade-offs

Why O_DIRECT?

Standard buffered I/O (fwrite) is faster but unsafe for financial data. If the OS crashes before flushing the Page Cache, trades are lost. O_DIRECT guarantees that when the write returns, the data is with the device controller.

Why Fixed-Size Pages?

O_DIRECT requires memory to be aligned to the disk sector size (usually 512 or 4096 bytes). Picolog manages a custom Slab Allocator that ensures all writes are perfectly aligned, avoiding expensive buffer copying in the kernel.

Why No Mutexes?

Locks cause context switches (futex), which cost ~1-2 microseconds. Picolog uses Atomic Cursors (AtomicU64) with Acquire/Release memory ordering to coordinate the Producer and Consumer threads, ensuring wait-free progress for the Producer.

## Future Roadmap

[ ] Implement Sparse Indexing (seq_id -> file_offset) for O(1) replay lookups.

[ ] Add CRC32 Checksums per page to detect torn writes on power loss. This provides the ability to write log messages greater than 4KB. (In the current implementation torn writes are not possible as NVMe guarantees ***all or nothing*** as long data size is equal to fs block size.)

[ ] Run the log worker in a isolated core to better utilize L1 and L2 cache.

[ ] Add proper documentation to all user facing functions.

