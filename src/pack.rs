//! Msgpack serialization compatible with the C yar_pack implementation.
//!
//! This module produces binary-identical output to the C msgpack wrapper
//! (yar_pack.c) which uses msgpack-c with compact integer encoding.

use std::io::{Cursor, Read};

use crate::error::{YarError, YarResult};

/// Data type enum matching C yar_data_type values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum YarDataType {
    Null = 1,
    Bool = 2,
    Long = 3,   // i64 (negative integers)
    Ulong = 4,  // u64 (non-negative integers)
    Double = 5, // f64
    String = 6,
    Map = 7,
    Array = 8,
}

/// Parsed msgpack data tree, equivalent to C's msgpack_object.
#[derive(Debug, Clone, PartialEq)]
pub enum YarData {
    Null,
    Bool(bool),
    Long(i64),
    Ulong(u64),
    Double(f64),
    /// Raw bytes stored as msgpack str (may contain non-UTF-8 data).
    String(Vec<u8>),
    Array(Vec<YarData>),
    Map(Vec<(YarData, YarData)>),
}

impl YarData {
    /// Returns the data type.
    pub fn data_type(&self) -> YarDataType {
        match self {
            YarData::Null => YarDataType::Null,
            YarData::Bool(_) => YarDataType::Bool,
            YarData::Long(_) => YarDataType::Long,
            YarData::Ulong(_) => YarDataType::Ulong,
            YarData::Double(_) => YarDataType::Double,
            YarData::String(_) => YarDataType::String,
            YarData::Array(_) => YarDataType::Array,
            YarData::Map(_) => YarDataType::Map,
        }
    }

