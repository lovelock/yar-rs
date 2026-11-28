# 数据模型

[返回文档索引](README.md)

Yar 载荷使用 msgpack。`yar-rs` 暴露三部分：

| 类型 | 作用 |
|---|---|
| `Packager` | 增量式 msgpack 构建器 |
| `Unpackager` | 把 msgpack 字节解析为 `YarData` 树 |
| `YarData` | 解析后的值树（null、bool、整数、double、string、array、map） |

## 类型

| `YarData` | msgpack | 构造 | 读取 |
|---|---|---|---|
| `Null` | nil | `push_null()` | 匹配 `YarData::Null` |
| `Bool` | true/false | `push_bool(bool)` | `as_bool()` |
| `Long` | 有符号整数 | `push_long(i64)` | `as_long()` |
| `Ulong` | 无符号整数 | `push_ulong(u64)` | `as_ulong()` |
| `Double` | float64 | `push_double(f64)` | `as_double()` |
| `String` | str（任意字节） | `push_string(&[u8])` | `as_bytes()`、`as_str()` |
| `Array` | array | `start_array(n)` / `push_array(n)` | `as_array()`、`iter()` |
| `Map` | map | `start_map(n)` / `push_map(n)` | `as_map()`、`iter()` |

非 UTF-8 字符串的 `as_str()` 返回 `None`；`as_bytes()` 始终可用。

## 整数与紧凑编码

msgpack 使用最短编码。`push_long` 写入的非负值会落在无符号格式区，因此解码为
`Ulong`：

- `push_long(42)` → 解码为 `Ulong(42)`
- `push_long(-42)` → 解码为 `Long(-42)`
- `push_ulong(42)` → 解码为 `Ulong(42)`
- 对 `Ulong` 调用 `as_long()` 返回 `None`

读取任意整数请用 `as_integer() -> Option<i64>`：它同时接受两种变体；超过
`i64::MAX` 的 `Ulong` 返回 `None`，不会回绕。

## 构造嵌套值

```rust,ignore
let mut user = Packager::start_map(2)?;
user.push_string(b"name")?;
user.push_string(b"alice")?;
user.push_string(b"age")?;
user.push_long(30)?;

let mut args = Packager::start_array(1)?;
args.push_packager(&user)?;          // 拼接已编码的 map
```

`push_packager` 直接追加另一个 packager 的字节，避免重新编码；`push_data` 序列化
一棵 `YarData` 树，并深拷贝字符串。

## 读取

```rust,ignore
let data = Unpackager::new(payload)?.into_data();
if let Some(map) = data.as_map() {
    for (key, value) in map {
        println!("{:?} = {:?}", key.as_str(), value);
    }
}
```

`iter()` 对数组返回元素，对 map 返回键值交替（`key0, value0, key1, value1, ...`）；
标量返回 `None`。

## 解析限制

请求是不可信的，解析器做了防御：

- **深度**：最多 128 层嵌套容器。
- **节点数**：单个载荷最多 262,144 个 msgpack 对象。
- **长度**：声明的 string/array/map 长度超过剩余输入时，在分配前拒绝。
- **Trailing bytes**：载荷必须恰好包含一个 msgpack 对象。

限制常量是 `MAX_PARSE_DEPTH` 与 `MAX_PARSE_NODES`。

## 线格式兼容

`Packager` 产生与 C 实现相同的紧凑 msgpack 字节（包括上面的整数编码），
`Unpackager` 也能解析 C 编码器的输出。外层 envelope 见[协议](protocol.md)。
