# Data Model

[Back to the guide index](README.md)

Yar payloads are msgpack. `yar-rs` exposes three pieces:

| Type | Role |
|---|---|
| `Packager` | Incremental msgpack builder |
| `Unpackager` | Parses msgpack bytes into a `YarData` tree |
| `YarData` | Parsed value tree (null, bool, integer, double, string, array, map) |

## Types

| `YarData` | msgpack | Build | Read |
|---|---|---|---|
| `Null` | nil | `push_null()` | matches `YarData::Null` |
| `Bool` | true/false | `push_bool(bool)` | `as_bool()` |
| `Long` | signed integer | `push_long(i64)` | `as_long()` |
| `Ulong` | unsigned integer | `push_ulong(u64)` | `as_ulong()` |
| `Double` | float64 | `push_double(f64)` | `as_double()` |
| `String` | str (arbitrary bytes) | `push_string(&[u8])` | `as_bytes()`, `as_str()` |
| `Array` | array | `start_array(n)` / `push_array(n)` | `as_array()`, `iter()` |
| `Map` | map | `start_map(n)` / `push_map(n)` | `as_map()`, `iter()` |

`as_str()` returns `None` for non-UTF-8 strings; `as_bytes()` always works
for a string.

## Integers and the compact encoding

msgpack uses the shortest encoding. A non-negative value written with
`push_long` is encoded in the unsigned format, so it decodes as `Ulong`:

- `push_long(42)` → decodes as `Ulong(42)`
- `push_long(-42)` → decodes as `Long(-42)`
- `push_ulong(42)` → decodes as `Ulong(42)`
- `as_long()` returns `None` for a `Ulong`

Read any integer with `as_integer() -> Option<i64>`, which accepts both
variants and returns `None` for a `Ulong` above `i64::MAX` instead of
wrapping.

## Building a nested value

```rust,ignore
let mut user = Packager::start_map(2)?;
user.push_string(b"name")?;
user.push_string(b"alice")?;
user.push_string(b"age")?;
user.push_long(30)?;

let mut args = Packager::start_array(1)?;
args.push_packager(&user)?;          // splice the encoded map
```

`push_packager` appends the other packager's bytes, so it avoids re-encoding
a tree. `push_data` serializes a `YarData` tree and deep-copies strings.

## Reading

```rust,ignore
let data = Unpackager::new(payload)?.into_data();
if let Some(map) = data.as_map() {
    for (key, value) in map {
        println!("{:?} = {:?}", key.as_str(), value);
    }
}
```

`iter()` yields array elements, or map keys and values interleaved
(`key0, value0, key1, value1, ...`). It returns `None` for scalars.

## Parser limits

The parser is defensive because requests are untrusted:

- **Depth**: at most 128 nested containers.
- **Nodes**: at most 262,144 msgpack objects per payload.
- **Lengths**: a declared string/array/map length larger than the remaining
  input is rejected before allocation.
- **Trailing bytes**: the payload must contain exactly one msgpack object.

These limits are exposed as `MAX_PARSE_DEPTH` and `MAX_PARSE_NODES`.

## Wire compatibility

`Packager` produces the same compact msgpack bytes as the C implementation
(including the integer encoding above), and `Unpackager` accepts the C
encoder's output. See [Protocol](protocol.md) for the envelopes that wrap
this data.
