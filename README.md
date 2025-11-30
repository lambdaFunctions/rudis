# Rudis

A Redis-compatible in-memory key-value store built to explore high-performance systems patterns in Rust. The driving question is: *how close to hardware speed can a userspace TCP server get?*

This is an educational project. It deliberately trades completeness for clarity — each design decision exists to demonstrate a specific systems concept.

---

## Architecture

```
                          NIC
                           │
               ┌───────────┴───────────┐
               │  SO_REUSEPORT kernel  │
               │  load-balancer        │
               └───────────┬───────────┘
                           │
            ┌──────────────┼──────────────┐
            │              │              │
       Core 0         Core 1         Core N
      Worker 0       Worker 1       Worker N
      Store  0       Store  1       Store  N
      Socket 0       Socket 1       Socket N
            │              │              │
            └──────────────┼──────────────┘
                           │
                    RESP clients
```

Each worker thread is pinned to a single CPU core and owns its own listening socket, store, and connection state. There is **no shared mutable state** between workers — and therefore no locks, no cache-line bouncing between cores.

---

## Key Design Decisions

### 1. Perfect Core Locality

The ideal is for every step of a request — from the NIC interrupt, through TCP processing, to data access — to execute on a single CPU core. When this holds, the working set stays in L1/L2 cache throughout and there is no need to invalidate cache lines across cores (the MESI protocol is never triggered between workers).

The implementation achieves this with two mechanisms working together:

- **CPU pinning** (`core_affinity`): each worker thread is bound to a specific core. The OS scheduler will never migrate it.
- **Socket affinity** (`SO_INCOMING_CPU`, Linux): the kernel is told which CPU core's socket should receive new connections. When a packet arrives, the kernel routes it to the socket whose thread is already on the same core that handled the NIC interrupt — so no cross-core hand-off happens at any point.

**Caveat**: perfect isolation also requires pinning NIC Rx queues to the same cores. This can be done manually:

```bash
# Find your NIC's IRQ numbers
cat /proc/interrupts | grep eth0

# Bind each Rx queue to the matching core (bitmask: 1=core0, 2=core1, 4=core2…)
echo 1 > /proc/irq/<irq_number>/smp_affinity

# Disable irqbalance so the kernel doesn't override your settings
systemctl stop irqbalance
```

Without this step, the NIC may still deliver interrupts to a different core, causing one cross-core transition per request.

---

### 2. Lock-Free Multi-Socket Routing (SO_REUSEPORT)

The traditional approach to multi-core TCP servers is a single listening socket with an `accept()` mutex. Every new connection requires acquiring that lock, which serialises the accept path and creates a contention hotspot.

Rudis avoids this entirely. Each worker creates its **own** socket, binds it to the same `IP:port`, and calls `listen()` independently. `SO_REUSEPORT` tells the kernel this is intentional. The kernel then distributes incoming connections across all these sockets using a hash of the 4-tuple (src IP, src port, dst IP, dst port), guaranteeing that packets from the same TCP flow always land on the same socket (and therefore the same core).

The result: `accept()` is completely lock-free. Each core can accept connections at full speed without any coordination with the others.

```rust
// Each worker sets this on its own socket before bind()
let reuse_port: libc::c_int = 1;
libc::setsockopt(fd, SOL_SOCKET, SO_REUSEPORT, &reuse_port, ...);
```

---

### 3. Real-Time Thread Scheduling (SCHED_FIFO)

By default, Linux uses the CFS (Completely Fair Scheduler) or its successor EEVDF, which will preempt a worker thread mid-request to give CPU time to other processes. This introduces unpredictable latency spikes.

Setting `SCHED_FIFO` (priority 1) moves the worker thread to the real-time scheduling class. A `SCHED_FIFO` thread is never preempted by normal-priority processes — it runs until it voluntarily blocks (e.g., on a `read()` call). This significantly reduces p99 tail latency.

```rust
// Linux
let mut param: libc::sched_param = std::mem::zeroed();
param.sched_priority = 1;
libc::sched_setscheduler(0, libc::SCHED_FIFO, &param);

// macOS equivalent
libc::pthread_setschedparam(libc::pthread_self(), libc::SCHED_FIFO, &param);
```

This requires elevated privileges. The call fails silently if the process lacks them — the worker still runs, just without the real-time guarantee.

---

### 4. Cache-Aware Store Design

Two cache-related decisions govern the store:

**Capacity bounded by L2 size.** The working set for each core's store is capped at 256 KB by default — matching a typical L2 cache. If the hot data fits in L2, reads are served in ~3–10 ns. Spilling into L3 costs ~30 ns; going to DRAM costs ~100–300 ns. The cap is a deliberate tradeoff: a smaller working set is faster to serve, even if it means rejecting writes once capacity is reached.

```rust
pub const L1_BYTES: usize = 32 * 1024;   //  32 KB
pub const L2_BYTES: usize = 256 * 1024;  // 256 KB — default store size
pub const L3_BYTES: usize = 8 * 1024 * 1024; // 8 MB
```

