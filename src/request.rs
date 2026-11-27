//! Request packing/unpacking compatible with the C yar_request implementation.
//!
//! Wire format: a msgpack map with 3 keys:
//! ```text
//! {
//!   "i": <ulong/u64>,     // request ID
//!   "m": <string>,        // method name
//!   "p": <any or null>,   // parameters (packed via push_packager or null)
//! }
//! ```

use crate::error::{YarError, YarResult};
use crate::pack::{Packager, Unpackager, YarData};

/// A Yar RPC request, binary-compatible with C `yar_request`.
pub struct YarRequest {
    pub id: u64,
    pub method: String,
    /// Stored after unpack (the "p" value from the wire map).
    pub params: Option<YarData>,
    /// Stored before pack (set via `set_parameters`).
    params_packed: Option<Packager>,
}

impl YarRequest {
    /// Creates a new request with the given id and method name.
    pub fn new(id: u64, method: &str) -> Self {
        YarRequest {
            id,
            method: method.to_string(),
            params: None,
            params_packed: None,
        }
    }

    /// Stores a Packager whose bytes will be embedded as the "p" value
    /// when `pack()` is called.
    ///
    /// Equivalent to C's `yar_request_set_parameters` which wraps the
    /// inner packager via `yar_pack_push_packager`.
    pub fn set_parameters(&mut self, params: Packager) {
        self.params_packed = Some(params);
    }

    /// Serializes the request to msgpack bytes.
    ///
    /// The output has `extra_bytes` zero bytes prepended (for the protocol
    /// header and packager name), followed by the msgpack map.
    ///
    /// Matches C `yar_request_pack` which allocates `extra_bytes + payload`
    /// and writes the map into the trailing region.
    pub fn pack(&self, extra_bytes: usize) -> YarResult<Vec<u8>> {
        let mut pk = Packager::start_map(3)?;

        // Key "i" -> request id (ulong)
        pk.push_string(b"i")?;
        pk.push_ulong(self.id)?;

        // Key "m" -> method name (string)
        pk.push_string(b"m")?;
        pk.push_string(self.method.as_bytes())?;

        // Key "p" -> parameters (packed bytes, an unpacked value, or null)
        pk.push_string(b"p")?;
        if let Some(ref inner) = self.params_packed {
            pk.push_packager(inner)?;
        } else if let Some(ref data) = self.params {
            pk.push_data(data)?;
        } else {
            pk.push_null()?;
        }

        let payload = pk.into_bytes();

        let mut output = vec![0u8; extra_bytes + payload.len()];
        output[extra_bytes..].copy_from_slice(&payload);
        Ok(output)
    }

    /// Deserializes a request from msgpack bytes, skipping `extra_bytes`
    /// at the start of the slice (header + packager name region).
    ///
    /// Matches C `yar_request_unpack` which creates an unpacker from
    /// `payload + extra_bytes`, verifies the top-level is a map, then
    /// iterates key-value pairs matching on "i", "m", "p".
    pub fn unpack(data: &[u8], extra_bytes: usize) -> YarResult<Self> {
        if data.len() < extra_bytes {
            return Err(YarError::Protocol(
                "payload shorter than extra_bytes offset".into(),
            ));
        }

        let msgpack_bytes = &data[extra_bytes..];
        let unpk = Unpackager::new(msgpack_bytes)?;
        let root = unpk.data();

        // The top-level must be a Map.
        let map = root
            .as_map()
            .ok_or_else(|| YarError::Protocol("request payload is not a msgpack map".into()))?;

        if map.len() < 2 {
            return Err(YarError::Protocol(
                "request map has fewer than 2 entries".into(),
            ));
        }

        let mut id: Option<u64> = None;
        let mut method: Option<String> = None;
        let mut params: Option<YarData> = None;

        for (key, value) in map {
            match key.as_str() {
                Some("i") => {
                    id = Some(value.as_ulong().ok_or_else(|| {
                        YarError::Protocol("request 'i' field is not an unsigned integer".into())
                    })?);
                }
                Some("m") => {
                    let bytes = value.as_bytes().ok_or_else(|| {
                        YarError::Protocol("request 'm' field is not a string".into())
                    })?;
                    method = Some(
                        std::str::from_utf8(bytes)
                            .map_err(|e| {
                                YarError::Protocol(format!("invalid UTF-8 in method: {}", e))
                            })?
                            .to_string(),
                    );
                }
                Some("p") => {
                    // Store the raw YarData reference (cloned) for later retrieval.
                    params = Some(value.clone());
                }
                _ => {
                    // Unknown keys are silently ignored, matching C behavior.
                }
            }
        }

        let id = id.ok_or_else(|| YarError::Protocol("request missing 'i' field".into()))?;
        let method =
            method.ok_or_else(|| YarError::Protocol("request missing 'm' field".into()))?;

        Ok(YarRequest {
            id,
            method,
            params,
            params_packed: None,
        })
    }

