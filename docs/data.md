# 数据模型：Packager / Unpackager / YarData

> 本文示例在 `tests/doc_examples.rs` 中有编译校验：
> `doc_pack_nested_structure`、`doc_yar_data_accessors`、`doc_unpack_iterator`。

Yar 用 **msgpack** 作为打包协议。本文介绍三套工具：

| 工具 | 作用 | C 中的对应物 |
|---|---|---|
| `Packager` | 增量式打包，逐步生成 msgpack 字节流 | `yar_packager` |
| `Unpackager` | 把 msgpack 字节解析成一棵 `YarData` 树 | `yar_unpack` |
| `YarData` | 解析后的数据树（代数类型） | `msgpack_object` / `yar_data` |

## 支持的类型

`YarDataType`（与 C 枚举值一致）：

| Rust | 值 | 对应 msgpack 格式 | Rust 中如何产生/读取 |
|---|---|---|---|
| `Null` | 1 | nil | `push_null()` / `as_bool()` 无对应即可忽略 |
| `Bool` | 2 | true/false | `push_bool(bool)` → `as_bool() -> Option<bool>` |
| `Long` | 3 | 负数/大数有符号整数 | `push_long(i64)`（负数、超出 fixint 的值）→ `as_long() -> Option<i64>` |
| `Ulong` | 4 | 非负紧凑整数 | `push_ulong(u64)`，以及 `push_long` 的小正数（见下）→ `as_ulong() -> Option<u64>` |
| `Double` | 5 | float64（恒定 9 字节） | `push_double(f64)` → `as_double() -> Option<f64>` |
| `String` | 6 | str（允许非 UTF-8 字节） | `push_string(&[u8])` → `as_bytes() -> Option<&[u8]>` / `as_str() -> Option<&str>` |
| `Map` | 7 | map | `start_map(n)` / `push_map(n)` → `as_map() -> Option<&[(YarData, YarData)]>` |
| `Array` | 8 | array | `start_array(n)` / `push_array(n)` → `as_array() -> Option<&[YarData]>` |

## ⚠️ 整数解码陷阱：优先使用 `as_integer()`

msgpack 使用"紧凑编码"：小正数直接落在无符号格式区。因此：

- `push_long(42)` 打包后解包得到 **`Ulong(42)`**（segment 不是 `Long`！）；
- `push_long(-42)` 解包得到 `Long(-42)`；
- `push_ulong(42)` 解包得到 `Ulong(42)`；
- 写法 `as_long()` 对 `Ulong` 返回 `None` —— 这是初学者最常见的坑。

**结论：读取任意整数时使用 `as_integer() -> Option<i64>`（Long 与 Ulong 通用）**。

```rust
let mut pk = Packager::start_array(4).unwrap();
pk.push_long(42).unwrap();
pk.push_long(-42).unwrap();
pk.push_string(b"hello").unwrap();
pk.push_array(0).unwrap();

let data: YarData = Unpackager::new(pk.as_bytes()).unwrap().into_data();
let arr = data.as_array().unwrap();
assert_eq!(arr.len(), 4);

// 小正数解包成 Ulong —— 用 as_integer()（或 as_ulong()）读取
assert_eq!(arr[0].as_ulong(), Some(42));
assert_eq!(arr[0].as_integer(), Some(42));
assert_eq!(arr[0].as_long(), None); // 注意：不是 Long！

// 负数解包成 Long
assert_eq!(arr[1].as_integer(), Some(-42));

// 字符串与空数组
assert_eq!(arr[2].as_str(), Some("hello"));
assert_eq!(arr[3].as_array().unwrap().len(), 0);
```

## Packager：增量式打包

### 两种容器写法

| API | 含义 |
|---|---|
| `Packager::start_array(n)` / `start_map(n)` | 新建 Packager 同时写入容器头（最常用请求参数、返回值） |
| `pk.push_array(n)` / `pk.push_map(n)` | 在**当前**序列里写一个容器头，后续 push 的值填入该容器（用于嵌套结构） |

### 一维打包顺序（与 C 手册一致）

容器是**扁平顺序**打包的：先写容器头，再按写入顺序填内容；嵌套容器的头会"打开"一个
新的子区域，填满自动回到上层。构造嵌套结构时必须**恰好写满声明的元素个数**，
否则整条 msgpack 流不完整、对端无法解析。

经典示例：构造 `{ a => [b, c], d => e }`：

