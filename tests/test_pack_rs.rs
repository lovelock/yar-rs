//! Pack integration tests - C binary compatibility with known msgpack test vectors.
//!
//! Verifies that the Rust pack/unpack implementation produces byte-identical
//! output to the C msgpack wrapper (yar_pack.c) and can parse C-produced bytes.

#![allow(clippy::approx_constant)] // 3.14 is a deliberate C test vector

use yar_rs::*;

// ---------------------------------------------------------------------------
// Known msgpack byte vectors (matching C msgpack-c compact encoding output)
// ---------------------------------------------------------------------------

#[test]
fn test_pack_null_c_vector() {
    // C: yar_pack_start_null() -> [0xc0]
    let mut pk = Packager::new();
    pk.push_null().unwrap();
    assert_eq!(pk.as_bytes(), &[0xc0], "null must be 0xc0");
}

#[test]
fn test_pack_true_c_vector() {
    // C: yar_pack_push_bool(true) -> [0xc3]
    let mut pk = Packager::new();
    pk.push_bool(true).unwrap();
    assert_eq!(pk.as_bytes(), &[0xc3], "true must be 0xc3");
}

#[test]
fn test_pack_false_c_vector() {
    // C: yar_pack_push_bool(false) -> [0xc2]
    let mut pk = Packager::new();
    pk.push_bool(false).unwrap();
    assert_eq!(pk.as_bytes(), &[0xc2], "false must be 0xc2");
}

#[test]
fn test_pack_long_42_c_vector() {
    // C: yar_pack_push_long(42) with compact encoding -> [0x2a] (positive fixint)
    let mut pk = Packager::new();
    pk.push_long(42).unwrap();
    assert_eq!(pk.as_bytes(), &[0x2a], "long 42 must be fixint 0x2a");
}

#[test]
fn test_pack_ulong_1000_c_vector() {
    // C: yar_pack_push_ulong(1000) with compact encoding -> [0xcd, 0x03, 0xe8] (uint16)
    let mut pk = Packager::new();
    pk.push_ulong(1000).unwrap();
    assert_eq!(
        pk.as_bytes(),
        &[0xcd, 0x03, 0xe8],
        "ulong 1000 must be uint16: 0xcd 0x03 0xe8"
    );
}

#[test]
fn test_pack_double_3_14_c_vector() {
    // C: yar_pack_push_double(3.14) -> [0xcb, 0x40, 0x09, 0x1e, 0xb8, 0x51, 0xeb, 0x85, 0x1f]
    let mut pk = Packager::new();
    pk.push_double(3.14).unwrap();
    let expected: &[u8] = &[0xcb, 0x40, 0x09, 0x1e, 0xb8, 0x51, 0xeb, 0x85, 0x1f];
    assert_eq!(
        pk.as_bytes(),
        expected,
        "double 3.14 must be 9 bytes: 0xcb + IEEE 754 big-endian"
    );
}

#[test]
fn test_pack_string_hello_c_vector() {
    // C: yar_pack_push_string("hello", 5) -> [0xa5, 0x68, 0x65, 0x6c, 0x6c, 0x6f]
    let mut pk = Packager::new();
    pk.push_string(b"hello").unwrap();
    let expected: &[u8] = &[0xa5, 0x68, 0x65, 0x6c, 0x6c, 0x6f];
    assert_eq!(
        pk.as_bytes(),
        expected,
        "string 'hello' must be fixstr(5) + bytes"
    );
}

#[test]
fn test_pack_array_null_true_42_c_vector() {
    // C: yar_pack_start_array(3) + push_null + push_bool(true) + push_long(42)
    // -> [0x93, 0xc0, 0xc3, 0x2a]
    let mut pk = Packager::start_array(3).unwrap();
    pk.push_null().unwrap();
    pk.push_bool(true).unwrap();
    pk.push_long(42).unwrap();
    assert_eq!(
        pk.as_bytes(),
        &[0x93, 0xc0, 0xc3, 0x2a],
        "array [null, true, 42] must be 0x93 0xc0 0xc3 0x2a"
    );
}