    /// Returns the stored parameters from a previous unpack, or None.
    ///
    /// Equivalent to C's `yar_request_get_parameters` which returns the
    /// stored YarData reference.
    pub fn get_parameters(&self) -> Option<&YarData> {
        self.params.as_ref()
    }

    /// Returns the method name.
    pub fn method(&self) -> &str {
        &self.method
    }

    /// Returns the request ID.
    pub fn id(&self) -> u64 {
        self.id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a Packager containing an array [42, "world"].
    fn make_params_packager() -> Packager {
        let mut pk = Packager::start_array(2).unwrap();
        pk.push_long(42).unwrap();
        pk.push_string(b"world").unwrap();
        pk
    }

    #[test]
    fn test_request_pack_unpack() {
        let mut req = YarRequest::new(1000, "hello");
        req.set_parameters(make_params_packager());

        let payload = req.pack(0).unwrap();
        let unpacked = YarRequest::unpack(&payload, 0).unwrap();

        assert_eq!(unpacked.id(), 1000);
        assert_eq!(unpacked.method(), "hello");

        let params = unpacked.get_parameters().expect("params should be present");
        let arr = params.as_array().expect("params should be an array");
        assert_eq!(arr.len(), 2);
        // 42 packed as push_long (compact) decodes as Ulong
        assert_eq!(arr[0].as_ulong(), Some(42));
        assert_eq!(arr[1].as_str(), Some("world"));
    }

    #[test]
    fn test_request_no_params() {
        let req = YarRequest::new(42, "ping");
        let payload = req.pack(0).unwrap();
        let unpacked = YarRequest::unpack(&payload, 0).unwrap();

        assert_eq!(unpacked.id(), 42);
        assert_eq!(unpacked.method(), "ping");

        let params = unpacked
            .get_parameters()
            .expect("params key always present");
        assert_eq!(params, &YarData::Null);
    }

    #[test]
    fn test_request_extra_bytes() {
        let mut req = YarRequest::new(999, "test");
        req.set_parameters(make_params_packager());

        let extra = 90usize;
        let payload = req.pack(extra).unwrap();

        // The first `extra` bytes must all be zero.
        assert!(payload.len() > extra);
        assert!(
            payload[..extra].iter().all(|&b| b == 0),
            "first {} bytes should be zero",
            extra
        );

        // Unpack with the same offset should recover the request.
        let unpacked = YarRequest::unpack(&payload, extra).unwrap();
        assert_eq!(unpacked.id(), 999);
        assert_eq!(unpacked.method(), "test");

        let params = unpacked.get_parameters().unwrap();
        let arr = params.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0].as_ulong(), Some(42));
        assert_eq!(arr[1].as_str(), Some("world"));
    }

    #[test]
    fn test_request_binary_compat() {
        // Pack a known request with no parameters and verify the msgpack body
        // byte-for-byte.
        //
        // Expected msgpack for { "i": 1, "m": "a", "p": null }:
        //   0x83                          fixmap(3)
        //   0xa1 0x69                     fixstr(1) "i"
        //   0x01                          fixint 1
        //   0xa1 0x6d                     fixstr(1) "m"
        //   0xa1 0x61                     fixstr(1) "a"
        //   0xa1 0x70                     fixstr(1) "p"
        //   0xc0                          nil
        let req = YarRequest::new(1, "a");
        let payload = req.pack(0).unwrap();

        let expected: Vec<u8> = vec![
            0x83, // fixmap(3)
            0xa1, 0x69, // fixstr(1) "i"
            0x01, // fixint 1
            0xa1, 0x6d, // fixstr(1) "m"
            0xa1, 0x61, // fixstr(1) "a"
            0xa1, 0x70, // fixstr(1) "p"
            0xc0, // nil
        ];

        assert_eq!(
            payload, expected,
            "binary compatibility: got {:02x?}, expected {:02x?}",
            payload, expected
        );
    }
}