```rust
// 目标结构：{ a => [b, c], d => e }
let mut pk = Packager::start_map(2).unwrap();

pk.push_string(b"a").unwrap(); // 第一个 key: a
pk.push_array(2).unwrap();     // 第一个 key 对应的 2 元素数组
pk.push_string(b"b").unwrap(); // 第一个元素
pk.push_string(b"c").unwrap(); // 第二个元素（数组填充完毕）

pk.push_string(b"d").unwrap(); // 第二个 key: d
pk.push_string(b"e").unwrap(); // 第二个 key 对应的值 e

// 解回来验证
let data = Unpackager::new(pk.as_bytes()).unwrap().into_data();
let map = data.as_map().unwrap();
assert_eq!(map.len(), 2);
let (k1, v1) = &map[1];
assert_eq!(k1.as_str(), Some("d"));
assert_eq!(v1.as_str(), Some("e"));
```

### 其他 API

| API | 说明 |
|---|---|
| `push_null()` / `push_bool(v)` / `push_long(v)` / `push_ulong(v)` / `push_double(v)` / `push_string(bytes)` | 写入叶子值 |
| `push_data(&YarData)` | 把解析出来的数据树重新打包（服务器转发/回显场景） |
| `push_packager(&Packager)` | 把另一个 Packager 的**原始 msgpack 字节**直接拼到当前流末尾（请求参数数组就是这么组装的） |
| `as_bytes() -> &[u8]` / `into_bytes() -> Vec<u8>` | 取出最终 msgpack 字节 |

### 注意

- `Packager::new()` 建立的是"无类型头"的空包，内容必须是你接下来 push 的第一段完整值；
  但**读取时 `Unpackager` 只解析从头开始的第一个 msgpack 对象**——
  如果你 push 了多个"顶层"值，后面的将读不到。请明确 start_array/先写容器头。
- 客户端 `call()` 传的每个 `Packager` 都必须包含一个完整 msgpack 值。

## Unpackager 与 YarData

```text
let data: YarData = Unpackager::new(some_msgpack_bytes)?.into_data();

// 按需读取（Option，类型不匹配为 None）
data.as_bool();  data.as_integer(); data.as_ulong();
data.as_double(); data.as_bytes(); data.as_str();
data.as_array(); data.as_map(); data.as_long();
data.data_type();                      // YarDataType
let (t, size) = data.data_type_and_size(); // 字节串：长度；数组：元素数；map：条目数
```

- `String` 以 `Vec<u8>` 原始字节保存。`as_str()` 内部做 UTF-8 校验，
  非 UTF-8 内容时返回 `None`（此时用 `as_bytes()` 处理字节）。
- `Unpackager::new(bytes)` 只解析**一个** msgpack 值；多余的尾部字节会被忽略。

## 迭代器：`UnpackIterator`

从数组或 map 上打开迭代器，行为与 C 的 `yar_unpack_iterator` 一致：

- `data.iter()` 打开迭代器；非容器类型返回 `None`；
- 数组：产出所有元素；
- **map：按 `key0, value0, key1, value1, ...` 交错产出**（总个数 = 条目数 × 2）；
- `it.next()` 前进到下一项并返回**新位置**；迭代耗尽后返回 **0**（注意不是 Option！）；
- `it.current()` 取当前项；`it.size()` 取总个数；`it.reset()` 回到开头。

```rust
let mut pk = Packager::start_array(2).unwrap();
pk.push_long(1).unwrap();
pk.push_long(2).unwrap();

let data: YarData = Unpackager::new(pk.as_bytes()).unwrap().into_data();
let mut it = data.iter().expect("不是容器类型时 iter() 返回 None");

// map 类型时 iterator 会按 key0, value0, key1, value1 ... 交错产出
let mut sum = 0;
loop {
    sum += it.current().as_integer().unwrap_or(0);
    if it.next() == 0 {
        break; // next() 返回 0 表示迭代结束（与 C 一致）
    }
}
assert_eq!(sum, 3);
it.reset(); // 回到开头
assert_eq!(it.position(), 0);
```

## 速查：常用读参写返值代码模式

```rust
// 服务器 handler 里的典型读写
let arr = req.get_parameters().and_then(|p| p.as_array()).unwrap_or(&[]);
let a = arr.first().and_then(|v| v.as_integer()).unwrap_or(0);
let s = arr.get(1).and_then(|v| v.as_str()).unwrap_or("default");

let mut pk = Packager::new();
pk.push_string(b"result").unwrap();
YarResponse::success(req.id(), pk)
```
