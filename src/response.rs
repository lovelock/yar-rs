//! Response pack/unpack — binary compatible with C yar_response.
//!
//! Wire format: msgpack map with 4 keys:
//!   "i" -> ulong (response ID)
//!   "s" -> long  (status code, 0 = success)
//!   "r" -> any or null (return value)
//!   "e" -> string or null (error message)

use crate::error::{YarError, YarResult};
use crate::pack::{Packager, Unpackager, YarData};

/// Yar RPC response structure.
pub struct YarResponse {
    pub id: u64,
    pub status: i64,
    pub error: Option<String>,
    pub retval: Option<YarData>,
    retval_packed: Option<Packager>,
}

impl YarResponse {
    /// Creates an empty success response with no return value.
    pub fn new(id: u64) -> Self {
        YarResponse {
            id,
            status: 0,
            error: None,
            retval: None,
            retval_packed: None,
        }
    }

    /// Creates a success response with a return value.
    pub fn success(id: u64, retval: Packager) -> Self {
        YarResponse {
            id,
            status: 0,
            error: None,
            retval: None,
            retval_packed: Some(retval),
        }
    }

    /// Creates an error response.
    pub fn error(id: u64, status: i64, message: &str) -> Self {
        let mut resp = YarResponse::new(id);
        resp.set_error(status, message);
        resp
    }

    /// Sets error with status code and message.
    /// Truncates message at 1024 bytes, appending "..." if truncated.
    pub fn set_error(&mut self, code: i64, message: &str) {
        self.status = code;
        let bytes = message.as_bytes();
        if bytes.len() > 1024 {
            // Truncate at 1024 bytes, being careful with UTF-8 boundaries
            let truncated = std::str::from_utf8(&bytes[..1024]).unwrap_or_else(|e| {
                // If we land mid-char, back up to valid boundary
                std::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap_or("")
            });
            self.error = Some(format!("{}...", truncated));
        } else {
            self.error = Some(message.to_string());
        }
    }

    /// Stores a packager as the return value for later serialization.
    pub fn set_retval(&mut self, retval: Packager) {
        self.retval_packed = Some(retval);
    }

    /// Serializes the response to msgpack bytes with extra_bytes prepended.
    pub fn pack(&self, extra_bytes: usize) -> YarResult<Vec<u8>> {
        let mut pk = Packager::start_map(4)?;

        // "i" -> id (ulong)
        pk.push_string(b"i")?;
        pk.push_ulong(self.id)?;

        // "s" -> status (long/signed)
        pk.push_string(b"s")?;
        pk.push_long(self.status)?;

        // "r" -> retval, retval data, or null
        pk.push_string(b"r")?;
        if let Some(ref packed) = self.retval_packed {
            pk.push_packager(packed)?;
        } else if let Some(ref data) = self.retval {
            pk.push_data(data)?;
        } else {
            pk.push_null()?;
        }

        // "e" -> error or null
        pk.push_string(b"e")?;
        if let Some(ref err) = self.error {
            pk.push_string(err.as_bytes())?;
        } else {
            pk.push_null()?;
        }

        let msgpack_bytes = pk.into_bytes();
        let mut result = vec![0u8; extra_bytes];
        result.extend_from_slice(&msgpack_bytes);
        Ok(result)
    }

    /// Deserializes a response from msgpack bytes, skipping extra_bytes at the start.
    pub fn unpack(data: &[u8], extra_bytes: usize) -> YarResult<Self> {
        if data.len() < extra_bytes {
            return Err(YarError::Protocol("data shorter than extra_bytes".into()));
        }
        let payload = &data[extra_bytes..];
        let unpk = Unpackager::new(payload)?;
        let root = unpk.data();

        let map = root
            .as_map()
            .ok_or_else(|| YarError::Protocol("response root is not a map".into()))?;

        let mut id: Option<u64> = None;
        let mut status: Option<i64> = None;
        let mut error: Option<String> = None;
        let mut retval: Option<YarData> = None;

        for (key, val) in map {
            match key.as_bytes() {
                Some(b"i") => {
                    id =
                        Some(val.as_ulong().ok_or_else(|| {
                            YarError::Protocol("response 'i' is not ulong".into())
                        })?);
                }
                Some(b"s") => {
                    // C code checks for YAR_DATA_ULONG but we handle both Long and Ulong.
                    // Ulong values above i64::MAX are rejected rather than wrapped.
                    status = Some(val.as_integer().ok_or_else(|| {
                        YarError::Protocol("response 's' is not an in-range integer".into())
                    })?);
                }
                Some(b"r") => {
                    if val.data_type() != crate::pack::YarDataType::Null {
                        retval = Some(val.clone());
                    }
                }
                Some(b"e") if val.data_type() != crate::pack::YarDataType::Null => {
                    let err_str = val
                        .as_str()
                        .ok_or_else(|| YarError::Protocol("response 'e' is not a string".into()))?;
                    error = Some(err_str.to_string());
                }
                _ => {} // ignore unknown keys
            }
        }

        let id = id.ok_or_else(|| YarError::Protocol("response missing 'i' field".into()))?;
        let status =
            status.ok_or_else(|| YarError::Protocol("response missing 's' field".into()))?;

        Ok(YarResponse {
            id,
            status,
            error,
            retval,
            retval_packed: None,
        })
    }

