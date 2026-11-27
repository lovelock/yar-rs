//! Yar protocol header - binary compatible with the C implementation.
//!
//! The header is packed (no padding), with fields stored in network byte order
//! (big-endian) on the wire. The C struct comment says "Total: 80 bytes" but
//! the actual fields sum to 82 (4+2+4+4+32+32+4).

use byteorder::{BigEndian, ByteOrder};

use crate::error::{YarError, YarResult};

/// Protocol flag: persistent connection
pub const PROTOCOL_PERSISTENT: u32 = 0x1;
/// Protocol flag: ping
pub const PROTOCOL_PING: u32 = 0x2;
/// Protocol flag: list
pub const PROTOCOL_LIST: u32 = 0x4;

/// Protocol magic number (native byte order value)
pub const PROTOCOL_MAGIC: u32 = 0x80DFEC60;

/// Size of the packed header in bytes.
/// id(4)+version(2)+magic(4)+reserved(4)+provider(32)+token(32)+body_len(4) = 82
pub const HEADER_SIZE: usize = 82;

// Keep the constant and the documented layout in sync.
const _: () = assert!(HEADER_SIZE == 4 + 2 + 4 + 4 + 32 + 32 + 4);

/// Yar protocol header, matching the C `yar_header` struct fields.
///
/// All numeric fields are stored in **host byte order**. Use [`to_bytes`] to
/// produce wire format and [`from_bytes`] to parse it. This keeps the public
/// type free of the packed/unaligned-field hazards of a direct C layout.
///
/// Layout on the wire (packed, no padding):
///   id:         4 bytes  (unsigned int)
///   version:    2 bytes  (unsigned short)
///   magic_num:  4 bytes  (unsigned int)
///   reserved:   4 bytes  (unsigned int)
///   provider:  32 bytes  (unsigned char[32])
///   token:     32 bytes  (unsigned char[32])
///   body_len:   4 bytes  (unsigned int)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct YarHeader {
    pub id: u32,
    pub version: u16,
    pub magic_num: u32,
    pub reserved: u32,
    pub provider: [u8; 32],
    pub token: [u8; 32],
    pub body_len: u32,
}

impl YarHeader {
    /// Create a new zeroed header with magic number preset.
    pub fn new() -> Self {
        YarHeader {
            id: 0,
            version: 0,
            magic_num: PROTOCOL_MAGIC,
            reserved: 0,
            provider: [0; 32],
            token: [0; 32],
            body_len: 0,
        }
    }

    /// Render (populate) this header in host byte order, matching
    /// `yar_protocol_render()` from C.
    ///
    /// Provider and token copy at most 31 bytes and leave a trailing NUL,
    /// matching C's `strncpy(..., sizeof(field))` + forced terminator.
    pub fn render(
        &mut self,
        id: u32,
        provider: Option<&str>,
        token: Option<&str>,
        body_len: u32,
        reserved: u32,
    ) {
        self.magic_num = PROTOCOL_MAGIC;
        self.id = id;
        self.body_len = body_len;
        self.reserved = reserved;

        copy_name(&mut self.provider, provider);
        copy_name(&mut self.token, token);
    }

    /// Validate the magic number and convert an in-place wire-order header to
    /// host byte order.
    ///
    /// A header produced by [`render`]/[`new`] is already in host order and
    /// only needs validation. This method exists for callers that received raw
    /// struct bytes from the C API.
    pub fn parse(&mut self) -> YarResult<()> {
        if self.magic_num == PROTOCOL_MAGIC {
            return Ok(());
        }

        // The fields came from wire bytes: convert them in place.
        if u32::from_be(self.magic_num) == PROTOCOL_MAGIC {
            self.id = u32::from_be(self.id);
            self.version = u16::from_be(self.version);
            self.magic_num = PROTOCOL_MAGIC;
            self.reserved = u32::from_be(self.reserved);
            self.body_len = u32::from_be(self.body_len);
            return Ok(());
        }

        Err(YarError::InvalidMagic)
    }

    /// Serialize the header to wire format (big-endian numeric fields).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; HEADER_SIZE];

        BigEndian::write_u32(&mut buf[0..4], self.id);
        BigEndian::write_u16(&mut buf[4..6], self.version);
        BigEndian::write_u32(&mut buf[6..10], self.magic_num);
        BigEndian::write_u32(&mut buf[10..14], self.reserved);
        buf[14..46].copy_from_slice(&self.provider);
        buf[46..78].copy_from_slice(&self.token);
        BigEndian::write_u32(&mut buf[78..82], self.body_len);

