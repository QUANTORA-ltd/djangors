//! Zero-intermediate-allocation JSON byte-buffer writers for [`Model`](crate::meta::Model) instances.
//!
//! Inspired by Fastify's `fast-json-stringify`, `#[derive(Model)]` precomputes
//! static JSON key fragments at compile time (`concat!("\"", field, "\":")`) and
//! streams field values directly into a contiguous `Vec<u8>` buffer without
//! building intermediate `Vec<(&'static str, Value)>` slices, cloning `String`
//! fields, or allocating `serde_json::Map` (`BTreeMap`) nodes.

use std::io::Write;

use crate::expr::Value;

const HEX: &[u8; 16] = b"0123456789abcdef";

/// Appends a JSON-quoted and escaped UTF-8 string slice directly into `buf`.
///
/// Scans contiguous runs of unescaped bytes and copies them in bulk with
/// `extend_from_slice`, escaping only `"` (`\"`), `\` (`\\`), and control
/// characters (`0x00..=0x1F`) per RFC 8259.
#[inline]
pub fn write_json_str(buf: &mut Vec<u8>, s: &str) {
    buf.push(b'"');
    write_json_str_escaped_inner(buf, s);
    buf.push(b'"');
}

#[inline]
fn write_json_str_escaped_inner(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        let escape = match b {
            b'"' => b"\\\"".as_slice(),
            b'\\' => b"\\\\".as_slice(),
            0x08 => b"\\b".as_slice(),
            0x0C => b"\\f".as_slice(),
            b'\n' => b"\\n".as_slice(),
            b'\r' => b"\\r".as_slice(),
            b'\t' => b"\\t".as_slice(),
            0x00..=0x1F => {
                if start < i {
                    buf.extend_from_slice(&bytes[start..i]);
                }
                buf.extend_from_slice(&[
                    b'\\',
                    b'u',
                    b'0',
                    b'0',
                    HEX[(b >> 4) as usize],
                    HEX[(b & 0x0F) as usize],
                ]);
                start = i + 1;
                continue;
            }
            _ => continue,
        };
        if start < i {
            buf.extend_from_slice(&bytes[start..i]);
        }
        buf.extend_from_slice(escape);
        start = i + 1;
    }
    if start < bytes.len() {
        buf.extend_from_slice(&bytes[start..]);
    }
}

/// Appends a signed 64-bit integer directly into `buf` without heap allocation.
#[inline]
pub fn write_json_i64(buf: &mut Vec<u8>, n: i64) {
    let _ = write!(buf, "{n}");
}

/// Appends a 64-bit float into `buf`, writing `null` for non-finite values (`NaN`, `±∞`)
/// to match `serde_json` float semantics.
#[inline]
pub fn write_json_f64(buf: &mut Vec<u8>, f: f64) {
    if f.is_finite() {
        let start = buf.len();
        let _ = write!(buf, "{f}");
        if !buf[start..]
            .iter()
            .any(|&b| b == b'.' || b == b'e' || b == b'E')
        {
            buf.extend_from_slice(b".0");
        }
    } else {
        buf.extend_from_slice(b"null");
    }
}

/// Appends a JSON boolean literal (`true` or `false`) into `buf`.
#[inline]
pub fn write_json_bool(buf: &mut Vec<u8>, b: bool) {
    if b {
        buf.extend_from_slice(b"true");
    } else {
        buf.extend_from_slice(b"false");
    }
}

/// Appends a JSON `null` literal into `buf`.
#[inline]
pub fn write_json_null(buf: &mut Vec<u8>) {
    buf.extend_from_slice(b"null");
}

/// Appends an RFC 3339 formatted `DateTime<Utc>` as a JSON string into `buf`.
#[inline]
pub fn write_json_datetime(buf: &mut Vec<u8>, dt: &chrono::DateTime<chrono::Utc>) {
    write_json_str(buf, &dt.to_rfc3339());
}

struct JsonStringWriter<'a>(&'a mut Vec<u8>);

impl std::fmt::Write for JsonStringWriter<'_> {
    #[inline]
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        write_json_str_escaped_inner(self.0, s);
        Ok(())
    }
}

/// Formats any `Display` value (`Uuid`, `NaiveDate`, `NaiveTime`, `Decimal`) directly
/// into `buf` as a JSON string without allocating an intermediate `String`.
#[inline]
pub fn write_json_display<T: std::fmt::Display>(buf: &mut Vec<u8>, val: &T) {
    use std::fmt::Write as _;
    buf.push(b'"');
    let _ = write!(JsonStringWriter(buf), "{val}");
    buf.push(b'"');
}

/// Formats any `Debug` value (`std::time::Duration`) directly into `buf` as a JSON
/// string without allocating an intermediate `String`.
#[inline]
pub fn write_json_debug<T: std::fmt::Debug>(buf: &mut Vec<u8>, val: &T) {
    use std::fmt::Write as _;
    buf.push(b'"');
    let _ = write!(JsonStringWriter(buf), "{val:?}");
    buf.push(b'"');
}

/// Writes an ORM [`Value`] directly into `buf` as JSON.
pub fn write_json_value(buf: &mut Vec<u8>, value: &Value) {
    match value {
        Value::I64(n) => write_json_i64(buf, *n),
        Value::F64(f) => write_json_f64(buf, *f),
        Value::Text(s) => write_json_str(buf, s),
        Value::Bool(b) => write_json_bool(buf, *b),
        Value::DateTime(dt) => write_json_datetime(buf, dt),
        Value::Null => write_json_null(buf),
        Value::List(items) => {
            buf.push(b'[');
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    buf.push(b',');
                }
                write_json_value(buf, item);
            }
            buf.push(b']');
        }
    }
}

/// Fallback serializer for hand-written [`Model`](crate::meta::Model) implementations
/// that only provide `field_values()`.
///
/// Sorts emitted keys lexicographically so the JSON byte output matches
/// `serde_json::Map` (`BTreeMap`) key order.
pub fn write_field_values_filtered(
    buf: &mut Vec<u8>,
    fields: &[(&'static str, Value)],
    include: &mut dyn FnMut(&'static str) -> bool,
) {
    let mut sorted: Vec<&(&'static str, Value)> =
        fields.iter().filter(|(name, _)| include(name)).collect();
    sorted.sort_by_key(|(a, _)| *a);
    buf.push(b'{');
    for (idx, (name, value)) in sorted.into_iter().enumerate() {
        if idx > 0 {
            buf.push(b',');
        }
        write_json_str(buf, name);
        buf.push(b':');
        write_json_value(buf, value);
    }
    buf.push(b'}');
}