    /// Returns the stored return value after unpacking.
    pub fn get_response(&self) -> Option<&YarData> {
        self.retval.as_ref()
    }

    /// Returns the status code.
    pub fn get_status(&self) -> i64 {
        self.status
    }

    /// Returns the error message if set.
    pub fn get_error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_response_pack_unpack_success() {
        let mut retval_pk = Packager::new();
        retval_pk.push_string(b"result").unwrap();

        let resp = YarResponse::success(1000, retval_pk);
        assert_eq!(resp.get_status(), 0);
        assert!(resp.get_error().is_none());

        let packed = resp.pack(0).unwrap();
        let unpacked = YarResponse::unpack(&packed, 0).unwrap();

        assert_eq!(unpacked.id, 1000);
        assert_eq!(unpacked.get_status(), 0);
        assert!(unpacked.get_error().is_none());
        let rv = unpacked.get_response().unwrap();
        assert_eq!(rv.as_str(), Some("result"));
    }

    #[test]
    fn test_response_pack_unpack_error() {
        let resp = YarResponse::error(1000, 8, "something failed");
        let packed = resp.pack(0).unwrap();
        let unpacked = YarResponse::unpack(&packed, 0).unwrap();

        assert_eq!(unpacked.id, 1000);
        assert_eq!(unpacked.get_status(), 8);
        assert_eq!(unpacked.get_error(), Some("something failed"));
        assert!(unpacked.get_response().is_none());
    }

    #[test]
    fn test_response_set_error() {
        let mut resp = YarResponse::new(1);
        resp.set_error(5, "formatted error: code=42");
        assert_eq!(resp.get_status(), 5);
        assert_eq!(resp.get_error(), Some("formatted error: code=42"));

        // Test truncation at 1024 bytes
        let long_msg = "x".repeat(2000);
        resp.set_error(1, &long_msg);
        let err = resp.get_error().unwrap();
        // 1024 bytes of 'x' + "..." = 1027
        assert_eq!(err.len(), 1027);
        assert!(err.ends_with("..."));
    }

    #[test]
    fn test_response_set_retval() {
        let mut resp = YarResponse::new(42);
        let mut pk = Packager::new();
        pk.push_ulong(99).unwrap();
        resp.set_retval(pk);

        let packed = resp.pack(0).unwrap();
        let unpacked = YarResponse::unpack(&packed, 0).unwrap();

        assert_eq!(unpacked.id, 42);
        assert_eq!(unpacked.get_status(), 0);
        let rv = unpacked.get_response().unwrap();
        assert_eq!(rv.as_ulong(), Some(99));
    }

    #[test]
    fn test_response_extra_bytes() {
        let resp = YarResponse::new(100);
        let packed = resp.pack(90).unwrap();

        // First 90 bytes should be zeros
        assert_eq!(packed.len(), 90 + (packed.len() - 90));
        assert!(packed[..90].iter().all(|&b| b == 0));

        let unpacked = YarResponse::unpack(&packed, 90).unwrap();
        assert_eq!(unpacked.id, 100);
        assert_eq!(unpacked.get_status(), 0);
    }

    #[test]
    fn test_response_getters() {
        let resp = YarResponse::error(500, 3, "timeout");
        assert_eq!(resp.get_status(), 3);
        assert_eq!(resp.get_error(), Some("timeout"));
        assert!(resp.get_response().is_none());

        let resp2 = YarResponse::new(1);
        assert_eq!(resp2.get_status(), 0);
        assert!(resp2.get_error().is_none());
        assert!(resp2.get_response().is_none());
    }

    #[test]
    fn test_response_binary_compat() {
        // Pack a known response: id=1, status=0, no retval, no error
        let resp = YarResponse::new(1);
        let packed = resp.pack(0).unwrap();

        // Expected: fixmap(4) = 0x84
        // "i" = fixstr(1) 0xa1 + 'i', value = fixint 0x01
        // "s" = fixstr(1) 0xa1 + 's', value = fixint 0x00
        // "r" = fixstr(1) 0xa1 + 'r', value = nil 0xc0
        // "e" = fixstr(1) 0xa1 + 'e', value = nil 0xc0
        let expected: Vec<u8> = vec![
            0x84, // fixmap(4)
            0xa1, b'i', 0x01, // "i" -> 1
            0xa1, b's', 0x00, // "s" -> 0
            0xa1, b'r', 0xc0, // "r" -> nil
            0xa1, b'e', 0xc0, // "e" -> nil
        ];
        assert_eq!(packed, expected);
    }
}
