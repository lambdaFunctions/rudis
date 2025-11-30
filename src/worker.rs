use crate::stats::Stats;
use crate::store::{CacheLevel, Store};
use crate::{cmd, resp};
use crate::BIND_ADDR;
use crate::LISTEN_BACKLOG;
use core_affinity::CoreId;
use socket2::{Domain, Socket, Type};
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::io::AsRawFd;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub fn run_worker(core_id: usize, read_budget_ms: u64, write_budget_ms: u64, stats: Arc<Stats>) {
    // Budget params reserved for future per-connection throttling.
    let _ = (read_budget_ms, write_budget_ms);

    pin_to_core(core_id);

    #[cfg(target_os = "linux")]
    unsafe {
        let mut sched_param: libc::sched_param = std::mem::zeroed();
        sched_param.sched_priority = 1;
        let _ = libc::sched_setscheduler(0, libc::SCHED_FIFO, &sched_param);
    }
    #[cfg(target_os = "macos")]
    unsafe {
        let mut sched_param: libc::sched_param = std::mem::zeroed();
        sched_param.sched_priority = 1;
        let _ = libc::pthread_setschedparam(libc::pthread_self(), libc::SCHED_FIFO, &sched_param);
    }

    let socket = Socket::new(Domain::IPV4, Type::STREAM, None).expect("create socket");
    set_socket_options(&socket, core_id).expect("set socket options");
    socket
        .bind(&BIND_ADDR.parse::<std::net::SocketAddr>().unwrap().into())
        .expect("bind");
    socket.listen(LISTEN_BACKLOG).expect("listen");

    println!("Core {} listener ready on {}", core_id, BIND_ADDR);

    let listener = std::net::TcpListener::from(std::net::TcpListener::try_from(socket).unwrap());

    // Store persists across connections — data survives client reconnects.
    let mut store = Store::for_level(CacheLevel::L2);
    let mut tmp = [0u8; 4096];

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                stats.connections_total.fetch_add(1, Ordering::Relaxed);
                stats.connections_active.fetch_add(1, Ordering::Relaxed);

                let mut buf: Vec<u8> = Vec::new();

                'conn: loop {
                    // Drain all fully-buffered commands before blocking on I/O.
                    // This handles pipelining: multiple commands in one read().
                    loop {
                        match resp::parse(&buf) {
                            Some((cmd_val, consumed)) => {
                                stats.packets_in.fetch_add(1, Ordering::Relaxed);
                                stats.bytes_in.fetch_add(consumed as u64, Ordering::Relaxed);
                                buf.drain(..consumed);

                                let reply = cmd::execute(&cmd_val, &mut store);
                                stats.packets_out.fetch_add(1, Ordering::Relaxed);
                                stats.bytes_out.fetch_add(reply.len() as u64, Ordering::Relaxed);

                                if stream.write_all(&reply).is_err() {
                                    stats.errors.fetch_add(1, Ordering::Relaxed);
                                    break 'conn;
                                }
                            }
                            None => break,
                        }
                    }

                    // Block until more data arrives.
                    match stream.read(&mut tmp) {
                        Ok(0) => break,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                        Err(e) => {
                            eprintln!("Core {} read error: {}", core_id, e);
                            stats.errors.fetch_add(1, Ordering::Relaxed);
                            break;
                        }
                    }
                }

                stats.connections_active.fetch_sub(1, Ordering::Relaxed);
                let _ = stream.shutdown(Shutdown::Both);
            }
            Err(e) => {
                stats.errors.fetch_add(1, Ordering::Relaxed);
                eprintln!("Core {} accept error: {}", core_id, e);
            }
        }
    }
}

fn pin_to_core(core_id: usize) {
    let core = CoreId { id: core_id };
    if !core_affinity::set_for_current(core) {
        eprintln!("Warning: failed to pin thread to core {}", core_id);
    } else {
        println!("Pinned thread {:?} to core {}", std::thread::current().id(), core_id);
    }
}

fn set_socket_options(
    socket: &Socket,
    #[cfg_attr(not(target_os = "linux"), allow(unused_variables))] cpu_id: usize,
) -> std::io::Result<()> {
    let reuse_port: libc::c_int = 1;
    unsafe {
        let ret = libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_REUSEPORT,
            &reuse_port as *const _ as *const libc::c_void,
            std::mem::size_of_val(&reuse_port) as libc::socklen_t,
        );
        if ret != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }

    #[cfg(target_os = "linux")]
    {
        let cpu = cpu_id as libc::c_int;
        unsafe {
            let ret = libc::setsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_INCOMING_CPU,
                &cpu as *const _ as *const libc::c_void,
                std::mem::size_of_val(&cpu) as libc::socklen_t,
            );
            if ret != 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
    }
    Ok(())
}
