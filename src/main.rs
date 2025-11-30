use std::thread;

use rudis::stats::Stats;
use rudis::worker::run_worker;
use rudis::{BUDGETS, CORES};


fn main() {
    let stats = Stats::new();
    let mut handles = vec![];

    for (i, &core_id) in CORES.iter().enumerate() {
        let (read_budget, write_budget) = BUDGETS[i];
        let stats = std::sync::Arc::clone(&stats);
        let handle = thread::spawn(move || {
            run_worker(core_id, read_budget, write_budget, stats);
        });
        handles.push(handle);
    }

    println!("All worker threads started. Press Ctrl+C to stop.");

    for handle in handles {
        let _ = handle.join();
    }
}
