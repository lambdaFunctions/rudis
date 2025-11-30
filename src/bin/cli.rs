// Interactive CLI for rudis — behaves like redis-cli.
//
// Usage:
//   cargo run --bin cli [host:port]
//   cargo run --bin cli 127.0.0.1:8080
//
// Supported commands: PING, SET, GET, DEL, EXISTS, DBSIZE, KEYS, FLUSHDB

use rudis::resp::{self, Value};
use std::io::{self, Read, Write, BufRead};
use std::net::TcpStream;

const DEFAULT_ADDR: &str = "127.0.0.1:8080";

fn main() {
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_ADDR.to_string());

    let mut stream = match TcpStream::connect(&addr) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Could not connect to {}: {}", addr, e);
            std::process::exit(1);
        }
    };
    stream.set_nodelay(true).unwrap();

    println!("Connected to {}. Type 'quit' or Ctrl-D to exit.", addr);

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut read_buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 4096];

    loop {
        print!("rudis> ");
        stdout.flush().unwrap();

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) | Err(_) => {
                println!();
                break;
            }
            Ok(_) => {}
        }

        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.eq_ignore_ascii_case("quit") || line.eq_ignore_ascii_case("exit") {
            break;
        }

        let tokens = tokenize(line);
        if tokens.is_empty() {
            continue;
        }

        let cmd = encode_cmd(&tokens);
        if let Err(e) = stream.write_all(&cmd) {
            eprintln!("(error) write failed: {}", e);
            break;
        }

        // Read exactly one RESP response, buffering across reads as needed.
        loop {
            match resp::parse(&read_buf) {
                Some((value, consumed)) => {
                    read_buf.drain(..consumed);
                    display(&value, 0);
                    break;
                }
                None => match stream.read(&mut tmp) {
                    Ok(0) => {
                        eprintln!("(error) server closed the connection");
                        return;
                    }
                    Ok(n) => read_buf.extend_from_slice(&tmp[..n]),
                    Err(e) => {
                        eprintln!("(error) read failed: {}", e);
                        return;
                    }
                },
            }
        }
    }
}

/// Split a command line into tokens, respecting double-quoted strings.
fn tokenize(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for ch in line.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ' ' | '\t' if !in_quotes => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Encode a slice of string tokens as a RESP array of bulk strings.
fn encode_cmd(tokens: &[String]) -> Vec<u8> {
    let refs: Vec<&[u8]> = tokens.iter().map(|s| s.as_bytes()).collect();
    resp::array(&refs)
}

/// Pretty-print a RESP value, indented by `depth` (used for nested arrays).
fn display(value: &Value, depth: usize) {
    let indent = "  ".repeat(depth);
    match value {
        Value::SimpleString(s) => println!("{}\"{}\"", indent, String::from_utf8_lossy(s)),
        Value::Error(e) => eprintln!("{}(error) {}", indent, String::from_utf8_lossy(e)),
        Value::Integer(n) => println!("{}(integer) {}", indent, n),
        Value::BulkString(Some(s)) => match std::str::from_utf8(s) {
            Ok(text) => println!("{}\"{}\"", indent, text),
            Err(_) => println!("{}{:?}", indent, s),
        },
        Value::BulkString(None) => println!("{}(nil)", indent),
        Value::Array(None) => println!("{}(nil)", indent),
        Value::Array(Some(items)) if items.is_empty() => println!("{}(empty array)", indent),
        Value::Array(Some(items)) => {
            for (i, item) in items.iter().enumerate() {
                print!("{}{}) ", indent, i + 1);
                display(item, depth + 1);
            }
        }
    }
}
