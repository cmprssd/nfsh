//! Frostbite's tagged binary tree ("DbObject"), used by `layout.toc` and
//! `chunkmanifest`. Each value is a type byte (low 5 bits the type, bit 7 set when the
//! value has no name), then a NUL-terminated name, then the payload. Lists and objects
//! carry a LEB128 byte size and end with a zero type byte.

use crate::reader::Reader;
use crate::{Result, bail};

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    List(Vec<Value>),
    Object(Vec<(String, Value)>),
    Bool(bool),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    String(String),
    Guid([u8; 16]),
    Sha1([u8; 20]),
    Blob(Vec<u8>),
}

impl Value {
    /// The field `key` of an object.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Value::Int(v) => Some(v.into()),
            Value::Long(v) => Some(v),
            _ => None,
        }
    }
}

/// Deeper nesting than this is treated as corrupt rather than risking the stack.
const MAX_DEPTH: usize = 64;

/// Reads the value at the start of `data`.
pub fn parse(data: &[u8]) -> Result<Value> {
    match read(&mut Reader::new(data), 0)? {
        Some((_, value)) => Ok(value),
        None => bail!("DbObject is empty"),
    }
}

/// One named value, or `None` at a terminator.
fn read(r: &mut Reader, depth: usize) -> Result<Option<(String, Value)>> {
    let at = r.pos();
    let tag = r.u8()?;
    if tag == 0 {
        return Ok(None);
    }
    let name = if tag & 0x80 == 0 {
        r.cstr()?.to_owned()
    } else {
        String::new()
    };
    let value = match tag & 0x1f {
        kind @ (0x01 | 0x02) => {
            if depth >= MAX_DEPTH {
                bail!("DbObject at {at:#x} nests deeper than {MAX_DEPTH}");
            }
            let size = usize::try_from(r.leb128()?).unwrap_or(usize::MAX);
            let body = r.bytes(size)?;
            let mut inner = Reader::new(body);
            let mut items = Vec::new();
            while inner.pos() < body.len() {
                match read(&mut inner, depth + 1)? {
                    Some(item) => items.push(item),
                    None => break,
                }
            }
            if kind == 0x01 {
                Value::List(items.into_iter().map(|(_, v)| v).collect())
            } else {
                Value::Object(items)
            }
        }
        0x06 => Value::Bool(r.u8()? != 0),
        0x07 => {
            let len = usize::try_from(r.leb128()?).unwrap_or(usize::MAX);
            let bytes = r.bytes(len)?;
            let text = bytes.strip_suffix(&[0]).unwrap_or(bytes);
            match std::str::from_utf8(text) {
                Ok(s) => Value::String(s.to_owned()),
                Err(_) => bail!("DbObject string at {at:#x} is not UTF-8"),
            }
        }
        0x08 => Value::Int(i32::from_le_bytes(r.array()?)),
        0x09 => Value::Long(i64::from_le_bytes(r.array()?)),
        0x0b => Value::Float(f32::from_le_bytes(r.array()?)),
        0x0c => Value::Double(f64::from_le_bytes(r.array()?)),
        0x0f => Value::Guid(r.array()?),
        0x10 => Value::Sha1(r.array()?),
        0x13 => {
            let len = usize::try_from(r.leb128()?).unwrap_or(usize::MAX);
            Value::Blob(r.bytes(len)?.to_vec())
        }
        other => bail!("DbObject value at {at:#x} has unknown type {other:#04x}"),
    };
    Ok(Some((name, value)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A named string field.
    fn string(name: &str, value: &str) -> Vec<u8> {
        let mut out = vec![0x07];
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        out.push(value.len() as u8 + 1);
        out.extend_from_slice(value.as_bytes());
        out.push(0);
        out
    }

    /// A list or object (`kind` 1 or 2) around `body`, with an optional name.
    fn container(kind: u8, name: Option<&str>, body: &[u8]) -> Vec<u8> {
        let mut out = vec![kind | if name.is_none() { 0x80 } else { 0 }];
        if let Some(name) = name {
            out.extend_from_slice(name.as_bytes());
            out.push(0);
        }
        out.push(body.len() as u8 + 1);
        out.extend_from_slice(body);
        out.push(0);
        out
    }

    #[test]
    fn reads_a_layout_shaped_tree() {
        let entry = container(2, None, &string("name", "Win32/globals"));
        let mut list = entry.clone();
        list.extend(container(2, None, &string("name", "Win32/ui")));
        let mut body = container(1, Some("superBundles"), &list);
        body.extend_from_slice(&[0x08, b'h', b'e', b'a', b'd', 0, 0x79, 0x09, 0x0b, 0x00]);
        body.extend_from_slice(&[0x06, b'o', b'k', 0, 1]);
        let value = parse(&container(2, None, &body)).unwrap();

        let names: Vec<_> = value
            .get("superBundles")
            .and_then(Value::as_list)
            .unwrap()
            .iter()
            .map(|b| b.get("name").and_then(Value::as_str).unwrap())
            .collect();
        assert_eq!(names, ["Win32/globals", "Win32/ui"]);
        assert_eq!(value.get("head").and_then(Value::as_i64), Some(723_321));
        assert_eq!(value.get("ok"), Some(&Value::Bool(true)));
    }

    #[test]
    fn rejects_truncated_and_unknown_values() {
        let mut data = container(2, None, &string("name", "x"));
        data.truncate(data.len() - 3);
        assert!(parse(&data).is_err());
        assert!(parse(&[0x9f]).is_err());
        assert!(parse(&[0]).is_err());
    }

    #[test]
    fn deep_nesting_is_an_error_not_a_crash() {
        let mut data = vec![0];
        for _ in 0..100 {
            let mut outer = vec![0x81];
            let mut len = data.len();
            while len >= 0x80 {
                outer.push(len as u8 | 0x80);
                len >>= 7;
            }
            outer.push(len as u8);
            outer.extend(data);
            data = outer;
        }
        let err = parse(&data).unwrap_err();
        assert!(err.0.contains("nests deeper"), "{err}");
    }
}