// ---------------------------------------------------------------------------
// Unpack C-produced bytes (parse known byte sequences)
// ---------------------------------------------------------------------------

#[test]
fn test_unpack_c_null() {
    let unpk = Unpackager::new(&[0xc0]).unwrap();
    assert_eq!(unpk.data(), &YarData::Null);
    assert_eq!(unpk.data().data_type(), YarDataType::Null);
}

#[test]
fn test_unpack_c_bool() {
    let unpk = Unpackager::new(&[0xc3]).unwrap();
    assert_eq!(unpk.data().as_bool(), Some(true));

    let unpk = Unpackager::new(&[0xc2]).unwrap();
    assert_eq!(unpk.data().as_bool(), Some(false));
}

#[test]
fn test_unpack_c_fixint() {
    let unpk = Unpackager::new(&[0x2a]).unwrap();
    assert_eq!(unpk.data().as_ulong(), Some(42));
}

#[test]
fn test_unpack_c_uint16() {
    // uint16: 0xcd + 2 bytes big-endian
    let unpk = Unpackager::new(&[0xcd, 0x03, 0xe8]).unwrap();
    assert_eq!(unpk.data().as_ulong(), Some(1000));
}

#[test]
fn test_unpack_c_double() {
    let bytes: &[u8] = &[0xcb, 0x40, 0x09, 0x1e, 0xb8, 0x51, 0xeb, 0x85, 0x1f];
    let unpk = Unpackager::new(bytes).unwrap();
    let val = unpk.data().as_double().unwrap();
    assert!((val - 3.14).abs() < 1e-15, "expected 3.14, got {}", val);
}

#[test]
fn test_unpack_c_string() {
    let bytes: &[u8] = &[0xa5, 0x68, 0x65, 0x6c, 0x6c, 0x6f];
    let unpk = Unpackager::new(bytes).unwrap();
    assert_eq!(unpk.data().as_str(), Some("hello"));
    assert_eq!(unpk.data().as_bytes(), Some(b"hello".as_ref()));
    let (dt, sz) = unpk.data().data_type_and_size();
    assert_eq!(dt, YarDataType::String);
    assert_eq!(sz, 5);
}

#[test]
fn test_unpack_c_array() {
    // fixarray(3) + null + true + fixint(42)
    let bytes: &[u8] = &[0x93, 0xc0, 0xc3, 0x2a];
    let unpk = Unpackager::new(bytes).unwrap();

    let arr = unpk.data().as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0], YarData::Null);
    assert_eq!(arr[1].as_bool(), Some(true));
    assert_eq!(arr[2].as_ulong(), Some(42));
}

// ---------------------------------------------------------------------------
// Pack produces identical bytes (Rust output == C output)
// ---------------------------------------------------------------------------

#[test]
fn test_rust_pack_produces_c_identical_bytes() {
    // Verify multiple types produce the exact same bytes as C msgpack-c

    // Empty string: fixstr(0) = 0xa0
    let mut pk = Packager::new();
    pk.push_string(b"").unwrap();
    assert_eq!(pk.as_bytes(), &[0xa0]);

    // Zero: positive fixint 0 = 0x00
    let mut pk = Packager::new();
    pk.push_long(0).unwrap();
    assert_eq!(pk.as_bytes(), &[0x00]);

    // Negative fixint: -1 = 0xff
    let mut pk = Packager::new();
    pk.push_long(-1).unwrap();
    assert_eq!(pk.as_bytes(), &[0xff]);

    // Negative fixint: -32 = 0xe0
    let mut pk = Packager::new();
    pk.push_long(-32).unwrap();
    assert_eq!(pk.as_bytes(), &[0xe0]);

    // int8: -128 = 0xd0 0x80
    let mut pk = Packager::new();
    pk.push_long(-128).unwrap();
    assert_eq!(pk.as_bytes(), &[0xd0, 0x80]);

    // Positive fixint max: 127 = 0x7f
    let mut pk = Packager::new();
    pk.push_long(127).unwrap();
    assert_eq!(pk.as_bytes(), &[0x7f]);

    // Empty array: fixarray(0) = 0x90
    let pk = Packager::start_array(0).unwrap();
    assert_eq!(pk.as_bytes(), &[0x90]);

    // Empty map: fixmap(0) = 0x80
    let pk = Packager::start_map(0).unwrap();
    assert_eq!(pk.as_bytes(), &[0x80]);
}

