//! Protocol integration tests - C binary compatibility verification.
//!
//! These tests verify that the Rust protocol implementation produces
//! wire-compatible bytes with the C yar_protocol implementation.

use byteorder::{BigEndian, ByteOrder};
use yar_rs::*;

// ---------------------------------------------------------------------------
// Header size
// ---------------------------------------------------------------------------

#[test]
fn test_header_size_is_82_bytes() {
    // C comment: "Total: 80 bytes" but actual fields sum to 82:
    // id(4) + version(2) + magic(4) + reserved(4) + provider(32) + token(32) + body_len(4) = 82
    assert_eq!(HEADER_SIZE, 82);
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

#[test]
fn test_protocol_constants() {
    assert_eq!(PROTOCOL_MAGIC, 0x80DFEC60);
    assert_eq!(PROTOCOL_PERSISTENT, 0x1);
    assert_eq!(PROTOCOL_PING, 0x2);
    assert_eq!(PROTOCOL_LIST, 0x4);
}

#[test]
fn test_flags_are_independent_bits() {
    assert_eq!(PROTOCOL_PERSISTENT & PROTOCOL_PING, 0);
    assert_eq!(PROTOCOL_PERSISTENT & PROTOCOL_LIST, 0);
    assert_eq!(PROTOCOL_PING & PROTOCOL_LIST, 0);
    // All flags combined
    assert_eq!(PROTOCOL_PERSISTENT | PROTOCOL_PING | PROTOCOL_LIST, 0x7);
}

// ---------------------------------------------------------------------------
// C binary compatibility: render produces correct wire bytes
// ---------------------------------------------------------------------------

#[test]
fn test_render_produces_c_compatible_bytes() {
    // Simulate what C's yar_protocol_render() does:
    //   header->id = htonl(1)
    //   header->magic_num = htonl(0x80DFEC60)
    //   header->body_len = htonl(64)
    //   header->reserved = htonl(0)
    let mut header = YarHeader::new();
    header.render(1, None, None, 64, 0);

    // Read the raw struct bytes (render stores in network byte order)
    let raw = header.to_bytes();

    // Bytes 0..4: id = 1 in big-endian
    assert_eq!(BigEndian::read_u32(&raw[0..4]), 1);
    // Bytes 4..6: version = 0 (not set by render)
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
fn test_render_with_provider_and_token() {
    let mut header = YarHeader::new();
    header.render(
        42,
        Some("my-provider"),
        Some("my-token"),
        1024,
        PROTOCOL_PERSISTENT,
    );

    let raw = header.to_bytes();

    assert_eq!(BigEndian::read_u32(&raw[0..4]), 42);
    assert_eq!(BigEndian::read_u32(&raw[6..10]), PROTOCOL_MAGIC);
    assert_eq!(BigEndian::read_u32(&raw[10..14]), PROTOCOL_PERSISTENT);
    assert_eq!(BigEndian::read_u32(&raw[78..82]), 1024);

    // Provider: "my-provider" (11 bytes) copied into bytes 14..46
    assert_eq!(&raw[14..25], b"my-provider");
    assert_eq!(&raw[25..46], &[0u8; 21]); // rest zeroed

    // Token: "my-token" (8 bytes) copied into bytes 46..78
    assert_eq!(&raw[46..54], b"my-token");
    assert_eq!(&raw[54..78], &[0u8; 24]); // rest zeroed
}

// ---------------------------------------------------------------------------
// Provider/token truncation at 16 bytes (matching C behavior)
// ---------------------------------------------------------------------------

#[test]
fn test_provider_token_keep_31_bytes() {
    // C uses strncpy(..., sizeof(field)) and forces byte 31 to NUL, so up to
    // 31 bytes of provider/token survive.
    let long_str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"; // 36 bytes
    let mut header = YarHeader::new();
    header.render(1, Some(long_str), Some(long_str), 0, 0);

    assert_eq!(&header.provider[..31], &long_str.as_bytes()[..31]);
    assert_eq!(header.provider[31], 0);
    assert_eq!(&header.token[..31], &long_str.as_bytes()[..31]);
    assert_eq!(header.token[31], 0);
}

#[test]
fn render_then_to_bytes_is_wire_compatible() {
    let mut header = YarHeader::new();
    header.render(0x12345678, Some("provider"), Some("token"), 100, 7);

    let bytes = header.to_bytes();
    assert_eq!(BigEndian::read_u32(&bytes[0..4]), 0x12345678);
    assert_eq!(BigEndian::read_u32(&bytes[6..10]), PROTOCOL_MAGIC);
    assert_eq!(BigEndian::read_u32(&bytes[10..14]), 7);
    assert_eq!(BigEndian::read_u32(&bytes[78..82]), 100);

    let parsed = YarHeader::from_bytes(&bytes).unwrap();
    assert_eq!(parsed.id, 0x12345678);
    assert_eq!(parsed.magic_num, PROTOCOL_MAGIC);
    assert_eq!(parsed.body_len, 100);
    assert_eq!(parsed.reserved, 7);
}

#[test]
fn test_provider_token_none_is_zeroed() {
    let mut header = YarHeader::new();
    header.render(1, None, None, 0, 0);
    assert_eq!(header.provider, [0u8; 32]);
    assert_eq!(header.token, [0u8; 32]);
}

// ---------------------------------------------------------------------------
// from_bytes: parse C-produced wire bytes
// ---------------------------------------------------------------------------

#[test]
fn test_from_bytes_parses_c_wire_format() {
    // Construct bytes exactly as C would lay them out on the wire
    let mut buf = vec![0u8; HEADER_SIZE];

    BigEndian::write_u32(&mut buf[0..4], 42); // id
    BigEndian::write_u16(&mut buf[4..6], 2); // version
    BigEndian::write_u32(&mut buf[6..10], PROTOCOL_MAGIC); // magic
    BigEndian::write_u32(&mut buf[10..14], PROTOCOL_PERSISTENT); // reserved
    buf[14..19].copy_from_slice(b"hello"); // provider
    buf[46..51].copy_from_slice(b"world"); // token
    BigEndian::write_u32(&mut buf[78..82], 512); // body_len

    let header = YarHeader::from_bytes(&buf).unwrap();
    assert_eq!(header.id, 42);
    assert_eq!(header.version, 2);
    assert_eq!(header.magic_num, PROTOCOL_MAGIC);
    assert_eq!(header.reserved, PROTOCOL_PERSISTENT);
    assert_eq!(header.body_len, 512);
    assert_eq!(&header.provider[..5], b"hello");
    assert_eq!(&header.token[..5], b"world");
}

#[test]
fn test_from_bytes_rejects_bad_magic() {
    let mut buf = vec![0u8; HEADER_SIZE];
    BigEndian::write_u32(&mut buf[6..10], 0xDEADBEEF); // wrong magic
    assert!(YarHeader::from_bytes(&buf).is_err());
}

#[test]
fn test_from_bytes_rejects_short_buffer() {
    let buf = [0u8; 10];
    assert!(YarHeader::from_bytes(&buf).is_err());
}

// ---------------------------------------------------------------------------
// to_bytes/from_bytes roundtrip
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Render/parse roundtrip
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Parse validates magic
// ---------------------------------------------------------------------------

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
    header.magic_num = 0xDEADBEEFu32.to_be();
    assert!(header.parse().is_err());
}

// ---------------------------------------------------------------------------
// All protocol flags
// ---------------------------------------------------------------------------

#[test]
fn test_persistent_flag_in_reserved() {
    let mut header = YarHeader::new();
    header.render(1, None, None, 0, PROTOCOL_PERSISTENT);

    let raw = header.to_bytes();
    assert_eq!(BigEndian::read_u32(&raw[10..14]), PROTOCOL_PERSISTENT);
}

#[test]
fn test_ping_flag_in_reserved() {
    let mut header = YarHeader::new();
    header.render(1, None, None, 0, PROTOCOL_PING);

    let raw = header.to_bytes();
    assert_eq!(BigEndian::read_u32(&raw[10..14]), PROTOCOL_PING);
}

#[test]
fn test_list_flag_in_reserved() {
    let mut header = YarHeader::new();
    header.render(1, None, None, 0, PROTOCOL_LIST);

    let raw = header.to_bytes();
    assert_eq!(BigEndian::read_u32(&raw[10..14]), PROTOCOL_LIST);
}

#[test]
fn test_combined_flags_in_reserved() {
    let flags = PROTOCOL_PERSISTENT | PROTOCOL_PING | PROTOCOL_LIST;
    let mut header = YarHeader::new();
    header.render(1, None, None, 0, flags);

    let raw = header.to_bytes();
    assert_eq!(BigEndian::read_u32(&raw[10..14]), flags);

    // Parse and verify flags survive roundtrip
    let parsed = YarHeader::from_bytes(&raw).unwrap();
    assert_eq!(parsed.reserved, flags);
}

// ---------------------------------------------------------------------------
// Wire byte layout verification
// ---------------------------------------------------------------------------

#[test]
fn test_wire_byte_layout_matches_c() {
    // Build a complete wire-format header as C would produce it
    let mut wire = vec![0u8; HEADER_SIZE];
    BigEndian::write_u32(&mut wire[0..4], 0x12345678); // id
    BigEndian::write_u16(&mut wire[4..6], 0x0001); // version
    BigEndian::write_u32(&mut wire[6..10], PROTOCOL_MAGIC); // magic
    BigEndian::write_u32(&mut wire[10..14], 0x00000003); // reserved (PERSISTENT|PING)
                                                         // provider at 14..46: "test\0..." (only first 4 bytes non-zero)
    wire[14..18].copy_from_slice(b"test");
    // token at 46..78: all zeros
    BigEndian::write_u32(&mut wire[78..82], 0x00000100); // body_len = 256

    let header = YarHeader::from_bytes(&wire).unwrap();
    assert_eq!(header.id, 0x12345678);
    assert_eq!(header.version, 1);
    assert_eq!(header.magic_num, PROTOCOL_MAGIC);
    assert_eq!(header.reserved, 3);
    assert_eq!(header.body_len, 256);
    assert_eq!(&header.provider[..4], b"test");

    // to_bytes should reproduce the exact same wire format
    let re_encoded = header.to_bytes();
    assert_eq!(re_encoded, wire);
}

// ---------------------------------------------------------------------------
// Packager name constant
// ---------------------------------------------------------------------------

#[test]
fn test_packager_name_constant() {
    assert_eq!(PACKAGER_NAME, b"MSGPACK\0");
    assert_eq!(PACKAGER_NAME.len(), 8);
    assert_eq!(PACKAGER_NAME_STR, "MSGPACK");
}

// ---------------------------------------------------------------------------
// Version
// ---------------------------------------------------------------------------

#[test]
fn test_version_constant() {
    assert_eq!(yar_rs::VERSION, env!("CARGO_PKG_VERSION"));
}

// ---------------------------------------------------------------------------
// Default header
// ---------------------------------------------------------------------------

#[test]
fn test_default_header_has_magic() {
    let header = YarHeader::new();
    assert_eq!(header.magic_num, PROTOCOL_MAGIC);
    assert_eq!(header.id, 0);
    assert_eq!(header.body_len, 0);
    assert_eq!(header.reserved, 0);
    assert_eq!(header.provider, [0u8; 32]);
    assert_eq!(header.token, [0u8; 32]);
}