        buf
    }

    /// Deserialize a header from wire bytes, validating the magic number.
    pub fn from_bytes(data: &[u8]) -> YarResult<Self> {
        if data.len() < HEADER_SIZE {
            return Err(YarError::Protocol(format!(
                "header too short: {} < {}",
                data.len(),
                HEADER_SIZE
            )));
        }

        let magic_num = BigEndian::read_u32(&data[6..10]);
        if magic_num != PROTOCOL_MAGIC {
            return Err(YarError::InvalidMagic);
        }

        let mut provider = [0u8; 32];
        provider.copy_from_slice(&data[14..46]);

        let mut token = [0u8; 32];
        token.copy_from_slice(&data[46..78]);

        Ok(YarHeader {
            id: BigEndian::read_u32(&data[0..4]),
            version: BigEndian::read_u16(&data[4..6]),
            magic_num,
            reserved: BigEndian::read_u32(&data[10..14]),
            provider,
            token,
            body_len: BigEndian::read_u32(&data[78..82]),
        })
    }
}

/// Copy a provider/token name into its fixed-width field, leaving a NUL at the
/// end like the C implementation.
fn copy_name(dst: &mut [u8; 32], src: Option<&str>) {
    dst.fill(0);
    if let Some(name) = src {
        let bytes = name.as_bytes();
        let len = bytes.len().min(31);
        dst[..len].copy_from_slice(&bytes[..len]);
    }
}