// ---------------------------------------------------------------------------
// Roundtrip: pack then unpack
// ---------------------------------------------------------------------------

#[test]
fn test_roundtrip_all_types() {
    // Build a complex nested structure and verify roundtrip
    let data = YarData::Map(vec![
        (YarData::String(b"null".to_vec()), YarData::Null),
        (YarData::String(b"bool".to_vec()), YarData::Bool(true)),
        (YarData::String(b"long".to_vec()), YarData::Long(-42)),
        (YarData::String(b"ulong".to_vec()), YarData::Ulong(1000)),
        (YarData::String(b"double".to_vec()), YarData::Double(3.14)),
        (YarData::String(b"string".to_vec()), YarData::String(b"hello".to_vec())),
        (
            YarData::String(b"array".to_vec()),
            YarData::Array(vec![YarData::Ulong(1), YarData::Ulong(2), YarData::Null]),
        ),
    ]);

    let mut pk = Packager::new();
    pk.push_data(&data).unwrap();
    let bytes = pk.as_bytes();

    let unpk = Unpackager::new(bytes).unwrap();
    assert_eq!(unpk.data(), &data);
}

// ---------------------------------------------------------------------------
// Map and array with nested structures
// ---------------------------------------------------------------------------

#[test]
fn test_nested_map_in_array() {
    // Array: [{"key": "value"}, null, 42]
    let mut inner_map = Packager::start_map(1).unwrap();
    inner_map.push_string(b"key").unwrap();
    inner_map.push_string(b"value").unwrap();

    let mut arr = Packager::start_array(3).unwrap();
    arr.push_packager(&inner_map).unwrap();
    arr.push_null().unwrap();
    arr.push_long(42).unwrap();

    let bytes = arr.as_bytes();
    let unpk = Unpackager::new(bytes).unwrap();

    let array = unpk.data().as_array().unwrap();
    assert_eq!(array.len(), 3);

    let map = array[0].as_map().unwrap();
    assert_eq!(map.len(), 1);
    assert_eq!(map[0].0.as_str(), Some("key"));
    assert_eq!(map[0].1.as_str(), Some("value"));

    assert_eq!(array[1], YarData::Null);
    assert_eq!(array[2].as_ulong(), Some(42));
}

#[test]
fn test_push_packager_embeds_raw_bytes() {
    // Build inner packager with a string
    let mut inner = Packager::new();
    inner.push_string(b"embedded").unwrap();

    // Outer array containing the inner's bytes
    let mut outer = Packager::start_array(1).unwrap();
    outer.push_packager(&inner).unwrap();

    let bytes = outer.as_bytes();
    let unpk = Unpackager::new(bytes).unwrap();

    let arr = unpk.data().as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0].as_str(), Some("embedded"));
}

// ---------------------------------------------------------------------------
// Iterator tests (matching C yar_unpack_iterator)
// ---------------------------------------------------------------------------

#[test]
fn test_array_iterator_matches_c_behavior() {
    let data = YarData::Array(vec![
        YarData::Null,
        YarData::Bool(true),
        YarData::Ulong(42),
    ]);

    let mut it = data.iter().unwrap();
    assert_eq!(it.size(), 3);
    assert_eq!(it.position(), 0);

    // First item: null
    assert_eq!(it.current().data_type(), YarDataType::Null);

    // Second item: true
    assert_ne!(it.next(), 0);
    assert_eq!(it.current().as_bool(), Some(true));

    // Third item: 42
    assert_ne!(it.next(), 0);
    assert_eq!(it.current().as_ulong(), Some(42));

    // No more items: next() returns 0 (matching C behavior)
    assert_eq!(it.next(), 0);
}

