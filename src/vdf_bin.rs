//! Steam's binary KeyValues format, as used by `userdata/<id>/config/shortcuts.vdf`.
//!
//! Keys and strings are kept as raw bytes so that a file read and written back without changes
//! is byte-for-byte identical, even if another tool put non-UTF-8 text in it.

use anyhow::{Result, bail};

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Map(Map),
    Str(Vec<u8>),
    Int(i32),
    Float(f32),
    Ptr(u32),
    Color(u32),
    U64(u64),
    I64(i64),
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Map(pub Vec<(Vec<u8>, Value)>);

const T_MAP: u8 = 0x00;
const T_STR: u8 = 0x01;
const T_INT: u8 = 0x02;
const T_FLOAT: u8 = 0x03;
const T_PTR: u8 = 0x04;
const T_COLOR: u8 = 0x06;
const T_U64: u8 = 0x07;
const T_END: u8 = 0x08;
const T_I64: u8 = 0x0A;
const T_END_ALT: u8 = 0x0B;

impl Map {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key.as_bytes()))
            .map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.0
            .iter_mut()
            .find(|(k, _)| k.eq_ignore_ascii_case(key.as_bytes()))
            .map(|(_, v)| v)
    }

    pub fn get_str(&self, key: &str) -> Option<String> {
        match self.get(key) {
            Some(Value::Str(s)) => Some(String::from_utf8_lossy(s).into_owned()),
            _ => None,
        }
    }

    pub fn get_int(&self, key: &str) -> Option<i32> {
        match self.get(key) {
            Some(Value::Int(i)) => Some(*i),
            _ => None,
        }
    }

    /// Replaces the value of an existing key (keeping its position and spelling), or appends it.
    pub fn set(&mut self, key: &str, value: Value) {
        match self.get_mut(key) {
            Some(v) => *v = value,
            None => self.0.push((key.as_bytes().to_vec(), value)),
        }
    }
}

pub fn parse(data: &[u8]) -> Result<Map> {
    let mut pos = 0;
    let map = parse_map(data, &mut pos)?;
    if pos != data.len() {
        bail!("unexpected data after the end at byte {pos}");
    }
    Ok(map)
}

fn take<'a>(data: &'a [u8], pos: &mut usize, n: usize) -> Result<&'a [u8]> {
    if *pos + n > data.len() {
        bail!("file ends early at byte {}", *pos);
    }
    let s = &data[*pos..*pos + n];
    *pos += n;
    Ok(s)
}

fn cstr(data: &[u8], pos: &mut usize) -> Result<Vec<u8>> {
    let rest = &data[*pos..];
    let Some(len) = rest.iter().position(|&b| b == 0) else {
        bail!("unterminated string at byte {}", *pos);
    };
    let s = rest[..len].to_vec();
    *pos += len + 1;
    Ok(s)
}

fn parse_map(data: &[u8], pos: &mut usize) -> Result<Map> {
    let mut map = Map::default();
    loop {
        let ty = take(data, pos, 1)?[0];
        if ty == T_END || ty == T_END_ALT {
            return Ok(map);
        }
        let key = cstr(data, pos)?;
        let value = match ty {
            T_MAP => Value::Map(parse_map(data, pos)?),
            T_STR => Value::Str(cstr(data, pos)?),
            T_INT => Value::Int(i32::from_le_bytes(take(data, pos, 4)?.try_into()?)),
            T_FLOAT => Value::Float(f32::from_le_bytes(take(data, pos, 4)?.try_into()?)),
            T_PTR => Value::Ptr(u32::from_le_bytes(take(data, pos, 4)?.try_into()?)),
            T_COLOR => Value::Color(u32::from_le_bytes(take(data, pos, 4)?.try_into()?)),
            T_U64 => Value::U64(u64::from_le_bytes(take(data, pos, 8)?.try_into()?)),
            T_I64 => Value::I64(i64::from_le_bytes(take(data, pos, 8)?.try_into()?)),
            other => bail!("unknown value type 0x{other:02x} at byte {}", *pos),
        };
        map.0.push((key, value));
    }
}

pub fn serialize(map: &Map) -> Vec<u8> {
    let mut out = Vec::new();
    write_map(map, &mut out);
    out
}

fn write_map(map: &Map, out: &mut Vec<u8>) {
    for (key, value) in &map.0 {
        let ty = match value {
            Value::Map(_) => T_MAP,
            Value::Str(_) => T_STR,
            Value::Int(_) => T_INT,
            Value::Float(_) => T_FLOAT,
            Value::Ptr(_) => T_PTR,
            Value::Color(_) => T_COLOR,
            Value::U64(_) => T_U64,
            Value::I64(_) => T_I64,
        };
        out.push(ty);
        out.extend_from_slice(key);
        out.push(0);
        match value {
            Value::Map(m) => write_map(m, out),
            Value::Str(s) => {
                out.extend_from_slice(s);
                out.push(0);
            }
            Value::Int(i) => out.extend_from_slice(&i.to_le_bytes()),
            Value::Float(f) => out.extend_from_slice(&f.to_le_bytes()),
            Value::Ptr(p) | Value::Color(p) => out.extend_from_slice(&p.to_le_bytes()),
            Value::U64(u) => out.extend_from_slice(&u.to_le_bytes()),
            Value::I64(i) => out.extend_from_slice(&i.to_le_bytes()),
        }
    }
    out.push(T_END);
}

pub fn str_val(s: &str) -> Value {
    Value::Str(s.as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut inner = Map::default();
        inner.set("appid", Value::Int(-123));
        inner.set("AppName", str_val("Game"));
        inner.set("tags", Value::Map(Map::default()));
        let mut list = Map::default();
        list.0.push((b"0".to_vec(), Value::Map(inner)));
        let mut root = Map::default();
        root.0.push((b"shortcuts".to_vec(), Value::Map(list)));
        let bytes = serialize(&root);
        assert_eq!(parse(&bytes).unwrap(), root);
        assert_eq!(serialize(&parse(&bytes).unwrap()), bytes);
    }

    /// Reads a real shortcuts.vdf and checks that writing it back gives identical bytes:
    /// BNET_TEST_SHORTCUTS=<path to a copy> cargo test -- --ignored
    #[test]
    #[ignore]
    fn real_file_roundtrip() {
        let path = std::env::var("BNET_TEST_SHORTCUTS").expect("set BNET_TEST_SHORTCUTS");
        let data = std::fs::read(path).unwrap();
        assert_eq!(serialize(&parse(&data).unwrap()), data);
    }

    #[test]
    fn rejects_truncated() {
        assert!(parse(b"\x00shortcuts\x00\x000\x00\x02appid\x00\x01").is_err());
    }
}