impl Default for YarHeader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_size() {
        // Fields: id(4)+version(2)+magic(4)+reserved(4)+provider(32)+token(32)+body_len(4) = 82
        assert_eq!(HEADER_SIZE, 82);
    }

    #[test]
    fn test_render_uses_host_order() {
        let mut header = YarHeader::new();
        header.render(0x12345678, None, None, 100, 42);

        assert_eq!(header.id, 0x12345678);
        assert_eq!(header.magic_num, PROTOCOL_MAGIC);
        assert_eq!(header.body_len, 100);
        assert_eq!(header.reserved, 42);
    }

    #[test]
    fn test_parse_validates_magic() {
        let mut header = YarHeader::new();
        header.render(1, None, None, 0, 0);

        assert!(header.parse().is_ok());
        assert_eq!(header.id, 1);
        assert_eq!(header.magic_num, PROTOCOL_MAGIC);
    }

    #[test]
    fn test_parse_rejects_bad_magic() {
        let mut header = YarHeader::new();
        // Write wrong magic in wire (network byte order) format
        header.magic_num = 0xDEADBEEFu32.to_be();
        assert!(header.parse().is_err());
    }

    #[test]
    fn test_render_parse_roundtrip() {
        let mut header = YarHeader::new();
        header.version = 1;
        header.render(99, Some("test-provider"), Some("test-token"), 256, 7);

        let mut parsed = header;
        assert!(parsed.parse().is_ok());

        assert_eq!(parsed.id, 99);
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.magic_num, PROTOCOL_MAGIC);
        assert_eq!(parsed.body_len, 256);
        assert_eq!(parsed.reserved, 7);
        assert_eq!(&parsed.provider[..13], b"test-provider");
        assert_eq!(&parsed.token[..10], b"test-token");
    }

    #[test]
    fn test_provider_token_keep_31_bytes() {
        let mut header = YarHeader::new();
        let long_str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        header.render(1, Some(long_str), Some(long_str), 0, 0);

        // C copies up to 32 bytes and forces a NUL at byte 31.
        assert_eq!(&header.provider[..31], &long_str.as_bytes()[..31]);
        assert_eq!(header.provider[31], 0);
        assert_eq!(&header.token[..31], &long_str.as_bytes()[..31]);
        assert_eq!(header.token[31], 0);
    }

    #[test]
    fn test_provider_token_none_zeroed() {
        let mut header = YarHeader::new();
        header.render(1, None, None, 0, 0);
        assert_eq!(header.provider, [0u8; 32]);
        assert_eq!(header.token, [0u8; 32]);
    }

    #[test]
    fn test_to_bytes_from_bytes_roundtrip() {
        let mut header = YarHeader::new();
        header.version = 3;
        header.render(42, Some("provider1"), Some("token1"), 1024, 0xFF);

        // Convert to host byte order first (render puts in network order)
        header.parse().unwrap();

        let bytes = header.to_bytes();
        assert_eq!(bytes.len(), HEADER_SIZE);

        let parsed = YarHeader::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.id, 42);
        assert_eq!(parsed.version, 3);
        assert_eq!(parsed.magic_num, PROTOCOL_MAGIC);
        assert_eq!(parsed.body_len, 1024);
        assert_eq!(parsed.reserved, 0xFF);
        assert_eq!(&parsed.provider[..9], b"provider1");
        assert_eq!(&parsed.token[..6], b"token1");
    }

    #[test]
    fn test_from_bytes_rejects_bad_magic() {
        let mut buf = vec![0u8; HEADER_SIZE];
        // Write a wrong magic number at offset 6 (after id:4 + version:2)
        BigEndian::write_u32(&mut buf[6..10], 0xDEADBEEF);
        assert!(YarHeader::from_bytes(&buf).is_err());
    }

    #[test]
    fn test_from_bytes_rejects_short_buffer() {
        let buf = [0u8; 10];
        assert!(YarHeader::from_bytes(&buf).is_err());
    }

    #[test]
    fn test_constants() {
        assert_eq!(PROTOCOL_PERSISTENT, 0x1);
        assert_eq!(PROTOCOL_PING, 0x2);
        assert_eq!(PROTOCOL_LIST, 0x4);
        assert_eq!(PROTOCOL_MAGIC, 0x80DFEC60);
    }

    #[test]
    fn test_c_binary_compatibility() {
        // Build a header as C render() would, verify the wire byte layout.
        // C: header->id = htonl(1)
        //    header->magic_num = htonl(0x80DFEC60)
        //    header->body_len = htonl(64)
        //    header->reserved = htonl(0)
        let mut header = YarHeader::new();
        header.render(1, None, None, 64, 0);

        // to_bytes() produces the wire layout.
        let raw = header.to_bytes();

        // Bytes 0..4: id = 1 in big-endian
        assert_eq!(BigEndian::read_u32(&raw[0..4]), 1);
        // Bytes 4..6: version = 0
        assert_eq!(BigEndian::read_u16(&raw[4..6]), 0);
        // Bytes 6..10: magic = 0x80DFEC60
        assert_eq!(BigEndian::read_u32(&raw[6..10]), PROTOCOL_MAGIC);
        // Bytes 10..14: reserved = 0
        assert_eq!(BigEndian::read_u32(&raw[10..14]), 0);
        // Bytes 14..46: provider (all zeros)
        assert_eq!(&raw[14..46], &[0u8; 32]);
        // Bytes 46..78: token (all zeros)
        assert_eq!(&raw[46..78], &[0u8; 32]);
        // Bytes 78..82: body_len = 64 in big-endian
        assert_eq!(BigEndian::read_u32(&raw[78..82]), 64);
    }

    #[test]
    fn test_from_bytes_with_c_layout() {
        // Construct bytes exactly as C would lay them out on the wire
        let mut buf = vec![0u8; HEADER_SIZE];
        let mut off = 0;

        BigEndian::write_u32(&mut buf[off..off + 4], 42); // id
        off += 4;
        BigEndian::write_u16(&mut buf[off..off + 2], 2); // version
        off += 2;
        BigEndian::write_u32(&mut buf[off..off + 4], PROTOCOL_MAGIC); // magic
        off += 4;
        BigEndian::write_u32(&mut buf[off..off + 4], PROTOCOL_PERSISTENT); // reserved
        off += 4;
        buf[off..off + 5].copy_from_slice(b"hello"); // provider
        off += 32;
        buf[off..off + 5].copy_from_slice(b"world"); // token
        off += 32;
        BigEndian::write_u32(&mut buf[off..off + 4], 512); // body_len

        let header = YarHeader::from_bytes(&buf).unwrap();
        assert_eq!(header.id, 42);
        assert_eq!(header.version, 2);
        assert_eq!(header.reserved, PROTOCOL_PERSISTENT);
        assert_eq!(header.body_len, 512);
        assert_eq!(&header.provider[..5], b"hello");
        assert_eq!(&header.token[..5], b"world");
    }
}