    /// Returns (type, size) where size is meaningful for String (byte length),
    /// Array (element count), or Map (entry count).
    /// For other types, size is 0.
    pub fn data_type_and_size(&self) -> (YarDataType, usize) {
        match self {
            YarData::Null => (YarDataType::Null, 0),
            YarData::Bool(_) => (YarDataType::Bool, 0),
            YarData::Long(_) => (YarDataType::Long, 0),
            YarData::Ulong(_) => (YarDataType::Ulong, 0),
            YarData::Double(_) => (YarDataType::Double, 0),
            YarData::String(s) => (YarDataType::String, s.len()),
            YarData::Array(a) => (YarDataType::Array, a.len()),
            YarData::Map(m) => (YarDataType::Map, m.len()),
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            YarData::Bool(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_long(&self) -> Option<i64> {
        match self {
            YarData::Long(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_ulong(&self) -> Option<u64> {
        match self {
            YarData::Ulong(v) => Some(*v),
            _ => None,
        }
    }

    /// Returns the numeric value as i64, converting from Ulong when it fits.
    ///
    /// Values above `i64::MAX` do not fit and yield `None` instead of wrapping
    /// to a negative number.
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            YarData::Long(v) => Some(*v),
            YarData::Ulong(v) => i64::try_from(*v).ok(),
            _ => None,
        }
    }

    pub fn as_double(&self) -> Option<f64> {
        match self {
            YarData::Double(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            YarData::String(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            YarData::String(v) => std::str::from_utf8(v).ok(),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[YarData]> {
        match self {
            YarData::Array(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_map(&self) -> Option<&[(YarData, YarData)]> {
        match self {
            YarData::Map(v) => Some(v),
            _ => None,
        }
    }

    /// Creates an iterator over array elements or map key-value pairs.
    /// For maps, yields keys and values interleaved (size = entries * 2).
    /// Returns None for non-container types.
    pub fn iter(&self) -> Option<UnpackIterator<'_>> {
        match self {
            YarData::Array(arr) => Some(UnpackIterator {
                items: arr.iter().collect(),
                position: 0,
            }),
            YarData::Map(map) => {
                let mut items: Vec<&YarData> = Vec::with_capacity(map.len() * 2);
                for (k, v) in map {
                    items.push(k);
                    items.push(v);
                }
                Some(UnpackIterator { items, position: 0 })
            }
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Packager - incremental msgpack builder
// ---------------------------------------------------------------------------

/// Incremental msgpack packager.
///
/// Mirrors the C `yar_packager` which wraps msgpack-c's sbuffer + packer.
/// Uses compact integer encoding matching `msgpack_pack_int64` /
/// `msgpack_pack_uint64` behavior.
pub struct Packager {
    buffer: Vec<u8>,
}

impl Packager {
    /// Creates an empty packager with no header.
    pub fn new() -> Self {
        Packager { buffer: Vec::new() }
    }

    /// Creates a packager and writes an array header.
    pub fn start_array(size: u32) -> YarResult<Self> {
        let mut pk = Packager::new();
        pk.push_array(size)?;
        Ok(pk)
    }

    /// Creates a packager and writes a map header.
    pub fn start_map(size: u32) -> YarResult<Self> {
        let mut pk = Packager::new();
        pk.push_map(size)?;
        Ok(pk)
    }

    /// Writes a nil value.
    pub fn push_null(&mut self) -> YarResult<()> {
        rmp::encode::write_nil(&mut self.buffer)?;
        Ok(())
    }

    /// Writes a boolean value.
    pub fn push_bool(&mut self, val: bool) -> YarResult<()> {
        rmp::encode::write_bool(&mut self.buffer, val)?;
        Ok(())
    }

    /// Writes a signed integer using compact encoding (msgpack_pack_int64).
    ///
    /// Small values use fewer bytes (fixint, int8, int16, etc.).
    /// Positive values may use unsigned encoding (matching msgpack-c behavior).
    pub fn push_long(&mut self, val: i64) -> YarResult<()> {
        rmp::encode::write_sint(&mut self.buffer, val)
            .map_err(|e| YarError::Pack(e.to_string()))?;
        Ok(())
    }

    /// Writes an unsigned integer using compact encoding (msgpack_pack_uint64).
    ///
    /// Small values use fewer bytes (fixint, uint8, uint16, etc.).
    pub fn push_ulong(&mut self, val: u64) -> YarResult<()> {
        rmp::encode::write_uint(&mut self.buffer, val)
            .map_err(|e| YarError::Pack(e.to_string()))?;
        Ok(())
    }

    /// Writes a 64-bit float (always 9 bytes: 0xcb + 8 bytes).
    pub fn push_double(&mut self, val: f64) -> YarResult<()> {
        rmp::encode::write_f64(&mut self.buffer, val)
            .map_err(|e| YarError::Pack(e.to_string()))?;
        Ok(())
    }

    /// Writes a msgpack str (header + body bytes).
    pub fn push_string(&mut self, data: &[u8]) -> YarResult<()> {
        rmp::encode::write_str_len(&mut self.buffer, data.len() as u32)
            .map_err(|e| YarError::Pack(e.to_string()))?;
        self.buffer.extend_from_slice(data);
        Ok(())
    }

    /// Writes an array header; elements follow via subsequent push calls.
    pub fn push_array(&mut self, size: u32) -> YarResult<()> {
        rmp::encode::write_array_len(&mut self.buffer, size)
            .map_err(|e| YarError::Pack(e.to_string()))?;
        Ok(())
    }

    /// Writes a map header; key-value pairs follow via subsequent push calls.
    pub fn push_map(&mut self, size: u32) -> YarResult<()> {
        rmp::encode::write_map_len(&mut self.buffer, size)
            .map_err(|e| YarError::Pack(e.to_string()))?;
        Ok(())
    }

    /// Serializes a YarData tree into the buffer.
    pub fn push_data(&mut self, data: &YarData) -> YarResult<()> {
        match data {
            YarData::Null => self.push_null(),
            YarData::Bool(v) => self.push_bool(*v),
            YarData::Long(v) => self.push_long(*v),
            YarData::Ulong(v) => self.push_ulong(*v),
            YarData::Double(v) => self.push_double(*v),
            YarData::String(v) => self.push_string(v),
            YarData::Array(arr) => {
                self.push_array(arr.len() as u32)?;
                for item in arr {
                    self.push_data(item)?;
                }
                Ok(())
            }
            YarData::Map(map) => {
                self.push_map(map.len() as u32)?;
                for (k, v) in map {
                    self.push_data(k)?;
                    self.push_data(v)?;
                }
                Ok(())
            }
        }
    }

    /// Appends another packager's raw msgpack bytes into this buffer.
    ///
    /// Equivalent to C's `yar_pack_push_packager` which copies sbuffer bytes.
    pub fn push_packager(&mut self, other: &Packager) -> YarResult<()> {
        self.buffer.extend_from_slice(&other.buffer);
        Ok(())
    }

    /// Returns a reference to the raw msgpack bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buffer
    }

    /// Consumes the packager and returns the raw msgpack bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.buffer
    }
}

impl Default for Packager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Unpackager - msgpack parser
// ---------------------------------------------------------------------------

/// Maximum nesting depth accepted by the msgpack parser.
///
/// The parser is recursive, so an unbounded depth lets a small request
/// overflow the stack (remote crash). 128 is far beyond any Yar payload in
/// practice while staying comfortably inside a runtime worker stack.
pub const MAX_PARSE_DEPTH: usize = 128;

/// Maximum number of msgpack objects accepted from a single payload.
///
/// `YarData` is roughly 32 bytes per node, so an 8 MiB body made of `nil`
/// markers would otherwise expand to hundreds of megabytes of memory.
pub const MAX_PARSE_NODES: usize = 1_000_000;

/// Parses msgpack bytes into a YarData tree.
pub struct Unpackager {
    data: YarData,
}

impl Unpackager {
    /// Parses msgpack bytes into a data tree.
    ///
    /// Rejects inputs that are too deeply nested, declare lengths larger than
    /// the remaining input, contain too many values, or have trailing bytes.
    pub fn new(data: &[u8]) -> YarResult<Self> {
        let mut parser = Parser::new(data);
        let parsed = parser.parse(0)?;
        let consumed = parser.cursor.position() as usize;
        if consumed != data.len() {
            return Err(YarError::Pack(format!(
                "trailing bytes after msgpack object: {}",
                data.len() - consumed
            )));
        }
        Ok(Unpackager { data: parsed })
    }

    /// Returns a reference to the parsed data.
    pub fn data(&self) -> &YarData {
        &self.data
    }

    /// Consumes the unpackager and returns the parsed data.
    pub fn into_data(self) -> YarData {
        self.data
    }
}

// ---------------------------------------------------------------------------
// Low-level msgpack parsing using rmp::decode
// ---------------------------------------------------------------------------

/// Recursive msgpack parser state.
struct Parser<'a> {
    cursor: Cursor<&'a [u8]>,
    nodes: usize,
}

impl<'a> Parser<'a> {
    fn new(data: &'a [u8]) -> Self {
        Parser {
            cursor: Cursor::new(data),
            nodes: 0,
        }
    }

    fn remaining(&self) -> usize {
        let len = self.cursor.get_ref().len();
        len.saturating_sub(self.cursor.position() as usize)
    }

    /// Reject a declared length that cannot possibly fit in the remaining
    /// input before allocating anything for it.
    fn ensure_fits(&self, len: usize, min_bytes_per_item: usize, kind: &str) -> YarResult<()> {
        let remaining = self.remaining();
        let max_items = remaining / min_bytes_per_item;
        if len > max_items {
            return Err(YarError::Pack(format!(
                "msgpack {} length {} exceeds remaining input ({} bytes)",
                kind, len, remaining
            )));
        }
        Ok(())
    }

    /// Parses one msgpack object, enforcing depth and node-count limits.
    fn parse(&mut self, depth: usize) -> YarResult<YarData> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > MAX_PARSE_NODES {
            return Err(YarError::Pack(format!(
                "msgpack node limit exceeded ({})",
                MAX_PARSE_NODES
            )));
        }
        if depth > MAX_PARSE_DEPTH {
            return Err(YarError::Pack(format!(
                "msgpack depth limit exceeded ({})",
                MAX_PARSE_DEPTH
            )));
        }

        // Peek at the marker without advancing the cursor position.
        let marker = {
            let pos = self.cursor.position() as usize;
            let inner = *self.cursor.get_ref();
            if pos >= inner.len() {
                return Err(YarError::Pack("unexpected end of msgpack data".into()));
            }
            rmp::Marker::from_u8(inner[pos])
        };

        match marker {
            // Nil
            rmp::Marker::Null => {
                rmp::decode::read_nil(&mut self.cursor)
                    .map_err(|e| YarError::Pack(e.to_string()))?;
                Ok(YarData::Null)
            }

            // Boolean
            rmp::Marker::True | rmp::Marker::False => {
                let val = rmp::decode::read_bool(&mut self.cursor)
                    .map_err(|e| YarError::Pack(e.to_string()))?;
                Ok(YarData::Bool(val))
            }

            // Unsigned integer types -> Ulong
            // read_int::<u64> reads the marker + data for all unsigned integer formats
            m if is_unsigned_marker(m) => {
                let val = rmp::decode::read_int::<u64, _>(&mut self.cursor)
                    .map_err(|e| YarError::Pack(e.to_string()))?;
                Ok(YarData::Ulong(val))
            }

            // Signed integer types -> Long
            // read_int::<i64> reads the marker + data for all signed integer formats
            m if is_signed_marker(m) => {
                let val = rmp::decode::read_int::<i64, _>(&mut self.cursor)
                    .map_err(|e| YarError::Pack(e.to_string()))?;
                Ok(YarData::Long(val))
            }

            // Float64 (msgpack_pack_double always uses 0xcb + 8 bytes)
            rmp::Marker::F64 => {
                let val = rmp::decode::read_f64(&mut self.cursor)
                    .map_err(|e| YarError::Pack(e.to_string()))?;
                Ok(YarData::Double(val))
            }

            // Float32 (not produced by C code, but handle for robustness)
            rmp::Marker::F32 => {
                let val = rmp::decode::read_f32(&mut self.cursor)
                    .map_err(|e| YarError::Pack(e.to_string()))?;
                Ok(YarData::Double(val as f64))
            }

            // String types - read length header then raw bytes
            rmp::Marker::FixStr(_) | rmp::Marker::Str8
            | rmp::Marker::Str16 | rmp::Marker::Str32 => {
                let len = rmp::decode::read_str_len(&mut self.cursor)
                    .map_err(|e| YarError::Pack(e.to_string()))? as usize;
                self.ensure_fits(len, 1, "string")?;
                self.read_str_body(len)
            }

            // Array types
            rmp::Marker::FixArray(_) | rmp::Marker::Array16 | rmp::Marker::Array32 => {
                let len = rmp::decode::read_array_len(&mut self.cursor)
                    .map_err(|e| YarError::Pack(e.to_string()))? as usize;
                self.ensure_fits(len, 1, "array")?;
                self.read_array(len, depth + 1)
            }

            // Map types
            rmp::Marker::FixMap(_) | rmp::Marker::Map16 | rmp::Marker::Map32 => {
                let len = rmp::decode::read_map_len(&mut self.cursor)
                    .map_err(|e| YarError::Pack(e.to_string()))? as usize;
                self.ensure_fits(len, 2, "map")?;
                self.read_map(len, depth + 1)
            }

            other => Err(YarError::Pack(format!(
                "unsupported msgpack type marker: {:?}",
                other
            ))),
        }
    }

    /// Reads `len` raw bytes from the cursor as a msgpack str body.
    fn read_str_body(&mut self, len: usize) -> YarResult<YarData> {
        let mut buf = vec![0u8; len];
        Read::read_exact(&mut self.cursor, &mut buf)?;
        Ok(YarData::String(buf))
    }

    /// Reads `len` msgpack objects into an array.
    fn read_array(&mut self, len: usize, depth: usize) -> YarResult<YarData> {
        // Never pre-allocate from a remotely supplied length.
        let mut items = Vec::with_capacity(len.min(1024));
        for _ in 0..len {
            items.push(self.parse(depth)?);
        }
        Ok(YarData::Array(items))
    }

    /// Reads `len` key-value pairs into a map.
    fn read_map(&mut self, len: usize, depth: usize) -> YarResult<YarData> {
        let mut entries = Vec::with_capacity(len.min(1024));
        for _ in 0..len {
            let key = self.parse(depth)?;
            let val = self.parse(depth)?;
            entries.push((key, val));
        }
        Ok(YarData::Map(entries))
    }
}

/// Returns true if the marker represents an unsigned integer type.
fn is_unsigned_marker(m: rmp::Marker) -> bool {
    matches!(
        m,
        rmp::Marker::FixPos(_)
            | rmp::Marker::U8
            | rmp::Marker::U16
            | rmp::Marker::U32
            | rmp::Marker::U64
    )
}

/// Returns true if the marker represents a signed integer type.
fn is_signed_marker(m: rmp::Marker) -> bool {
    matches!(
        m,
        rmp::Marker::FixNeg(_)
            | rmp::Marker::I8
            | rmp::Marker::I16
            | rmp::Marker::I32
            | rmp::Marker::I64
    )
}

// ---------------------------------------------------------------------------
// Iterator for arrays and maps
// ---------------------------------------------------------------------------

/// Iterator over array elements or map key-value pairs.
///
/// For maps, yields keys and values interleaved:
/// key0, value0, key1, value1, ... (total size = entries * 2).
/// This matches C's `yar_unpack_iterator` behavior.
pub struct UnpackIterator<'a> {
    items: Vec<&'a YarData>,
    position: usize,
}

impl<'a> UnpackIterator<'a> {
    /// Total number of items (array elements, or map entries * 2).
    pub fn size(&self) -> usize {
        self.items.len()
    }

    /// Current position (0-indexed).
    pub fn position(&self) -> usize {
        self.position
    }

    /// Advances to the next item. Returns the new position, or 0 at end.
    /// Matches C's `yar_unpack_iterator_next` which returns 0 when exhausted.
    pub fn next(&mut self) -> usize {
        if self.position + 1 < self.items.len() {
            self.position += 1;
            self.position
        } else {
            0
        }
    }

    /// Returns the current item.
    ///
    /// For an empty container there is no current item, so `Null` is returned
    /// instead of panicking.
    pub fn current(&self) -> &'a YarData {
        static NULL_DATA: YarData = YarData::Null;
        self.items.get(self.position).copied().unwrap_or(&NULL_DATA)
    }

    /// Resets the iterator to the beginning.
    pub fn reset(&mut self) {
        self.position = 0;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::approx_constant)] // 3.14 is a deliberate C test vector
mod tests {
    use super::*;

    // ---- Null ----

    #[test]
    fn test_pack_unpack_null() {
        let mut pk = Packager::new();
        pk.push_null().unwrap();
        let bytes = pk.as_bytes();

        assert_eq!(bytes, &[0xc0], "null serializes to 0xc0");

        let unpk = Unpackager::new(bytes).unwrap();
        assert_eq!(unpk.data().data_type(), YarDataType::Null);
        assert_eq!(unpk.data(), &YarData::Null);
    }

    // ---- Bool ----

    #[test]
    fn test_pack_unpack_bool() {
        // true
        let mut pk = Packager::new();
        pk.push_bool(true).unwrap();
        assert_eq!(pk.as_bytes(), &[0xc3], "true is 0xc3");

        let unpk = Unpackager::new(pk.as_bytes()).unwrap();
        assert_eq!(unpk.data().data_type(), YarDataType::Bool);
        assert_eq!(unpk.data().as_bool(), Some(true));

        // false
        let mut pk = Packager::new();
        pk.push_bool(false).unwrap();
        assert_eq!(pk.as_bytes(), &[0xc2], "false is 0xc2");

        let unpk = Unpackager::new(pk.as_bytes()).unwrap();
        assert_eq!(unpk.data().as_bool(), Some(false));
    }

    // ---- Long (signed i64) ----

    #[test]
    fn test_pack_long_compact_encoding() {
        // Positive fixint: 0 -> 0x00
        let mut pk = Packager::new();
        pk.push_long(0).unwrap();
        assert_eq!(pk.as_bytes(), &[0x00]);

        // Positive fixint: 42 -> 0x2a
        let mut pk = Packager::new();
        pk.push_long(42).unwrap();
        assert_eq!(pk.as_bytes(), &[0x2a]);

        // Positive fixint: 127 -> 0x7f
        let mut pk = Packager::new();
        pk.push_long(127).unwrap();
        assert_eq!(pk.as_bytes(), &[0x7f]);

        // Negative fixint: -1 -> 0xff
        let mut pk = Packager::new();
        pk.push_long(-1).unwrap();
        assert_eq!(pk.as_bytes(), &[0xff]);

        // Negative fixint: -32 -> 0xe0
        let mut pk = Packager::new();
        pk.push_long(-32).unwrap();
        assert_eq!(pk.as_bytes(), &[0xe0]);

        // int8: -128 -> 0xd0 0x80
        let mut pk = Packager::new();
        pk.push_long(-128).unwrap();
        assert_eq!(pk.as_bytes(), &[0xd0, 0x80]);
    }

    #[test]
    fn test_pack_long_roundtrip() {
        let values: Vec<i64> = vec![0, -1, 42, -128, 127, i64::MIN, i64::MAX, 12345, -12345];
        for &val in &values {
            let mut pk = Packager::new();
            pk.push_long(val).unwrap();
            let bytes = pk.as_bytes();
            assert!(
                !bytes.is_empty() && bytes.len() <= 9,
                "long {} packed to {} bytes",
                val,
                bytes.len()
            );

            let unpk = Unpackager::new(bytes).unwrap();
            if val >= 0 {
                // Non-negative values decode as Ulong (write_sint uses unsigned
                // encoding for positive values, matching msgpack-c behavior)
                assert_eq!(
                    unpk.data().data_type(),
                    YarDataType::Ulong,
                    "non-negative long {} decodes as Ulong",
                    val
                );
                assert_eq!(unpk.data().as_ulong(), Some(val as u64));
            } else {
                // Negative values decode as Long
                assert_eq!(
                    unpk.data().data_type(),
                    YarDataType::Long,
                    "negative long {} decodes as Long",
                    val
                );
                assert_eq!(unpk.data().as_long(), Some(val));
            }
        }
    }

    // ---- Ulong (unsigned u64) ----

    #[test]
    fn test_pack_ulong_roundtrip() {
        let values: Vec<u64> = vec![0, 1, 255, 65535, u64::MAX, 1000, 12345];
        for &val in &values {
            let mut pk = Packager::new();
            pk.push_ulong(val).unwrap();
            let bytes = pk.as_bytes();
            assert!(
                !bytes.is_empty() && bytes.len() <= 9,
                "ulong {} packed to {} bytes",
                val,
                bytes.len()
            );

            let unpk = Unpackager::new(bytes).unwrap();
            assert_eq!(unpk.data().data_type(), YarDataType::Ulong);
            assert_eq!(unpk.data().as_ulong(), Some(val));
        }
    }

    // ---- Double ----

    #[test]
    fn test_pack_double() {
        let values: Vec<f64> = vec![0.0, 3.14, -1.5, 1e100, -1e-100];
        for &val in &values {
            let mut pk = Packager::new();
            pk.push_double(val).unwrap();
            let bytes = pk.as_bytes();

            assert_eq!(bytes.len(), 9, "double is always 9 bytes");
            assert_eq!(bytes[0], 0xcb, "double prefix is 0xcb");

            let unpk = Unpackager::new(bytes).unwrap();
            assert_eq!(unpk.data().data_type(), YarDataType::Double);
            let decoded = unpk.data().as_double().unwrap();
            assert!(
                (decoded - val).abs() < 1e-15 || (val == 0.0 && decoded == 0.0),
                "double {} roundtrip: got {}",
                val,
                decoded
            );
        }
    }

    // ---- String ----

    #[test]
    fn test_pack_string_empty() {
        let mut pk = Packager::new();
        pk.push_string(b"").unwrap();
        let bytes = pk.as_bytes();

        // fixstr len=0: 0xa0
        assert_eq!(bytes, &[0xa0], "empty string is fixstr(0) = 0xa0");

        let unpk = Unpackager::new(bytes).unwrap();
        let (dt, sz) = unpk.data().data_type_and_size();
        assert_eq!(dt, YarDataType::String);
        assert_eq!(sz, 0);
    }

    #[test]
    fn test_pack_string_hello() {
        let mut pk = Packager::new();
        pk.push_string(b"hello").unwrap();
        let bytes = pk.as_bytes();

        // fixstr len=5: 0xa5 + "hello" = 6 bytes
        assert_eq!(bytes.len(), 6);
        assert_eq!(bytes[0], 0xa5);
        assert_eq!(&bytes[1..], b"hello");

        let unpk = Unpackager::new(bytes).unwrap();
        let (dt, sz) = unpk.data().data_type_and_size();
        assert_eq!(dt, YarDataType::String);
        assert_eq!(sz, 5);
        assert_eq!(unpk.data().as_bytes(), Some(b"hello".as_ref()));
        assert_eq!(unpk.data().as_str(), Some("hello"));
    }

    #[test]
    fn test_pack_string_binary_data() {
        let binary: Vec<u8> = vec![0x00, 0x01, 0x02, 0xff, 0x00];
        let mut pk = Packager::new();
        pk.push_string(&binary).unwrap();
        let bytes = pk.as_bytes();

        let unpk = Unpackager::new(bytes).unwrap();
        assert_eq!(unpk.data().data_type(), YarDataType::String);
        assert_eq!(unpk.data().as_bytes(), Some(binary.as_slice()));
    }

    // ---- Array ----

    #[test]
    fn test_pack_array() {
        // Array of 3 elements: null, bool(true), long(42)
        let mut pk = Packager::start_array(3).unwrap();
        pk.push_null().unwrap();
        pk.push_bool(true).unwrap();
        pk.push_long(42).unwrap();
        let bytes = pk.as_bytes();

        // fixarray(3)=0x93, nil=0xc0, true=0xc3, fixint(42)=0x2a
        assert_eq!(bytes.len(), 4);
        assert_eq!(bytes[0], 0x93);
        assert_eq!(bytes[1], 0xc0);
        assert_eq!(bytes[2], 0xc3);
        assert_eq!(bytes[3], 0x2a);

        let unpk = Unpackager::new(bytes).unwrap();
        let (dt, sz) = unpk.data().data_type_and_size();
        assert_eq!(dt, YarDataType::Array);
        assert_eq!(sz, 3);

        // Iterate
        let mut it = unpk.data().iter().unwrap();
        assert_eq!(it.size(), 3);

        assert_eq!(it.current().data_type(), YarDataType::Null);

        assert_ne!(it.next(), 0);
        assert_eq!(it.current().data_type(), YarDataType::Bool);
        assert_eq!(it.current().as_bool(), Some(true));

        assert_ne!(it.next(), 0);
        // 42 packed as int64 compact = fixint -> decodes as Ulong
        assert_eq!(it.current().data_type(), YarDataType::Ulong);
        assert_eq!(it.current().as_ulong(), Some(42));

        assert_eq!(it.next(), 0); // no more
    }

    // ---- Map ----

    #[test]
    fn test_pack_map() {
        // Map of 2 entries: {"name" => "yar", "version" => 1}
        let mut pk = Packager::start_map(2).unwrap();
        pk.push_string(b"name").unwrap();
        pk.push_string(b"yar").unwrap();
        pk.push_string(b"version").unwrap();
        pk.push_long(1).unwrap();
        let bytes = pk.as_bytes();

        let unpk = Unpackager::new(bytes).unwrap();
        let (dt, sz) = unpk.data().data_type_and_size();
        assert_eq!(dt, YarDataType::Map);
        assert_eq!(sz, 2);

        // Map iterator: size = entries * 2 = 4
        let mut it = unpk.data().iter().unwrap();
        assert_eq!(it.size(), 4);

        // Key 0: "name"
        assert_eq!(it.current().as_bytes(), Some(b"name".as_ref()));

        // Value 0: "yar"
        it.next();
        assert_eq!(it.current().as_bytes(), Some(b"yar".as_ref()));

        // Key 1: "version"
        it.next();
        assert_eq!(it.current().as_bytes(), Some(b"version".as_ref()));

        // Value 1: 1
        it.next();
        assert_eq!(it.current().as_ulong(), Some(1));

        // No more
        assert_eq!(it.next(), 0);
    }

    // ---- Nested (push_packager) ----

    #[test]
    fn test_push_packager_nested() {
        // Build inner: array [1, 2, 3]
        let mut inner = Packager::start_array(3).unwrap();
        inner.push_long(1).unwrap();
        inner.push_long(2).unwrap();
        inner.push_long(3).unwrap();

        // Build outer: map {"items" => [1, 2, 3]}
        let mut outer = Packager::start_map(1).unwrap();
        outer.push_string(b"items").unwrap();
        outer.push_packager(&inner).unwrap();
        let bytes = outer.as_bytes();

        let unpk = Unpackager::new(bytes).unwrap();
        let (dt, sz) = unpk.data().data_type_and_size();
        assert_eq!(dt, YarDataType::Map);
        assert_eq!(sz, 1);

        // Iterate map
        let mut it = unpk.data().iter().unwrap();
        assert_eq!(it.current().as_bytes(), Some(b"items".as_ref()));

        it.next();
        let inner_data = it.current();
        let (inner_dt, inner_sz) = inner_data.data_type_and_size();
        assert_eq!(inner_dt, YarDataType::Array);
        assert_eq!(inner_sz, 3);

        // Iterate inner array
        let mut inner_it = inner_data.iter().unwrap();
        assert_eq!(inner_it.current().as_ulong(), Some(1));
        inner_it.next();
        assert_eq!(inner_it.current().as_ulong(), Some(2));
        inner_it.next();
        assert_eq!(inner_it.current().as_ulong(), Some(3));
        assert_eq!(inner_it.next(), 0);
    }

    #[test]
    fn test_push_packager_string_in_array() {
        // Build inner packager with a string
        let mut inner = Packager::new();
        inner.push_string(b"pushed").unwrap();

        // Outer: array[1] containing the inner's bytes
        let mut outer = Packager::start_array(1).unwrap();
        outer.push_packager(&inner).unwrap();
        let bytes = outer.as_bytes();

        let unpk = Unpackager::new(bytes).unwrap();
        let arr = unpk.data().as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0].as_bytes(), Some(b"pushed".as_ref()));
    }

    // ---- push_data ----

    #[test]
    fn test_push_data_roundtrip() {
        let data = YarData::Map(vec![
            (YarData::String(b"key".to_vec()), YarData::Ulong(42)),
            (
                YarData::String(b"nested".to_vec()),
                YarData::Array(vec![YarData::Null, YarData::Bool(true)]),
            ),
        ]);

        let mut pk = Packager::new();
        pk.push_data(&data).unwrap();
        let bytes = pk.as_bytes();

        let unpk = Unpackager::new(bytes).unwrap();
        assert_eq!(unpk.data(), &data);
    }

    // ---- Iterator ----

    #[test]
    fn test_iterator_reset() {
        let data = YarData::Array(vec![
            YarData::Ulong(1),
            YarData::Ulong(2),
            YarData::Ulong(3),
        ]);

        let mut it = data.iter().unwrap();
        assert_eq!(it.current().as_ulong(), Some(1));
        it.next();
        assert_eq!(it.current().as_ulong(), Some(2));

        it.reset();
        assert_eq!(it.position(), 0);
        assert_eq!(it.current().as_ulong(), Some(1));
    }

    // ---- Edge cases ----

    #[test]
    fn test_empty_array() {
        let pk = Packager::start_array(0).unwrap();
        let bytes = pk.as_bytes();
        assert_eq!(bytes, &[0x90]); // fixarray(0)

        let unpk = Unpackager::new(bytes).unwrap();
        assert_eq!(unpk.data().as_array(), Some(vec![].as_slice()));
    }

    #[test]
    fn test_empty_map() {
        let pk = Packager::start_map(0).unwrap();
        let bytes = pk.as_bytes();
        assert_eq!(bytes, &[0x80]); // fixmap(0)

        let unpk = Unpackager::new(bytes).unwrap();
        assert_eq!(unpk.data().as_map(), Some(vec![].as_slice()));
    }

    #[test]
    fn test_into_bytes() {
        let mut pk = Packager::new();
        pk.push_null().unwrap();
        let bytes = pk.into_bytes();
        assert_eq!(bytes, vec![0xc0]);
    }

    #[test]
    fn test_data_type_and_size() {
        assert_eq!(YarData::Null.data_type_and_size(), (YarDataType::Null, 0));
        assert_eq!(
            YarData::Bool(true).data_type_and_size(),
            (YarDataType::Bool, 0)
        );
        assert_eq!(
            YarData::Long(-5).data_type_and_size(),
            (YarDataType::Long, 0)
        );
        assert_eq!(
            YarData::Ulong(10).data_type_and_size(),
            (YarDataType::Ulong, 0)
        );
        assert_eq!(
            YarData::Double(1.0).data_type_and_size(),
            (YarDataType::Double, 0)
        );
        assert_eq!(
            YarData::String(b"hi".to_vec()).data_type_and_size(),
            (YarDataType::String, 2)
        );
        assert_eq!(
            YarData::Array(vec![YarData::Null; 3]).data_type_and_size(),
            (YarDataType::Array, 3)
        );
        assert_eq!(
            YarData::Map(vec![]).data_type_and_size(),
            (YarDataType::Map, 0)
        );
    }

    #[test]
    fn test_as_integer() {
        assert_eq!(YarData::Long(-5).as_integer(), Some(-5));
        assert_eq!(YarData::Ulong(10).as_integer(), Some(10));
        assert_eq!(YarData::Null.as_integer(), None);
    }

    #[test]
    fn test_invalid_data() {
        // Empty input should fail
        assert!(Unpackager::new(&[]).is_err());

        // Truncated data should fail
        assert!(Unpackager::new(&[0xcb, 0x00]).is_err()); // double with only 2 of 9 bytes
    }

    // ---- Binary compatibility test vectors ----

    #[test]
    fn test_binary_compat_known_vectorss() {
        // null
        let mut pk = Packager::new();
        pk.push_null().unwrap();
        assert_eq!(pk.as_bytes(), &[0xc0]);

        // true
        let mut pk = Packager::new();
        pk.push_bool(true).unwrap();
        assert_eq!(pk.as_bytes(), &[0xc3]);

        // false
        let mut pk = Packager::new();
        pk.push_bool(false).unwrap();
        assert_eq!(pk.as_bytes(), &[0xc2]);

        // fixint 42 (via push_long, compact encoding)
        let mut pk = Packager::new();
        pk.push_long(42).unwrap();
        assert_eq!(pk.as_bytes(), &[0x2a]);

        // fixint 42 (via push_ulong, compact encoding)
        let mut pk = Packager::new();
        pk.push_ulong(42).unwrap();
        assert_eq!(pk.as_bytes(), &[0x2a]);

        // empty string: fixstr(0) = 0xa0
        let mut pk = Packager::new();
        pk.push_string(b"").unwrap();
        assert_eq!(pk.as_bytes(), &[0xa0]);

        // "hello": fixstr(5) = 0xa5 + bytes
        let mut pk = Packager::new();
        pk.push_string(b"hello").unwrap();
        assert_eq!(pk.as_bytes(), &[0xa5, b'h', b'e', b'l', b'l', b'o']);
    }

    #[test]
    fn test_pack_unpack_complex_nested() {
        // Build: {"status": "ok", "data": [1, null, {"x": 3.14}]}
        let mut inner_map = Packager::start_map(1).unwrap();
        inner_map.push_string(b"x").unwrap();
        inner_map.push_double(3.14).unwrap();

        let mut arr = Packager::start_array(3).unwrap();
        arr.push_long(1).unwrap();
        arr.push_null().unwrap();
        arr.push_packager(&inner_map).unwrap();

        let mut outer = Packager::start_map(2).unwrap();
        outer.push_string(b"status").unwrap();
        outer.push_string(b"ok").unwrap();
        outer.push_string(b"data").unwrap();
        outer.push_packager(&arr).unwrap();

        let bytes = outer.as_bytes();
        let unpk = Unpackager::new(bytes).unwrap();

        let map = unpk.data().as_map().unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map[0].0.as_str(), Some("status"));
        assert_eq!(map[0].1.as_str(), Some("ok"));
        assert_eq!(map[1].0.as_str(), Some("data"));

        let arr = map[1].1.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_ulong(), Some(1));
        assert_eq!(arr[1], YarData::Null);

        let inner = arr[2].as_map().unwrap();
        assert_eq!(inner.len(), 1);
        assert_eq!(inner[0].0.as_str(), Some("x"));
        assert!((inner[0].1.as_double().unwrap() - 3.14).abs() < 1e-15);
    }
}
