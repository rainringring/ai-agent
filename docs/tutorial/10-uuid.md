# 10 · uuid —— 生成与使用 128 位标识符

> 版本：`uuid 1.28.0`，本项目 `features = ["v4"]`（默认 feature 是 `std`，未关）
> 环境：`rustc 1.97.1`、`edition 2024`
> 方法名逐个核对自本地 registry 的 `uuid-1.28.0` 源码。

---

## 一、UUID 到底是什么

一个 **128 位（16 字节）** 的标识符，标准写法是 36 个字符的十六进制：

```
550e8400-e29b-41d4-a716-446655440000
└──8──┘ └4┘ └4┘ └4┘ └────12────┘      共 32 个 hex 字符 + 4 个连字符
```

三个属性决定了它好用：

| 属性 | 含义 |
|---|---|
| 足够大 | 2^128 ≈ 3.4×10^38 个可能值 |
| 不需要协调 | 任何进程、任何机器各自生成，碰撞概率可忽略（v4 有 122 位随机熵） |
| 可传输 | 纯文本形式，放进 JSON / URL / 日志都无损 |

现行标准是 **RFC 9562**（取代了老的 RFC 4122），版本号的语义由它定义。

⚠️ 常见误解：**UUID 不是“密码/令牌”**。它的随机性用于“唯一”，不是“不可猜测的凭证”——要令牌请用专门的方案（`rand` + base64、`secrecy` 等）。

---

## 二、版本家族：什么时候用哪个

| 版本 | 生成方式 | 确定性 | 时间有序 | 典型用途 |
|---|---|---|---|---|
| **v1** | 时间戳 + 节点（MAC） | ❌ | 部分 | 老系统；**泄露 MAC 有隐私问题**，别用 |
| v3 | 命名空间 + name（MD5） | ✅ 同输入同输出 | ❌ | 需要“可复算”的稳定 id |
| **v4** | 纯随机 | ❌ | ❌ | **默认选择**：session、trace、幂等键 |
| v5 | 命名空间 + name（SHA1） | ✅ | ❌ | 同 v3，但用 SHA1（更推荐） |
| v6 | 时间戳重排（可排序） | ❌ | ✅ | v1 的“可排序”修正版 |
| **v7** | Unix 毫秒时间戳 + 随机 | ❌ | ✅ | **数据库主键**（B-tree 友好） |
| v8 | 自定义位布局 | 视实现 | 视实现 | 实验/特殊场景 |

**最重要的取舍**：v4 是随机的 → 当数据库主键时插入位置随机，索引页反复分裂；v7 把时间放高位 → 新行基本追加在末尾，写放大和缓存命中都更好。**主键用 v7，其它用 v4。**

---

## 三、本项目当前的 feature 状态（⚠️ 直接影响你能写哪些代码）

`uuid` 的 feature 是“按版本拆”的，没开的版本对应的方法**根本不存在**。

| feature | 提供 | 本项目 |
|---|---|---|
| `v4` | `Uuid::new_v4()` | ✅ 已开 |
| `v7` | `Uuid::now_v7()` / `Uuid::new_v7(ts)` / `Timestamp` | ❌ 未开 |
| `v1` / `v6` | `new_v1` / `now_v6` | ❌ |
| `v3` / `v5` | `new_v3` / `new_v5`（命名空间式） | ❌ |
| `serde` | `Uuid` 的 `Serialize`/`Deserialize` | ❌ **未开** |
| `std`（默认） | 字符串解析/显示、`Error` 实现 | ✅ |
| `fast-rng` | 用 `rand::SmallRng` 替代 `getrandom` | ❌（一般不需要） |
| `js` | WASM 下的随机源 | ❌（非 WASM 不用） |

所以现在能做的：

```rust
let id = uuid::Uuid::new_v4();          // ✅
let s: &str = ".."; uuid::Uuid::parse_str(s);   // ✅（std 自带）
```

现在**不能**做的，以及要加什么：

```toml
# 想在 JSON 里直接读写 Uuid（比如放进 MCP 工具入参/返回、serde_json）
uuid = { version = "1.28.0", features = ["v4", "serde"] }

# 想用时间有序主键
uuid = { version = "1.28.0", features = ["v4", "v7"] }
```

---

## 四、常用方法（逐个核对过的签名）

### 4.1 构造

```rust
use uuid::Uuid;

Uuid::new_v4()          // 随机（需要 v4）      
Uuid::now_v7()          // 当前时间 + 随机（需要 v7）
Uuid::nil()             // 全 0：00000000-0000-0000-0000-000000000000
Uuid::max()             // 全 f
Uuid::from_u128(x)      // 从 u128
Uuid::from_bytes([u8; 16])
uuid::uuid!("550e8400-e29b-41d4-a716-446655440000")   // 编译期字面量，格式错误直接编译失败
```

### 4.2 解析与判断

```rust
Uuid::parse_str("550e8400e29b41d4a716446655440000")?   // 接受无连字符形式
Uuid::try_parse(s)?                                    // const fn 版本，可用于 const 上下文
u.is_nil()                                             // 是否是全 0
```

`parse_str` 接受四种形态：hyphenated（标准）、simple（无连字符）、braced（`{...}`）、urn（`urn:uuid:...`）——**不要自己写正则校验**。

### 4.3 输出格式

```rust
u.to_string()          // Display：小写 + 连字符（最常用）
u.hyphenated()         // 可继续 .upper() / .lower() / .encode_lower(&mut buf)
u.simple()             // 无连字符
u.braced()             // {..}
u.urn()                // urn:uuid:..
```

