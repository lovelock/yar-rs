//! Cross-language (wire-format) verification tests.
//!
//! These tests construct wire packets exactly as the original C implementation
//! would produce them and verify that the Rust code can parse them byte-for-byte.
//! The original C implementation (legacy/) has been removed; these recorded
//! wire formats remain the compatibility contract.
//!
//! Run with: cargo test --test test_cross_language

use yar_rs::*;

// ---------------------------------------------------------------------------
// Wire-level cross-language verification
// ---------------------------------------------------------------------------

/// Helper: safely read a u32 from a packed YarHeader field.
macro_rules! packed_u32 {
    ($h:expr, $field:ident) => {{
        let h = &$h;
        unsafe { std::ptr::read_unaligned(std::ptr::addr_of!(h.$field)) }
    }};
}

#[test]
fn test_cross_language_wire_format_compatibility() {
    // This test verifies wire format compatibility without needing a running C server.
    // We construct packets as the C client would and verify the Rust parser can read them,
    // and vice versa.

    // 1. Build a request packet as C would produce it
    let mut wire_request = vec![0u8; HEADER_SIZE + PACKAGER_NAME.len()];

    // Write header in C wire format (big-endian)
    use byteorder::{BigEndian, ByteOrder};
    BigEndian::write_u32(&mut wire_request[0..4], 1000);            // id
    BigEndian::write_u16(&mut wire_request[4..6], 0);               // version
    BigEndian::write_u32(&mut wire_request[6..10], PROTOCOL_MAGIC); // magic
    BigEndian::write_u32(&mut wire_request[10..14], 0);             // reserved
    // provider[14..46] stays zero
    // token[46..78] stays zero

    // Write packager name
    wire_request[HEADER_SIZE..HEADER_SIZE + 8].copy_from_slice(PACKAGER_NAME);

    // Build the msgpack request body: {i:1000, m:"echo", p:null}
    let req = YarRequest::new(1000, "echo");
    let msgpack_body = req.pack(0).unwrap();

    // Set body_len in header
    let body_len = (PACKAGER_NAME.len() + msgpack_body.len()) as u32;
    BigEndian::write_u32(&mut wire_request[78..82], body_len);

    // Combine into full packet
    wire_request.extend_from_slice(&msgpack_body);

    // 2. Verify the Rust server-side code can parse this C-produced packet
    // Parse header
    let header = YarHeader::from_bytes(&wire_request[..HEADER_SIZE]).unwrap();
    assert_eq!(packed_u32!(header, id), 1000);
    assert_eq!(packed_u32!(header, magic_num), PROTOCOL_MAGIC);
    assert_eq!(packed_u32!(header, body_len) as usize, PACKAGER_NAME.len() + msgpack_body.len());

    // Parse packager name
    let body_start = HEADER_SIZE;
    let packager_name = &wire_request[body_start..body_start + PACKAGER_NAME.len()];
    assert_eq!(packager_name, PACKAGER_NAME.as_slice());

    // Parse request
    let req_body_start = body_start + PACKAGER_NAME.len();
    let parsed_req = YarRequest::unpack(&wire_request[req_body_start..], 0).unwrap();
    assert_eq!(parsed_req.id(), 1000);
    assert_eq!(parsed_req.method(), "echo");

    // 3. Build a response as the Rust server would, verify C-compatible format
    let mut retval = Packager::new();
    retval.push_string(b"echo:ok").unwrap();
    let resp = YarResponse::success(1000, retval);

    let extra_bytes = HEADER_SIZE + PACKAGER_NAME.len();
    let mut resp_payload = resp.pack(extra_bytes).unwrap();

    // Build response header in wire format
    let resp_body_len = (resp_payload.len() - HEADER_SIZE) as u32;
    BigEndian::write_u32(&mut resp_payload[0..4], 1000);            // id
    BigEndian::write_u16(&mut resp_payload[4..6], 0);               // version
    BigEndian::write_u32(&mut resp_payload[6..10], PROTOCOL_MAGIC); // magic
    BigEndian::write_u32(&mut resp_payload[10..14], 0);             // reserved
    BigEndian::write_u32(&mut resp_payload[78..82], resp_body_len); // body_len

    // Write packager name
    resp_payload[HEADER_SIZE..HEADER_SIZE + 8].copy_from_slice(PACKAGER_NAME);

    // 4. Verify the response can be parsed back (as C client would)
    let resp_header = YarHeader::from_bytes(&resp_payload[..HEADER_SIZE]).unwrap();
    assert_eq!(packed_u32!(resp_header, id), 1000);
    assert_eq!(packed_u32!(resp_header, magic_num), PROTOCOL_MAGIC);

    let resp_body = &resp_payload[HEADER_SIZE + PACKAGER_NAME.len()..];
    let parsed_resp = YarResponse::unpack(resp_body, 0).unwrap();
    assert_eq!(parsed_resp.id, 1000);
    assert_eq!(parsed_resp.get_status(), 0);
    assert_eq!(parsed_resp.get_response().unwrap().as_str(), Some("echo:ok"));
}

// ---------------------------------------------------------------------------
// Protocol-level compatibility: header byte layout
// ---------------------------------------------------------------------------

#[test]
fn test_cross_language_header_byte_compatibility() {
    // Verify that a header produced by Rust is byte-identical to what C expects
    let mut header = YarHeader::new();
    header.render(42, Some("RustClient"), None, 256, PROTOCOL_PERSISTENT);

    // Read raw struct bytes
    let raw: [u8; HEADER_SIZE] = unsafe {
        std::ptr::read_unaligned(&header as *const YarHeader as *const [u8; HEADER_SIZE])
    };

    // C expects these fields in network byte order at these offsets:
    use byteorder::{BigEndian, ByteOrder};
    assert_eq!(BigEndian::read_u32(&raw[0..4]), 42);             // id
    assert_eq!(BigEndian::read_u32(&raw[6..10]), PROTOCOL_MAGIC); // magic
    assert_eq!(BigEndian::read_u32(&raw[10..14]), PROTOCOL_PERSISTENT); // reserved
    assert_eq!(&raw[14..24], b"RustClient");                      // provider
    assert_eq!(BigEndian::read_u32(&raw[78..82]), 256);          // body_len
}
