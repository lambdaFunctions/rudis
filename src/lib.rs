pub mod admin;
pub mod cmd;
pub mod resp;
pub mod stats;
pub mod store;
pub mod worker;

pub const BIND_ADDR: &str = "0.0.0.0:8080";
pub const LISTEN_BACKLOG: i32 = 128;
pub const CORES: &[usize] = &[0, 1];
pub const BUDGETS: &[(u64, u64)] = &[(900, 100), (900, 100)];
