use crate::stats::{Snapshot, Stats};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;

pub const ADMIN_ADDR: &str = "0.0.0.0:9090";

pub fn run_admin(stats: Arc<Stats>) {
    let listener = TcpListener::bind(ADMIN_ADDR).expect("bind admin server");
    println!("Admin dashboard at http://{}", ADMIN_ADDR);

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let stats = Arc::clone(&stats);
                thread::spawn(move || handle_conn(stream, stats));
            }
            Err(e) => eprintln!("Admin accept error: {}", e),
        }
    }
}

fn handle_conn(mut stream: std::net::TcpStream, stats: Arc<Stats>) {
    let mut buf = [0u8; 1024];
    let n = match stream.read(&mut buf) {
        Ok(n) if n > 0 => n,
        _ => return,
    };

    let req = std::str::from_utf8(&buf[..n]).unwrap_or("");
    let path = req.split_whitespace().nth(1).unwrap_or("/");
    let snap = stats.snapshot();

    let (content_type, body) = if path == "/stats" || path == "/stats/" {
        ("application/json", render_json(&snap))
    } else {
        ("text/html; charset=utf-8", render_html(&snap))
    };

    let http_header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        content_type,
        body.len()
    );
    let _ = stream.write_all(http_header.as_bytes());
    let _ = stream.write_all(body.as_bytes());
}

fn fmt_bytes(b: u64) -> String {
    if b >= 1_073_741_824 {
        format!("{:.2} GB", b as f64 / 1_073_741_824.0)
    } else if b >= 1_048_576 {
        format!("{:.2} MB", b as f64 / 1_048_576.0)
    } else if b >= 1_024 {
        format!("{:.2} KB", b as f64 / 1_024.0)
    } else {
        format!("{} B", b)
    }
}

fn fmt_uptime(secs: u64) -> String {
    format!("{:02}h {:02}m {:02}s", secs / 3600, (secs % 3600) / 60, secs % 60)
}

fn render_json(snap: &Snapshot) -> String {
    format!(
        r#"{{
  "uptime_secs": {},
  "packets_in": {},
  "packets_out": {},
  "packets_in_per_sec": {},
  "packets_out_per_sec": {},
  "bytes_in": {},
  "bytes_out": {},
  "bytes_in_per_sec": {},
  "bytes_out_per_sec": {},
  "connections_active": {},
  "connections_total": {},
  "errors": {},
  "memory_rss_bytes": {}
}}"#,
        snap.uptime_secs,
        snap.packets_in,
        snap.packets_out,
        snap.packets_in_per_sec,
        snap.packets_out_per_sec,
        snap.bytes_in,
        snap.bytes_out,
        snap.bytes_in_per_sec,
        snap.bytes_out_per_sec,
        snap.connections_active,
        snap.connections_total,
        snap.errors,
        snap.memory_rss_bytes,
    )
}

fn render_html(snap: &Snapshot) -> String {
    // Static header with CSS — kept as a raw string to avoid brace-escaping the CSS.
    const HEADER: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>Rudis Admin</title>
  <meta http-equiv="refresh" content="2">
  <style>
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body { font-family: 'Courier New', monospace; background: #0f0f1a; color: #c9d1d9; padding: 2rem; }
    h1 { color: #58a6ff; font-size: 1.5rem; margin-bottom: 0.3rem; }
    .sub { color: #8b949e; font-size: 0.85rem; margin-bottom: 2rem; }
    .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 1.5rem; }
    .card { background: #161b22; border: 1px solid #30363d; border-radius: 8px; padding: 1.2rem; }
    .card h2 { color: #58a6ff; font-size: 0.8rem; text-transform: uppercase; letter-spacing: 0.07em; margin-bottom: 0.8rem; }
    table { width: 100%; border-collapse: collapse; }
    td { padding: 0.4rem 0; font-size: 0.88rem; }
    td:first-child { color: #8b949e; }
    td:last-child { color: #7ee787; text-align: right; font-weight: bold; }
    tr:not(:last-child) td { border-bottom: 1px solid #21262d; }
    .footer { margin-top: 2rem; color: #484f58; font-size: 0.78rem; }
    a { color: #58a6ff; text-decoration: none; }
    a:hover { text-decoration: underline; }
  </style>
</head>
<body>
  <h1>Rudis Admin</h1>
  <p class="sub">Refreshes every 2&nbsp;s &nbsp;&middot;&nbsp; <a href="/stats">JSON&nbsp;API&nbsp;(/stats)</a></p>
  <div class="grid">"#;

    let uptime = fmt_uptime(snap.uptime_secs);
    let rss = fmt_bytes(snap.memory_rss_bytes);
    let bytes_in = fmt_bytes(snap.bytes_in);
    let bytes_out = fmt_bytes(snap.bytes_out);
    let bytes_in_s = fmt_bytes(snap.bytes_in_per_sec);
    let bytes_out_s = fmt_bytes(snap.bytes_out_per_sec);

    let cards = format!(
        r#"
    <div class="card">
      <h2>Overview</h2>
      <table>
        <tr><td>Uptime</td><td>{uptime}</td></tr>
        <tr><td>Active connections</td><td>{conn_active}</td></tr>
        <tr><td>Total connections</td><td>{conn_total}</td></tr>
        <tr><td>Errors</td><td>{errors}</td></tr>
        <tr><td>Process RSS</td><td>{rss}</td></tr>
      </table>
    </div>
    <div class="card">
      <h2>Throughput (avg/s since start)</h2>
      <table>
        <tr><td>Packets in/s</td><td>{pkts_in_s}</td></tr>
        <tr><td>Packets out/s</td><td>{pkts_out_s}</td></tr>
        <tr><td>Bytes in/s</td><td>{bytes_in_s}</td></tr>
        <tr><td>Bytes out/s</td><td>{bytes_out_s}</td></tr>
      </table>
    </div>
    <div class="card">
      <h2>Totals</h2>
      <table>
        <tr><td>Packets in</td><td>{pkts_in}</td></tr>
        <tr><td>Packets out</td><td>{pkts_out}</td></tr>
        <tr><td>Bytes in</td><td>{bytes_in}</td></tr>
        <tr><td>Bytes out</td><td>{bytes_out}</td></tr>
      </table>
    </div>"#,
        uptime = uptime,
        conn_active = snap.connections_active,
        conn_total = snap.connections_total,
        errors = snap.errors,
        rss = rss,
        pkts_in_s = snap.packets_in_per_sec,
        pkts_out_s = snap.packets_out_per_sec,
        bytes_in_s = bytes_in_s,
        bytes_out_s = bytes_out_s,
        pkts_in = snap.packets_in,
        pkts_out = snap.packets_out,
        bytes_in = bytes_in,
        bytes_out = bytes_out,
    );

    let footer = format!(
        "\n  </div>\n  <p class=\"footer\">rudis {} &nbsp;&middot;&nbsp; data on {} &nbsp;&middot;&nbsp; admin on {}</p>\n</body>\n</html>",
        env!("CARGO_PKG_VERSION"),
        crate::BIND_ADDR,
        ADMIN_ADDR,
    );

    format!("{}{}{}", HEADER, cards, footer)
}