⚠️ 大小写、有无连字符**不影响相等性**（比较的是 128 位值），但**影响字符串比较**。要比较/排序，请用 `Uuid` 类型本身，不要比字符串。

### 4.4 取内部值

```rust
u.as_u128()      // u128
u.as_bytes()     // &[u8; 16]
u.get_version()  // Option<Version>，v4 时是 Some(V4)
u.get_version_num()  // usize，4
```

### 4.5 排序

`Uuid` 实现了 `Ord`，按**字节序**比较。这是 v7 作主键的前提：v7 的高位是毫秒时间戳，所以“字节序 ≈ 时间序 ≈ 插入序”。

---

## 五、serde：为什么你现在不能把 Uuid 放进 JSON

本项目没开 `serde` feature，所以：

```rust
#[derive(Serialize)]
struct Foo { id: Uuid }     // ❌ 编译失败：
                            // the trait `Serialize` is not implemented for `Uuid`
```

开了 `serde` 之后的行为：

| 目标格式 | 序列化成 |
|---|---|
| 人类可读（JSON/YAML/TOML） | 字符串 `"550e8400-e29b-41d4-a716-446655440000"` |
| 二进制（bincode/postcard） | 16 字节 |

`serde` feature 不改变 API，只是补了两个 trait 实现——**但它会改变 Cargo.lock 的 feature 集合，需要重新解析一遍**。

---

## 六、常见坑（8 条）

| # | 坑 | 说明 / 对策 |
|---|---|---|
| 1 | `new_v4` 不存在 | 没开 `v4` feature。本项目已开，够用 |
| 2 | `Serialize` 报错 | 没开 `serde` feature（见上节） |
| 3 | v4 当数据库主键 | 随机插入 → 索引页分裂、写放大。**主键用 v7** |
| 4 | 自己写正则校验 UUID | `parse_str` 已经覆盖四种形态；正则还会漏掉/错杀 |
| 5 | 用 `nil()` 表示“没有值” | 全 0 是个**合法 id**，容易和“缺失”混淆。用 `Option<Uuid>` |
| 6 | 拿 UUID 当安全令牌 | 122 位随机够“不可碰撞”，但**不是**为此设计的；令牌用 `rand` + 编码 + 过期/签名 |
| 7 | 比较字符串 | 大小写/连字符差异会导致误判；比较 `Uuid` 值 |
| 8 | 存储用 `CHAR(36)` | 16 字节二进制（`BINARY(16)`/`BYTEA`）省一半空间；字符串只用于展示和传输 |

另外两个小点：

- `fast-rng` 用 `rand::SmallRng`，吞吐高但**不是密码学安全**的随机源；普通实体 id 无所谓，安全场景别用。
- `uuid!` 宏在编译期校验格式，比运行时 `parse_str` 更早暴露拼写错误。

---

## 七、放进本项目的三个真实用途

### 7.1 把一轮 agent 的日志串起来（trace id）

```rust
let trace_id = uuid::Uuid::new_v4();
let span = tracing::info_span!("agent_run", %trace_id);
let _g = span.enter();
// 之后所有 tracing 输出都会带上 trace_id，一次 grep 就能捞出整轮调用
```

这是 uuid + tracing 最划算的组合：跨函数、跨请求都能对上。

### 7.2 工具调用的幂等键

```rust
// 同一个 tool_call 重试时用同一个 key，服务端可据此去重
let idempotency_key = uuid::Uuid::new_v4();
```

注意：**OpenAI 的 `tool_call.id` 不是 UUID**（形如 `call_abc123`，服务端生成），别混用；重试时**原样复用**你第一次生成的 key，而不是每次新生成。

### 7.3 实体 id（expense / session）

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Expense {
    pub id: uuid::Uuid,     // ⚠️ 需要 serde feature
    pub amount_cents: i64,
}
```

如果要当数据库主键，建议 `features = ["v7"]` 用 `Uuid::now_v7()`。

---

## 八、速查

```rust
// 生成
let id = uuid::Uuid::new_v4();                 // 随机
let id = uuid::Uuid::now_v7();                 // 时间有序（需 v7）
let id = uuid::uuid!("550e8400-e29b-41d4-a716-446655440000");  // 编译期

// 解析
let u = uuid::Uuid::parse_str(s)?;

// 输出
u.to_string();          // 标准形式
u.simple().to_string(); // 无连字符
u.as_u128(); u.as_bytes();

// 判断
u.is_nil(); u.get_version(); u.get_version_num();

// Cargo.toml
uuid = { version = "1.28.0", features = ["v4", "serde"] }   // + "v7" 如需主键
```

**一句话**：uuid 提供的是「不需要协调、全局唯一、可文本传输」的 128 位 id——**默认 v4（随机）**，要当数据库主键就换 **v7（时间有序）**；JSON 里要读写它，必须显式打开 `serde` feature，这一点本项目目前还没开。

---

## 与本系列其它文档的关系

- 与 [`02-serde.md`](./02-serde.md)：`Uuid` 默认不实现 `Serialize`，是那个“trait 没实现”错误的典型例子；
- 与 [`09-rmcp-and-chrono.md`](./09-rmcp-and-chrono.md)：MCP 工具的入参/返回值要进 JSON Schema，`Uuid` 想作为参数类型需要额外 feature，通常直接用 `String` 更省事；
- 与 [`03-tracing.md`](./03-tracing.md)：`trace_id` 放进 span 字段是这套 id 最常见的用法。
