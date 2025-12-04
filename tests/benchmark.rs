use crossbeam_channel::unbounded;
use picologger::Logger;
use std::fs;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
#[repr(C)]
struct Data {
    val: u64,
    ts: u64,
    padding: [u8; 84],
}

impl Default for Data {
    fn default() -> Self {
        Self {
            val: 0,
            ts: 0,
            padding: [0; 84],
        }
    }
}

#[test]
fn benchmark_throughput_latency() {
    let path = "bench_test.log";
    if std::path::Path::new(path).exists() {
        fs::remove_file(path).unwrap();
    }

    let mut logger = Logger::<Data>::new()
        .with_write_config(path.to_string(), 4096, 1_000_000, 10_000);
    logger.start().unwrap();

    let (tx, rx) = unbounded::<(u64, Instant)>();

    let monitor_handle = thread::spawn(move || {
        let mut disk_latencies = Vec::with_capacity(100_000);
        while let Ok((seq_id, start_ts)) = rx.recv() {
            while Logger::<Data>::get_last_flushed_entry() < seq_id {
                thread::yield_now();
            }
            disk_latencies.push(start_ts.elapsed());
        }
        disk_latencies
    });

    let start_bench = Instant::now();
    let run_duration = Duration::from_secs(20);
    let mut latencies = Vec::with_capacity(7_000_000);
    let mut count = 0;

    while start_bench.elapsed() < run_duration {
        let iter_start = Instant::now();
        let seq_id = logger.log(Data {
            val: count,
            ts: 0,
            padding: [0; 84],
        });
        let iter_end = Instant::now();
        latencies.push(iter_end.duration_since(iter_start));

        if count % 1000 == 0 {
            if let Some(id) = seq_id {
                let _ = tx.send((id, iter_start));
            }

            //Simulating slightly burst workload by sleeping at random time between 1 and 5 microseconds
            let sleep_duration = Duration::from_micros(fastrand::u64(1..5));
            thread::sleep(sleep_duration);
        }

        count += 1;
    }

    drop(tx);
    let mut disk_latencies = monitor_handle.join().unwrap();

    let total_duration = start_bench.elapsed();

    latencies.sort();
    let len = latencies.len();
    let p50 = latencies[(len as f64 * 0.50) as usize];
    let p95 = latencies[(len as f64 * 0.95) as usize];
    let p99 = latencies[(len as f64 * 0.99) as usize];

    disk_latencies.sort();
    let d_len = disk_latencies.len();
    let d_p50 = if d_len > 0 {
        disk_latencies[(d_len as f64 * 0.50) as usize]
    } else {
        Duration::ZERO
    };
    let d_p95 = if d_len > 0 {
        disk_latencies[(d_len as f64 * 0.95) as usize]
    } else {
        Duration::ZERO
    };
    let d_p99 = if d_len > 0 {
        disk_latencies[(d_len as f64 * 0.99) as usize]
    } else {
        Duration::ZERO
    };

    println!("Total Ops: {}", count);
    println!("Duration: {:.2?}", total_duration);
    println!(
        "Throughput: {:.2} Op/s",
        count as f64 / total_duration.as_secs_f64()
    );
    println!("Submission Latency (P50): {:.2?}", p50);
    println!("Submission Latency (P95): {:.2?}", p95);
    println!("Submission Latency (P99): {:.2?}", p99);
    println!("Disk Latency (P50): {:.2?}", d_p50);
    println!("Disk Latency (P95): {:.2?}", d_p95);
    println!("Disk Latency (P99): {:.2?}", d_p99);

    drop(logger);
    if std::path::Path::new(path).exists() {
        fs::remove_file(path).unwrap();
    }
}
