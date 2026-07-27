//! Yar protocol header - binary compatible with the C implementation.
//!
//! The header is packed (no padding), with fields stored in network byte order
//! (big-endian) on the wire. The C struct comment says "Total: 80 bytes" but
//! the actual fields sum to 82 (4+2+4+4+32+32+4).

use byteorder::{BigEndian, ByteOrder};
use std::mem;

/// Protocol flag: persistent connection
pub const PROTOCOL_PERSISTENT: u32 = 0x1;
/// Protocol flag: ping
pub const PROTOCOL_PING: u32 = 0x2;
/// Protocol flag: list
pub const PROTOCOL_LIST: u32 = 0x4;

/// Protocol magic number (native byte order value)
pub const PROTOCOL_MAGIC: u32 = 0x80DFEC60;

/// Size of the packed header in bytes.
/// Derived from the actual struct layout: id(4)+version(2)+magic(4)+reserved(4)+provider(32)+token(32)+body_len(4) = 82
pub const HEADER_SIZE: usize = mem::size_of::<YarHeader>();

/// Yar protocol header, matching the C `yar_header` struct exactly.
///
/// Layout (packed, no padding):
///   id:         4 bytes  (unsigned int)
///   version:    2 bytes  (unsigned short)
///   magic_num:  4 bytes  (unsigned int)
///   reserved:   4 bytes  (unsigned int)
///   provider:  32 bytes  (unsigned char[32])
///   token:     32 bytes  (unsigned char[32])
///   body_len:   4 bytes  (unsigned int)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
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

    /// Render (populate) this header, matching `yar_protocol_render()` from C.
    ///
    /// Sets id, magic_num, body_len, reserved in network byte order and copies
    /// up to 16 bytes of provider and token (matching the C implementation which
    /// only copies 16 bytes despite the 32-byte fields).
    pub fn render(
        &mut self,
        id: u32,
        provider: Option<&str>,
        token: Option<&str>,
        body_len: u32,
        reserved: u32,
    ) {
        self.magic_num = PROTOCOL_MAGIC.to_be();
        self.id = id.to_be();
        self.body_len = body_len.to_be();
        self.reserved = reserved.to_be();

        self.provider = [0u8; 32];
        if let Some(p) = provider {
            let bytes = p.as_bytes();
            let len = std::cmp::min(bytes.len(), 16);
            self.provider[..len].copy_from_slice(&bytes[..len]);
        }

        self.token = [0u8; 32];
        if let Some(t) = token {
            let bytes = t.as_bytes();
            let len = std::cmp::min(bytes.len(), 16);
            self.token[..len].copy_from_slice(&bytes[..len]);
        }
    }

    /// Parse this header from wire format, converting fields from network byte
    /// order to host byte order and validating the magic number.
    ///
    /// Returns `Ok(())` on success, or `Err(())` if the magic number does not match.
    pub fn parse(&mut self) -> Result<(), ()> {
        let magic = self.magic_num.to_be();
        if magic != PROTOCOL_MAGIC {
            return Err(());
        }
        self.magic_num = magic;
        self.id = self.id.to_be();
        self.body_len = self.body_len.to_be();
        self.reserved = self.reserved.to_be();
        Ok(())
    }

    /// Serialize the header to a byte buffer in network byte order (big-endian).
    ///
    /// The header fields are expected to be in host byte order.
    /// This method produces wire-format bytes suitable for sending over the network.
    pub fn to_bytes(&self) -> Vec<u8> {
        let id = { self.id };
        let version = { self.version };
        let magic_num = { self.magic_num };
        let reserved = { self.reserved };
        let provider = self.provider;
        let token = self.token;
        let body_len = { self.body_len };

        let mut buf = vec![0u8; HEADER_SIZE];
        let mut off = 0;

        BigEndian::write_u32(&mut buf[off..off + 4], id);
        off += 4;

        BigEndian::write_u16(&mut buf[off..off + 2], version);
        off += 2;

        BigEndian::write_u32(&mut buf[off..off + 4], magic_num);
        off += 4;

        BigEndian::write_u32(&mut buf[off..off + 4], reserved);
        off += 4;

        buf[off..off + 32].copy_from_slice(&provider);
        off += 32;

        buf[off..off + 32].copy_from_slice(&token);
        let _ = off + 32;

        BigEndian::write_u32(&mut buf[78..82], body_len);

        buf
    }

    /// Deserialize a header from a byte slice in network byte order.
    /// Validates the magic number and returns `Err(())` if it does not match or
    /// if the buffer is too short.
    pub fn from_bytes(data: &[u8]) -> Result<Self, ()> {
        if data.len() < HEADER_SIZE {
            return Err(());
        }

        let mut off = 0;

        let id = BigEndian::read_u32(&data[off..off + 4]);
        off += 4;

        let version = BigEndian::read_u16(&data[off..off + 2]);
        off += 2;

        let magic_num = BigEndian::read_u32(&data[off..off + 4]);
        off += 4;

        let reserved = BigEndian::read_u32(&data[off..off + 4]);
        off += 4;

        let mut provider = [0u8; 32];
        provider.copy_from_slice(&data[off..off + 32]);
        off += 32;

        let mut token = [0u8; 32];
        token.copy_from_slice(&data[off..off + 32]);
        off += 32;

        let body_len = BigEndian::read_u32(&data[off..off + 4]);
        let _ = off;

        if magic_num != PROTOCOL_MAGIC {
            return Err(());
        }

        Ok(YarHeader {
            id,
            version,
            magic_num,
            reserved,
            provider,
            token,
            body_len,
        })
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

    /// Helper: safely read a u32 from a packed YarHeader field
    macro_rules! packed_u32 {
        ($h:expr, $field:ident) => {{
            let h = &$h;
            unsafe { std::ptr::read_unaligned(std::ptr::addr_of!(h.$field)) }
        }};
    }

    /// Helper: safely read a u16 from a packed YarHeader field
    macro_rules! packed_u16 {
        ($h:expr, $field:ident) => {{
            let h = &$h;
            unsafe { std::ptr::read_unaligned(std::ptr::addr_of!(h.$field)) }
        }};
    }

    #[test]
    fn test_header_size() {
        // Fields: id(4)+version(2)+magic(4)+reserved(4)+provider(32)+token(32)+body_len(4) = 82
        assert_eq!(mem::size_of::<YarHeader>(), HEADER_SIZE);
        assert_eq!(mem::size_of::<YarHeader>(), 82);
    }

    #[test]
    fn test_render_network_byte_order() {
        let mut header = YarHeader::new();
        header.render(0x12345678, None, None, 100, 42);

        assert_eq!(u32::from_be(packed_u32!(header, id)), 0x12345678);
        assert_eq!(u32::from_be(packed_u32!(header, magic_num)), PROTOCOL_MAGIC);
        assert_eq!(u32::from_be(packed_u32!(header, body_len)), 100);
        assert_eq!(u32::from_be(packed_u32!(header, reserved)), 42);
    }

    #[test]
    fn test_parse_validates_magic() {
        let mut header = YarHeader::new();
        header.render(1, None, None, 0, 0);

        assert!(header.parse().is_ok());
        assert_eq!(packed_u32!(header, id), 1);
        assert_eq!(packed_u32!(header, magic_num), PROTOCOL_MAGIC);
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

        assert_eq!(packed_u32!(parsed, id), 99);
        assert_eq!(packed_u16!(parsed, version), 1);
        assert_eq!(packed_u32!(parsed, magic_num), PROTOCOL_MAGIC);
        assert_eq!(packed_u32!(parsed, body_len), 256);
        assert_eq!(packed_u32!(parsed, reserved), 7);
        assert_eq!(&parsed.provider[..13], b"test-provider");
        assert_eq!(&parsed.token[..10], b"test-token");
    }

    #[test]
    fn test_provider_token_only_16_bytes() {
        let mut header = YarHeader::new();
        let long_str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        header.render(1, Some(long_str), Some(long_str), 0, 0);

        // Only first 16 bytes should be copied, rest stays zeroed
        assert_eq!(&header.provider[..16], b"ABCDEFGHIJKLMNOP");
        assert_eq!(&header.provider[16..32], &[0u8; 16]);
        assert_eq!(&header.token[..16], b"ABCDEFGHIJKLMNOP");
        assert_eq!(&header.token[16..32], &[0u8; 16]);
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
        assert_eq!(packed_u32!(parsed, id), 42);
        assert_eq!(packed_u16!(parsed, version), 3);
        assert_eq!(packed_u32!(parsed, magic_num), PROTOCOL_MAGIC);
        assert_eq!(packed_u32!(parsed, body_len), 1024);
        assert_eq!(packed_u32!(parsed, reserved), 0xFF);
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

        // The rendered struct bytes should match network byte order directly.
        // Since render() stores in network byte order, we read raw struct bytes.
        let raw: [u8; HEADER_SIZE] = unsafe {
            std::ptr::read_unaligned(&header as *const YarHeader as *const [u8; HEADER_SIZE])
        };

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
        assert_eq!(packed_u32!(header, id), 42);
        assert_eq!(packed_u16!(header, version), 2);
        assert_eq!(packed_u32!(header, reserved), PROTOCOL_PERSISTENT);
        assert_eq!(packed_u32!(header, body_len), 512);
        assert_eq!(&header.provider[..5], b"hello");
        assert_eq!(&header.token[..5], b"world");
    }
}
