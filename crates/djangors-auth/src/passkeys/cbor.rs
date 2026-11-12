//! A small, strict CBOR (RFC 8949) reader covering what WebAuthn needs.
//!
//! WebAuthn sends attestation objects, COSE public keys, and extension maps as CBOR.
//! This reader supports integers, byte and text strings, arrays, maps, booleans, and
//! null. It rejects floats, tags, indefinite lengths, and nesting deeper than
//! [`MAX_DEPTH`], and it never allocates more items than there are input bytes, so
//! hostile input produces an error rather than a panic or a large allocation.
//! [`decode_prefix`] reports how many bytes one item used, which WebAuthn needs to
//! find where the credential public key ends inside authenticator data.

use super::PasskeyError;

const MAX_DEPTH: usize = 16;

/// A decoded CBOR value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Value {
    Int(i128),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Value>),
    Map(Vec<(Value, Value)>),
    Bool(bool),
    Null,
}

impl Value {
    /// Looks up an integer key in a map.
    pub(crate) fn get_int(&self, key: i128) -> Option<&Value> {
        match self {
            Value::Map(entries) => entries
                .iter()
                .find(|(k, _)| *k == Value::Int(key))
                .map(|(_, v)| v),
            _ => None,
        }
    }

    /// Looks up a text key in a map.
    pub(crate) fn get_text(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Map(entries) => entries
                .iter()
                .find(|(k, _)| matches!(k, Value::Text(t) if t == key))
                .map(|(_, v)| v),
            _ => None,
        }
    }

    pub(crate) fn as_int(&self) -> Option<i128> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub(crate) fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }

    pub(crate) fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(t) => Some(t),
            _ => None,
        }
    }
}

fn malformed(reason: &'static str) -> PasskeyError {
    PasskeyError::InvalidResponse(format!("malformed CBOR: {reason}"))
}

/// Decodes exactly one item that must fill the whole input.
pub(crate) fn decode(input: &[u8]) -> Result<Value, PasskeyError> {
    let (value, used) = decode_prefix(input)?;
    if used != input.len() {
        return Err(malformed("trailing bytes"));
    }
    Ok(value)
}

/// Decodes one item from the start of `input`, returning it and the bytes it used.
pub(crate) fn decode_prefix(input: &[u8]) -> Result<(Value, usize), PasskeyError> {
    let mut reader = Reader { input, pos: 0 };
    let value = reader.item(0)?;
    Ok((value, reader.pos))
}

struct Reader<'a> {
    input: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], PasskeyError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|end| *end <= self.input.len())
            .ok_or_else(|| malformed("unexpected end of input"))?;
        let bytes = &self.input[self.pos..end];
        self.pos = end;
        Ok(bytes)
    }

    fn remaining(&self) -> usize {
        self.input.len() - self.pos
    }

    /// Reads the argument that follows an initial byte with additional info `info`.
    fn argument(&mut self, info: u8) -> Result<u64, PasskeyError> {
        let width = match info {
            0..=23 => return Ok(u64::from(info)),
            24 => 1,
            25 => 2,
            26 => 4,
            27 => 8,
            31 => return Err(malformed("indefinite lengths are not supported")),
            _ => return Err(malformed("reserved additional information")),
        };
        let bytes = self.take(width)?;
        Ok(bytes.iter().fold(0u64, |acc, b| (acc << 8) | u64::from(*b)))
    }

    /// Converts a length argument to `usize`, refusing lengths longer than the input.
    fn length(&self, arg: u64) -> Result<usize, PasskeyError> {
        usize::try_from(arg)
            .ok()
            .filter(|len| *len <= self.remaining())
            .ok_or_else(|| malformed("length exceeds input"))
    }

    fn item(&mut self, depth: usize) -> Result<Value, PasskeyError> {
        if depth > MAX_DEPTH {
            return Err(malformed("nesting too deep"));
        }
        let initial = self.take(1)?[0];
        let major = initial >> 5;
        let info = initial & 0x1f;
        match major {
            0 => Ok(Value::Int(i128::from(self.argument(info)?))),
            1 => Ok(Value::Int(-1 - i128::from(self.argument(info)?))),
            2 => {
                let arg = self.argument(info)?;
                let len = self.length(arg)?;
                Ok(Value::Bytes(self.take(len)?.to_vec()))
            }
            3 => {
                let arg = self.argument(info)?;
                let len = self.length(arg)?;
                let bytes = self.take(len)?.to_vec();
                String::from_utf8(bytes)
                    .map(Value::Text)
                    .map_err(|_| malformed("text is not UTF-8"))
            }
            4 => {
                // Every item takes at least one byte, so a count above the remaining
                // input is malformed; checking first bounds the allocation.
                let arg = self.argument(info)?;
                let count = self.length(arg)?;
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(self.item(depth + 1)?);
                }
                Ok(Value::Array(items))
            }
            5 => {
                let arg = self.argument(info)?;
                let count = self.length(arg)?;
                let mut entries: Vec<(Value, Value)> = Vec::with_capacity(count);
                for _ in 0..count {
                    let key = self.item(depth + 1)?;
                    let value = self.item(depth + 1)?;
                    if entries.iter().any(|(k, _)| *k == key) {
                        return Err(malformed("duplicate map key"));
                    }
                    entries.push((key, value));
                }
                Ok(Value::Map(entries))
            }
            6 => Err(malformed("tags are not supported")),
            _ => match info {
                20 => Ok(Value::Bool(false)),
                21 => Ok(Value::Bool(true)),
                22 | 23 => Ok(Value::Null),
                _ => Err(malformed("floats and simple values are not supported")),
            },
        }
    }
}
