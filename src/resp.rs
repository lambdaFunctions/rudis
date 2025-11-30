/// A decoded RESP value.
#[derive(Debug, Clone)]
pub enum Value {
    SimpleString(Vec<u8>),
    Error(Vec<u8>),
    Integer(i64),
    BulkString(Option<Vec<u8>>), // None = nil
    Array(Option<Vec<Value>>),   // None = nil
}

/// Try to parse one RESP value from `buf`.
/// Returns `Some((value, bytes_consumed))` or `None` when more data is needed.
pub fn parse(buf: &[u8]) -> Option<(Value, usize)> {
    if buf.is_empty() {
        return None;
    }
    match buf[0] {
        b'+' => {
            let (line, n) = parse_line(&buf[1..])?;
            Some((Value::SimpleString(line), 1 + n))
        }
        b'-' => {
            let (line, n) = parse_line(&buf[1..])?;
            Some((Value::Error(line), 1 + n))
        }
        b':' => {
            let (line, n) = parse_line(&buf[1..])?;
            let i: i64 = std::str::from_utf8(&line).ok()?.parse().ok()?;
            Some((Value::Integer(i), 1 + n))
        }
        b'$' => {
            let (line, header) = parse_line(&buf[1..])?;
            let len: i64 = std::str::from_utf8(&line).ok()?.parse().ok()?;
            if len < 0 {
                return Some((Value::BulkString(None), 1 + header));
            }
            let len = len as usize;
            let start = 1 + header;
            if buf.len() < start + len + 2 {
                return None; // need the data bytes + trailing \r\n
            }
            Some((Value::BulkString(Some(buf[start..start + len].to_vec())), start + len + 2))
        }
        b'*' => {
            let (line, header) = parse_line(&buf[1..])?;
            let count: i64 = std::str::from_utf8(&line).ok()?.parse().ok()?;
            if count < 0 {
                return Some((Value::Array(None), 1 + header));
            }
            let mut pos = 1 + header;
            let mut items = Vec::with_capacity(count as usize);
            for _ in 0..count {
                let (item, n) = parse(&buf[pos..])?;
                items.push(item);
                pos += n;
            }
            Some((Value::Array(Some(items)), pos))
        }
        _ => {
            // Inline command: "GET key\r\n" — sent by netcat or simple clients.
            let (line, n) = parse_line(buf)?;
            let tokens: Vec<Value> = line
                .split(|&b| b == b' ')
                .filter(|t| !t.is_empty())
                .map(|t| Value::BulkString(Some(t.to_vec())))
                .collect();
            if tokens.is_empty() {
                return None;
            }
            Some((Value::Array(Some(tokens)), n))
        }
    }
}

fn parse_line(buf: &[u8]) -> Option<(Vec<u8>, usize)> {
    buf.windows(2)
        .position(|w| w == b"\r\n")
        .map(|p| (buf[..p].to_vec(), p + 2))
}

// ── Encoders ──────────────────────────────────────────────────────────────────

pub fn ok() -> &'static [u8] {
    b"+OK\r\n"
}

pub fn pong() -> &'static [u8] {
    b"+PONG\r\n"
}

pub fn null_bulk() -> &'static [u8] {
    b"$-1\r\n"
}

pub fn error(msg: &str) -> Vec<u8> {
    format!("-ERR {}\r\n", msg).into_bytes()
}

pub fn integer(n: i64) -> Vec<u8> {
    format!(":{}\r\n", n).into_bytes()
}

pub fn bulk(data: &[u8]) -> Vec<u8> {
    let mut out = format!("${}\r\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\r\n");
    out
}

pub fn array(items: &[&[u8]]) -> Vec<u8> {
    let mut out = format!("*{}\r\n", items.len()).into_bytes();
    for item in items {
        out.extend(bulk(item));
    }
    out
}