#[test]
fn test_map_iterator_interleaved() {
    // Map iterator yields: key0, value0, key1, value1, ...
    // Size = entries * 2 (matching C behavior)
    let data = YarData::Map(vec![
        (YarData::String(b"name".to_vec()), YarData::String(b"yar".to_vec())),
        (YarData::String(b"version".to_vec()), YarData::Ulong(1)),
    ]);

    let mut it = data.iter().unwrap();
    assert_eq!(it.size(), 4); // 2 entries * 2

    // Key 0: "name"
    assert_eq!(it.current().as_str(), Some("name"));
    // Value 0: "yar"
    it.next();
    assert_eq!(it.current().as_str(), Some("yar"));
    // Key 1: "version"
    it.next();
    assert_eq!(it.current().as_str(), Some("version"));
    // Value 1: 1
    it.next();
    assert_eq!(it.current().as_ulong(), Some(1));
    // No more
    assert_eq!(it.next(), 0);
}

#[test]
fn test_iterator_reset() {
    let data = YarData::Array(vec![YarData::Ulong(1), YarData::Ulong(2)]);
    let mut it = data.iter().unwrap();

    assert_eq!(it.current().as_ulong(), Some(1));
    it.next();
    assert_eq!(it.current().as_ulong(), Some(2));

    it.reset();
    assert_eq!(it.position(), 0);
    assert_eq!(it.current().as_ulong(), Some(1));
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_unpack_empty_input_fails() {
    assert!(Unpackager::new(&[]).is_err());
}

#[test]
fn test_unpack_truncated_data_fails() {
    // Double with only 2 of 9 bytes
    assert!(Unpackager::new(&[0xcb, 0x00]).is_err());
    // String claiming 10 bytes but only 2 provided
    assert!(Unpackager::new(&[0xaa, 0x68, 0x65]).is_err());
}

#[test]
fn test_into_bytes_consumes_packager() {
    let mut pk = Packager::new();
    pk.push_null().unwrap();
    let bytes = pk.into_bytes();
    assert_eq!(bytes, vec![0xc0]);
}

#[test]
fn test_into_data_consumes_unpackager() {
    let unpk = Unpackager::new(&[0xc3]).unwrap();
    let data = unpk.into_data();
    assert_eq!(data, YarData::Bool(true));
}

#[test]
fn test_data_type_and_size() {
    assert_eq!(YarData::Null.data_type_and_size(), (YarDataType::Null, 0));
    assert_eq!(YarData::Bool(true).data_type_and_size(), (YarDataType::Bool, 0));
    assert_eq!(YarData::Long(-5).data_type_and_size(), (YarDataType::Long, 0));
    assert_eq!(YarData::Ulong(10).data_type_and_size(), (YarDataType::Ulong, 0));
    assert_eq!(YarData::Double(1.0).data_type_and_size(), (YarDataType::Double, 0));
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
fn test_as_integer_converts_both_types() {
    assert_eq!(YarData::Long(-5).as_integer(), Some(-5));
    assert_eq!(YarData::Ulong(10).as_integer(), Some(10));
    assert_eq!(YarData::Null.as_integer(), None);
    assert_eq!(YarData::Bool(true).as_integer(), None);
}

#[test]
fn test_binary_data_roundtrip() {
    let binary: Vec<u8> = vec![0x00, 0x01, 0x02, 0xff, 0xfe, 0x80];
    let mut pk = Packager::new();
    pk.push_string(&binary).unwrap();

    let unpk = Unpackager::new(pk.as_bytes()).unwrap();
    assert_eq!(unpk.data().as_bytes(), Some(binary.as_slice()));
}
