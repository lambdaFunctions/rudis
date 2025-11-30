use crate::resp::{self, Value};
use crate::store::Store;

/// Execute a parsed RESP command against the store and return an encoded response.
pub fn execute(cmd: &Value, store: &mut Store) -> Vec<u8> {
    let args = match cmd {
        Value::Array(Some(a)) if !a.is_empty() => a,
        _ => return resp::error("invalid command format"),
    };
    let name = match &args[0] {
        Value::BulkString(Some(b)) => b.to_ascii_uppercase(),
        _ => return resp::error("command name must be a bulk string"),
    };

    match name.as_slice() {
        b"PING"    => ping(&args[1..]),
        b"SET"     => set(&args[1..], store),
        b"GET"     => get(&args[1..], store),
        b"DEL"     => del(&args[1..], store),
        b"EXISTS"  => exists(&args[1..], store),
        b"DBSIZE"  => resp::integer(store.len() as i64),
        b"FLUSHDB" => { store.flush(); resp::ok().to_vec() }
        b"KEYS"    => keys(store),
        b"COMMAND" => resp::ok().to_vec(), // stub — lets redis-cli connect without errors
        _ => resp::error(&format!(
            "unknown command `{}`",
            String::from_utf8_lossy(&name)
        )),
    }
}

fn ping(args: &[Value]) -> Vec<u8> {
    match args.first() {
        Some(Value::BulkString(Some(msg))) => resp::bulk(msg),
        _ => resp::pong().to_vec(),
    }
}

fn set(args: &[Value], store: &mut Store) -> Vec<u8> {
    match args {
        [Value::BulkString(Some(k)), Value::BulkString(Some(v)), ..] => {
            match store.set(k.clone(), v.clone()) {
                Ok(()) => resp::ok().to_vec(),
                Err(e) => resp::error(e),
            }
        }
        _ => resp::error("SET requires: key value"),
    }
}

fn get(args: &[Value], store: &Store) -> Vec<u8> {
    match args.first() {
        Some(Value::BulkString(Some(k))) => match store.get(k) {
            Some(v) => resp::bulk(v),
            None => resp::null_bulk().to_vec(),
        },
        _ => resp::error("GET requires: key"),
    }
}

fn del(args: &[Value], store: &mut Store) -> Vec<u8> {
    let n = args
        .iter()
        .filter_map(|a| if let Value::BulkString(Some(k)) = a { Some(k.as_slice()) } else { None })
        .filter(|k| store.del(k))
        .count();
    resp::integer(n as i64)
}

fn exists(args: &[Value], store: &Store) -> Vec<u8> {
    let n = args
        .iter()
        .filter_map(|a| if let Value::BulkString(Some(k)) = a { Some(k.as_slice()) } else { None })
        .filter(|k| store.get(k).is_some())
        .count();
    resp::integer(n as i64)
}

fn keys(store: &Store) -> Vec<u8> {
    let ks: Vec<Vec<u8>> = store.keys().map(|k| k.to_vec()).collect();
    let refs: Vec<&[u8]> = ks.iter().map(Vec::as_slice).collect();
    resp::array(&refs)
}