**False-sharing prevention via `CachePadded`.** If multiple `Store` instances are laid out in memory, their internal fields could share a 64-byte cache line. A write on core 0 would then force core 1 to re-fetch that line from L3, negating the locality work. Wrapping the inner struct in `crossbeam::CachePadded<T>` pads it to a full cache line, ensuring writes are always isolated.

```rust
pub struct Store(CachePadded<Inner>);
```

---

### 5. RESP Protocol with Pipelining

RESP (REdis Serialization Protocol) is a simple binary-framed protocol. Each message is prefixed with its type (`+`, `-`, `:`, `$`, `*`) and, for bulk strings and arrays, its byte length. This gives O(1) field extraction — no scanning for delimiters, no copying to parse.

Compared to a text/JSON API, RESP is more cache-friendly on the hot path: the parser touches fewer bytes, does less work, and the resulting values fit more easily into registers.

The server also handles **pipelining**: a client can send multiple commands in one TCP segment without waiting for responses. The inner dispatch loop drains all fully-received commands before blocking on the next `read()`:

```rust
'conn: loop {
    // Dispatch all complete commands already in the buffer.
    loop {
        match resp::parse(&buf) {
            Some((cmd, consumed)) => {
                buf.drain(..consumed);
                let reply = cmd::execute(&cmd, &mut store);
                stream.write_all(&reply)?;
            }
            None => break, // need more data
        }
    }
    // Block until more bytes arrive.
    let n = stream.read(&mut tmp)?;
    buf.extend_from_slice(&tmp[..n]);
}
```

This means a client doing pipelined writes won't stall waiting for ACKs between commands — throughput stays high even over a loopback connection.

---

### 6. Lock-Free Monitoring with Relaxed Atomics

Statistics (packet counts, byte counts, connection counts) are tracked with `AtomicU64` using `Ordering::Relaxed`. This is intentional.

`Relaxed` imposes no happens-before relationship and generates no memory barriers. On x86-64, it compiles to a plain `lock xadd` or `lock inc` — a single instruction with no pipeline stall. Stronger orderings (`SeqCst`, `Acquire/Release`) would add full memory barriers that flush the store buffer and stall the CPU, which is unnecessary overhead for approximate monitoring counters.

The admin server on port 9090 reads these counters via `Arc<Stats>` without ever blocking the worker. The dashboard auto-refreshes every 2 seconds and also exposes a `/stats` JSON endpoint.

---

## Supported Commands

| Command | Signature | Description |
|---------|-----------|-------------|
| `PING` | `PING [message]` | Returns `PONG` or echoes the message |
| `SET` | `SET key value` | Store a key-value pair |
| `GET` | `GET key` | Retrieve a value by key |
| `DEL` | `DEL key [key …]` | Delete one or more keys; returns count |
| `EXISTS` | `EXISTS key [key …]` | Check existence; returns count |
| `DBSIZE` | `DBSIZE` | Number of keys in the store |
| `KEYS` | `KEYS` | List all keys |
| `FLUSHDB` | `FLUSHDB` | Clear all keys |

---

## Binaries

| Binary | Command | Description |
|--------|---------|-------------|
| `rudis` | `cargo run --bin rudis` | Start the server (workers only, no admin) |
| `worker` | `cargo run --bin worker` | Identical to `rudis`; standalone entry point |
| `admin` | `cargo run --bin admin` | Server + admin dashboard on http://localhost:9090 |
| `cli` | `cargo run --bin cli [host:port]` | Interactive REPL (behaves like `redis-cli`) |
| `client` | `cargo run --bin client` | Latency/throughput benchmark (PING flood) |

---

## Project Structure

```
src/
├── lib.rs          — module tree + global constants (BIND_ADDR, CORES, BUDGETS)
├── main.rs         — spawns one worker thread per configured core
├── worker.rs       — core loop: CPU pinning, socket setup, RESP dispatch
├── store.rs        — cache-padded HashMap with L1/L2/L3 capacity budgets
├── resp.rs         — RESP parser (streaming, handles partial reads) + encoders
├── cmd.rs          — command dispatch: maps RESP commands to store operations
├── stats.rs        — lock-free atomic counters + process RSS measurement
├── admin.rs        — minimal HTTP server: HTML dashboard + JSON /stats endpoint
└── bin/
    ├── worker.rs   — standalone worker entry point
    ├── admin.rs    — worker + admin server entry point
    ├── cli.rs      — interactive REPL client
    └── client.rs   — PING benchmark with p50/p99/max latency reporting
docs/
├── ram.md                     — cache line sizes, SIMD widths, write strategies
├── listeners_architectures.md — RSS, SO_REUSEPORT, SO_INCOMING_CPU, eBPF, XDP
└── make_data_l2_friendly.md   — strategies for keeping working sets in L2
```

