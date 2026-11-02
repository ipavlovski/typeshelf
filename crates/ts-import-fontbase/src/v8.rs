//! Decodes values Chromium stores in IndexedDB: a Blink envelope (optionally
//! snappy-compressed) around V8's structured-clone serialization.

use anyhow::{Context, Result, bail};

use crate::leveldb::varint;

#[derive(Clone, PartialEq, Debug)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn num(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn fields(&self) -> &[(String, Value)] {
        match self {
            Value::Obj(f) => f,
            _ => &[],
        }
    }

    pub fn items(&self) -> &[Value] {
        match self {
            Value::Arr(a) => a,
            _ => &[],
        }
    }
}

/// An IndexedDB record value: varint record version, then a Blink-serialized value.
pub fn decode_record(raw: &[u8]) -> Result<Value> {
    let mut i = 0;
    varint(raw, &mut i)?;
    decode_blink(&raw[i..])
}

/// `FF <version> [02 <snappy>]` envelope around the V8 payload.
pub fn decode_blink(raw: &[u8]) -> Result<Value> {
    let mut i = 0;
    if raw.first() == Some(&0xff) {
        i = 1;
        varint(raw, &mut i)?;
        match raw.get(i) {
            Some(0x01) => bail!("value is stored in an external blob"),
            Some(0x02) => {
                let inner = snap::raw::Decoder::new().decompress_vec(&raw[i + 1..])?;
                return Reader { b: &inner, i: 0 }.value();
            }
            _ => {}
        }
    }
    Reader { b: &raw[i..], i: 0 }.value()
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8> {
        let b = *self.b.get(self.i).context("truncated value")?;
        self.i += 1;
        Ok(b)
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn varint(&mut self) -> Result<u64> {
        varint(self.b, &mut self.i)
    }

    fn bytes(&mut self, n: usize) -> Result<&[u8]> {
        let s = self.b.get(self.i..self.i + n).context("truncated value")?;
        self.i += n;
        Ok(s)
    }

    fn value(&mut self) -> Result<Value> {
        loop {
            let tag = self.byte()?;
            return Ok(match tag {
                0x00 => continue,
                // Version headers (Blink and V8).
                0xff => {
                    self.varint()?;
                    continue;
                }
                // Blink trailer offset: u64 offset + u32 size.
                0xfe => {
                    self.bytes(12)?;
                    continue;
                }
                b'o' => {
                    let mut fields = Vec::new();
                    while self.peek() != Some(b'{') {
                        let key = match self.value()? {
                            Value::Str(s) => s,
                            Value::Num(n) => n.to_string(),
                            other => format!("{other:?}"),
                        };
                        fields.push((key, self.value()?));
                    }
                    self.i += 1;
                    self.varint()?;
                    Value::Obj(fields)
                }
                b'"' => {
                    let n = self.varint()? as usize;
                    Value::Str(self.bytes(n)?.iter().map(|&c| c as char).collect())
                }
                b'c' => {
                    let n = self.varint()? as usize;
                    let units: Vec<u16> =
                        self.bytes(n)?.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).collect();
                    Value::Str(String::from_utf16_lossy(&units))
                }
                b'S' => {
                    let n = self.varint()? as usize;
                    Value::Str(String::from_utf8_lossy(self.bytes(n)?).into_owned())
                }
                b'I' => {
                    let v = self.varint()?;
                    Value::Num(((v >> 1) as i64 ^ -((v & 1) as i64)) as f64)
                }
                b'U' => Value::Num(self.varint()? as f64),
                b'N' | b'D' => Value::Num(f64::from_le_bytes(self.bytes(8)?.try_into()?)),
                b'T' => Value::Bool(true),
                b'F' => Value::Bool(false),
                b'0' | b'_' => Value::Null,
                b'A' => {
                    let n = self.varint()? as usize;
                    let mut items = Vec::with_capacity(n.min(1 << 16));
                    for _ in 0..n {
                        items.push(self.value()?);
                    }
                    // Extra named properties, then the end tag with two counts.
                    while self.peek() != Some(b'$') {
                        self.value()?;
                        self.value()?;
                    }
                    self.i += 1;
                    self.varint()?;
                    self.varint()?;
                    Value::Arr(items)
                }
                b'a' => {
                    let n = self.varint()? as usize;
                    let mut items = vec![Value::Null; n.min(1 << 16)];
                    while self.peek() != Some(b'@') {
                        let k = self.value()?;
                        let v = self.value()?;
                        if let Some(slot) = k.num().and_then(|k| items.get_mut(k as usize)) {
                            *slot = v;
                        }
                    }
                    self.i += 1;
                    self.varint()?;
                    self.varint()?;
                    Value::Arr(items)
                }
                // Back-reference to an earlier object; FontBase data has none that matter.
                b'^' => {
                    self.varint()?;
                    Value::Null
                }
                t => bail!("unsupported V8 tag {:?} at {}", t as char, self.i - 1),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_objects_arrays_and_numbers() {
        // { a: "x", n: -3, list: [1.5, true], u: "Жж" }
        let mut v = vec![0xff, 0x0f, b'o', b'"', 1, b'a', b'"', 1, b'x', b'"', 1, b'n', b'I', 5];
        v.extend([b'"', 4, b'l', b'i', b's', b't', b'A', 2, b'N']);
        v.extend(1.5f64.to_le_bytes());
        v.extend([b'T', b'$', 0, 2, b'"', 1, b'u', b'c', 4, 0x16, 0x04, 0x36, 0x04, b'{', 4]);
        let val = decode_blink(&v).unwrap();
        assert_eq!(val.get("a").and_then(Value::str), Some("x"));
        assert_eq!(val.get("n").and_then(Value::num), Some(-3.0));
        assert_eq!(val.get("list").unwrap().items(), [Value::Num(1.5), Value::Bool(true)]);
        assert_eq!(val.get("u").and_then(Value::str), Some("Жж"));
    }

    #[test]
    fn decodes_snappy_envelope() {
        let inner = [0xff, 0x0f, b'"', 2, b'h', b'i'];
        let mut raw = vec![0xff, 0x11, 0x02];
        raw.extend(snap::raw::Encoder::new().compress_vec(&inner).unwrap());
        assert_eq!(decode_blink(&raw).unwrap(), Value::Str("hi".into()));
    }
}
