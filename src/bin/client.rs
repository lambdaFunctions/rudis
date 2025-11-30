// Test client: mirrors the server's worker topology.
// Spawns one thread per server worker (one per CORE), each maintaining its own
// TCP connection. Every thread sends a fixed payload, reads back the echo, and
// reports round-trip latency + throughput when done.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::thread;
use std::time::{Duration, Instant};

const SERVER_ADDR: &str = "127.0.0.1:8080";

// Must match the CORES slice in main.rs so we have one client per worker.
const NUM_WORKERS: usize = 2;

// How many round-trips each client thread runs.
const REQUESTS_PER_CLIENT: usize = 1_000;

// Payload sent on every request.
const PAYLOAD: &[u8; 23] = b"PING_RUDIS_TEST_PAYLOAD";

fn run_client(worker_idx: usize) {
    let stream = TcpStream::connect(SERVER_ADDR)
        .unwrap_or_else(|e| panic!("client {} connect failed: {}", worker_idx, e));

    stream
        .set_nodelay(true)
        .expect("set_nodelay failed");

    let mut stream = stream;
    let mut buf = vec![0u8; PAYLOAD.len()];
    let mut latencies = Vec::with_capacity(REQUESTS_PER_CLIENT);

    for _ in 0..REQUESTS_PER_CLIENT {
        let t0 = Instant::now();

        stream
            .write_all(PAYLOAD)
            .unwrap_or_else(|e| panic!("client {} write failed: {}", worker_idx, e));

        let mut received = 0;
        while received < PAYLOAD.len() {
            let n = stream
                .read(&mut buf[received..])
                .unwrap_or_else(|e| panic!("client {} read failed: {}", worker_idx, e));
            if n == 0 {
                panic!("client {}: server closed connection early", worker_idx);
            }
            received += n;
        }

        latencies.push(t0.elapsed());

        assert_eq!(
            &buf[..PAYLOAD.len()],
            PAYLOAD,
            "client {}: echo mismatch",
            worker_idx
        );
    }

    let total: Duration = latencies.iter().sum();
    let avg_us = total.as_micros() / REQUESTS_PER_CLIENT as u128;

    latencies.sort_unstable();
    let p50 = latencies[REQUESTS_PER_CLIENT / 2].as_micros();
    let p99 = latencies[REQUESTS_PER_CLIENT * 99 / 100].as_micros();
    let max = latencies.last().unwrap().as_micros();

    let throughput_rps = REQUESTS_PER_CLIENT as f64 / total.as_secs_f64();

    println!(
        "client {:>2} | {:>6} reqs | avg {:>6}µs | p50 {:>6}µs | p99 {:>6}µs | max {:>6}µs | {:.0} req/s",
        worker_idx, REQUESTS_PER_CLIENT, avg_us, p50, p99, max, throughput_rps
    );
}

fn main() {
    println!(
        "Connecting {} client(s) to {} ({} requests each, {} bytes/payload)",
        NUM_WORKERS,
        SERVER_ADDR,
        REQUESTS_PER_CLIENT,
        PAYLOAD.len()
    );

    let handles: Vec<_> = (0..NUM_WORKERS)
        .map(|i| thread::spawn(move || run_client(i)))
        .collect();

    for handle in handles {
        handle.join().expect("client thread panicked");
    }

    println!("All clients done.");
}