---

## Possible Improvements

### Protocol & Correctness

- **Key expiration (TTL)**: `SET key value EX seconds` is the most-used Redis feature not yet implemented. Would require storing a deadline alongside each value, a lazy-expiry check on `GET`, and a background reaper thread that scans for expired keys at configurable intervals.

- **More data types**: Redis's power comes from its richer types — lists (`LPUSH`/`LRANGE`), sets (`SADD`/`SMEMBERS`), sorted sets (`ZADD`/`ZRANGE`), and hashes (`HSET`/`HGET`). Each maps to a different in-memory structure (doubly-linked list, hash set, skip list, nested hashmap) with different cache characteristics.

- **Transactions (MULTI/EXEC)**: Would require buffering a command queue per connection and executing it atomically with respect to the store.

- **Pub/Sub**: Needs a separate fan-out registry shared across connections (the one place where cross-connection state would be required).

---

### Performance

- **Read/write budget throttling**: `BUDGETS` constants in `lib.rs` define per-core read and write time limits (milliseconds per second). The values are passed to each worker but the throttling logic is not yet wired up. The intent is to prevent one connection from saturating a core's I/O budget and starving others — particularly relevant once non-blocking I/O is added.

- **Non-blocking I/O (epoll/kqueue)**: The current worker handles one connection at a time (synchronous `accept` → serve → next). Adding `epoll` (Linux) or `kqueue` (macOS) would let each worker multiplex thousands of concurrent connections without spawning threads, at the cost of a more complex event loop. This is the standard model for high-connection-count servers.

- **LRU eviction policy**: When the store is full, `SET` currently returns an error. A more practical policy — LRU, FIFO, or random sampling (as Redis does) — would allow continuous operation under memory pressure.

- **jemalloc**: Rust's default allocator can fragment under many small allocations (short keys and values). Plugging in `jemalloc` (as Redis does) would reduce fragmentation and improve allocator throughput.

- **Arena/pool allocation for small values**: Most keys and values are short strings. A slab allocator or arena per worker would reduce per-allocation overhead and improve cache locality for the allocator metadata itself.

- **Zero-copy parsing**: Currently, `resp::parse` copies bulk string data into `Vec<u8>`. Using a `Bytes`/`BytesMut` approach (e.g., the `bytes` crate) would allow the parser to hold a reference into the read buffer, eliminating allocations on the hot path.

---

### Hardware & Kernel

- **NIC IRQ affinity**: As described above, full core locality also requires binding each NIC Rx queue to the same core as the corresponding worker, and disabling `irqbalance`. Without this, the NIC interrupt may fire on a different core than the one serving the connection.

- **eBPF packet steering (`SO_ATTACH_REUSEPORT_EBPF`)**: Instead of the kernel's default 4-tuple hash, an eBPF program attached to the socket group can implement custom routing logic — e.g., route writes to core 0 and reads to cores 1–N, or balance based on key hash.

- **XDP (eXpress Data Path)**: Processes packets at the NIC driver level, before they enter the kernel networking stack. Eliminates one context switch and several memory copies per packet. Suitable for extremely latency-sensitive paths.

- **RDMA**: Bypasses the OS network stack entirely by having the NIC DMA data directly into a user-space buffer. Achieves sub-microsecond latency on capable hardware. A Rust + C implementation reference is in the [datenlord article](https://medium.com/@datenlord/implementing-an-rdma-userland-driver-3c29ae943bc3).

---

### Operational

- **Persistence**: All data is lost on restart. Redis uses two complementary strategies: RDB (point-in-time snapshots, compact, slower recovery) and AOF (append-only log, slower writes, faster/complete recovery). Either would require a background I/O thread to avoid blocking the worker.

- **TLS**: The current transport is plaintext TCP. Adding TLS (`rustls` or `native-tls`) is necessary for production use.

- **Configuration file**: Core count, bind address, store capacity, and budgets are currently hardcoded in `lib.rs`. A TOML configuration file would make the server deployable without recompilation.

- **Graceful shutdown**: `SIGTERM`/`SIGINT` are not handled. A clean shutdown would flush in-flight responses and optionally persist state before exiting.

- **Replication**: A primary-replica model would allow reads to scale horizontally and provide basic fault tolerance.

---

## References

- SO_REUSEPORT, SO_INCOMING_CPU, eBPF steering, XDP — `docs/listeners_architectures.md`
- Cache line sizes, SIMD alignment, write strategies — `docs/ram.md`
- L2-friendly data layout patterns — `docs/make_data_l2_friendly.md`
- Redis architecture overview — https://datasturdy.com/redis-architecture-a-detailed-exploration/
- RESP protocol specification — https://redis.io/docs/latest/develop/reference/protocol-spec/
- Redis benchmark tooling — https://www.digitalocean.com/community/tutorials/how-to-perform-redis-benchmark-tests
- jemalloc internals — https://jemalloc.net/jemalloc.3.html
