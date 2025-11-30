use rudis::admin::run_admin;
use rudis::stats::Stats;
use rudis::worker::run_worker;
use rudis::{BUDGETS, CORES};
use std::sync::Arc;
use std::thread;

fn main() {
    let stats = Stats::new();

    for (i, &core_id) in CORES.iter().enumerate() {
        let (read_budget, write_budget) = BUDGETS[i];
        let stats = Arc::clone(&stats);
        thread::spawn(move || {
            run_worker(core_id, read_budget, write_budget, stats);
        });
    }

    println!("Workers started on {} cores. Press Ctrl+C to stop.", CORES.len());

    run_admin(stats);
}
