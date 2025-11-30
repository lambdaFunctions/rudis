use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

pub struct Stats {
    pub packets_in: AtomicU64,
    pub packets_out: AtomicU64,
    pub bytes_in: AtomicU64,
    pub bytes_out: AtomicU64,
    pub connections_total: AtomicU64,
    pub connections_active: AtomicI64,
    pub errors: AtomicU64,
    started_at: Instant,
}

pub struct Snapshot {
    pub uptime_secs: u64,
    pub packets_in: u64,
    pub packets_out: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub packets_in_per_sec: u64,
    pub packets_out_per_sec: u64,
    pub bytes_in_per_sec: u64,
    pub bytes_out_per_sec: u64,
    pub connections_active: u64,
    pub connections_total: u64,
    pub errors: u64,
    pub memory_rss_bytes: u64,
}

impl Stats {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            packets_in: AtomicU64::new(0),
            packets_out: AtomicU64::new(0),
            bytes_in: AtomicU64::new(0),
            bytes_out: AtomicU64::new(0),
            connections_total: AtomicU64::new(0),
            connections_active: AtomicI64::new(0),
            errors: AtomicU64::new(0),
            started_at: Instant::now(),
        })
    }

    pub fn uptime_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    pub fn snapshot(&self) -> Snapshot {
        let uptime = self.uptime_secs().max(1);
        let packets_in = self.packets_in.load(Ordering::Relaxed);
        let packets_out = self.packets_out.load(Ordering::Relaxed);
        let bytes_in = self.bytes_in.load(Ordering::Relaxed);
        let bytes_out = self.bytes_out.load(Ordering::Relaxed);

        Snapshot {
            uptime_secs: self.uptime_secs(),
            packets_in,
            packets_out,
            bytes_in,
            bytes_out,
            packets_in_per_sec: packets_in / uptime,
            packets_out_per_sec: packets_out / uptime,
            bytes_in_per_sec: bytes_in / uptime,
            bytes_out_per_sec: bytes_out / uptime,
            connections_active: self.connections_active.load(Ordering::Relaxed).max(0) as u64,
            connections_total: self.connections_total.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            memory_rss_bytes: process_rss_bytes(),
        }
    }
}

// Returns current process RSS. On macOS ru_maxrss is bytes; on Linux it's kilobytes.
fn process_rss_bytes() -> u64 {
    unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        if libc::getrusage(libc::RUSAGE_SELF, &mut usage) != 0 {
            return 0;
        }
        #[cfg(target_os = "macos")]
        { usage.ru_maxrss as u64 }
        #[cfg(target_os = "linux")]
        { usage.ru_maxrss as u64 * 1024 }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        { 0 }
    }
}
