# 02 · serde —— 数据结构与 JSON 互转

> 版本：`serde 1.0.229` + `serde_json 1.0.151`，`Cargo.toml` 已开 `features = ["derive"]`

## 一、先分清"框架"和"格式"

这是新手最容易混淆的一点：

- **`serde`** 是**框架**：它只定义两个 trait（`Serialize` / `Deserialize`）和一套 derive 宏。**它自己不认 JSON**。
- **`serde_json`** 是**格式实现**：把 serde 的数据模型编解码成 JSON 文本。

所以两者是**两层**的关系：

```
你的 struct
    │  #[derive(Serialize, Deserialize)]
    ▼
serde 的数据模型（Data Model）
    │
    ├──► serde_json ──► JSON 文本
    ├──► toml        ──► TOML 文本
    ├──► serde_yaml  ──► YAML 文本
    └──► bincode     ──► 二进制
```

**换上一种格式不用改 struct**，这是 serde 的核心价值。本项目 `Cargo.toml` 里 `serde_json` 就是那个格式实现。

`features = ["derive"]` 是必须的：没有它，`#[derive(Serialize)]` 根本不存在（trait 在，宏不在）。

---

## 二、最小可用

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct User {
    id: u64,
    name: String,
    email: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let u = User { id: 1, name: "小明".into(), email: None };

    // 序列化
    let json = serde_json::to_string(&u)?;
    println!("{json}");                  // {"id":1,"name":"小明","email":null}

    // 反序列化
    let back: User = serde_json::from_str(&json)?;
    assert_eq!(back.id, 1);
    Ok(())
}
```

两点观察：

1. 字段名默认**原样照搬**（`snake_case` 就是 `snake_case`）。
2. `Option::None` 序列化成 `null`，**除非**加了 `skip_serializing_if`。

---

## 三、属性速查表（最常用的 14 个）

属性写在字段或 struct 上，`serde(...)` 里可逗号分隔多个。

### 命名相关

| 属性 | 作用 | 例子 |
|---|---|---|
| `rename = "x"` | 单个字段改名 | `#[serde(rename = "userName")]` |
| `rename_all = "camelCase"` | struct 级别批量改名 | 见下方取值表 |
| `alias = "x"` | 反序列化时额外接受的名字（可多个） | 兼容旧字段名 |
| `rename_all_fields` | 对 enum 的所有 variant 字段生效 | 少用 |

`rename_all` 合法取值：`lowercase`、`UPPERCASE`、`PascalCase`、`camelCase`、`snake_case`、`SCREAMING_SNAKE_CASE`、`kebab-case`、`SCREAMING-KEBAB-CASE`。

### 缺省与可选

| 属性 | 作用 |
|---|---|
| `default` | 字段缺失时用 `Default::default()` |
| `default = "路径"` | 字段缺失时调用指定函数 |
| `skip_serializing_if = "Option::is_none"` | 值是 `None` 时不输出这个键 |
| `skip` | 序列化和反序列化都跳过 |
| `skip_serializing` / `skip_deserializing` | 只跳一边 |

### 结构与校验

| 属性 | 作用 |
|---|---|
| `flatten` | 把内层 struct 的字段平铺到外层（内联） |
| `deny_unknown_fields` | 遇到未知字段直接报错（默认是忽略） |
| `tag = "type"` | enum 用「内部标签」表示 |
| `tag` + `content` | enum 用「相邻标签」表示 |
| `untagged` | enum 不写标签，靠结构猜 |
| `transparent` | newtype 包装器对外透明 |
| `with = "模块"` | 自定义序列化逻辑 |
| `borrow` | 反序列化时借用而非复制（需要生命周期） |

---

## 四、字段缺失的规则（容易被搞错）

规则只有两条，但很重要：

### 1. `Option<T>` 字段缺失 → 自动是 `None`，不用加 `default`

这不是惯例，是 serde 的**特例实现**：反序列化结束时若字段没出现，serde 用一个「缺失字段反序列化器」按 `T` 的类型分派，遇到 `Option` 时走 `visit_none()`。

```rust
#[derive(Deserialize)]
struct A {
    id: u64,
    email: Option<String>,   // JSON 里没有 "email" 也能成功，得到 None
}
```

### 2. 非 `Option` 字段缺失 → 报错，除非 `#[serde(default)]`

```rust
// JSON: {"id": 1}
#[derive(Deserialize)]
struct B {
    id: u64,
    retries: u32,            // ❌ missing field `retries`
}

#[derive(Deserialize)]
struct C {
    id: u64,
    #[serde(default)]
    retries: u32,            // ✅ 缺失 → 0
}
```

用自定义默认值：

```rust
fn default_model() -> String { "gpt-4o-mini".into() }

#[derive(Deserialize)]
struct Req {
    #[serde(default = "default_model")]
    model: String,
}
```

⚠️ **注意 `null` 和「缺失」是两回事**：
- 字段缺失 → 走上面的规则；
- 字段存在但值是 `null` → 只有 `Option<T>` 能接，`u32` 会报 `invalid type: null`。

---

## 五、枚举的四种表示法

这是 serde 最强大也最容易用错的地方。假设：

```rust
enum Shape {
    Circle { r: f64 },
    Rect { w: f64, h: f64 },
}
```

### 1. 外部标签（默认）

variant 名作为唯一的键：

```json
{ "Circle": { "r": 1.0 } }
```

### 2. 内部标签 `#[serde(tag = "type")]`

variant 名塞进一个字段：

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
enum Shape { Circle { r: f64 }, Rect { w: f64, h: f64 } }
```

```json
{ "type": "Circle", "r": 1.0 }
```

⚠️ **限制**：内部标签**不支持 tuple variant**（`Circle(f64)` 这种），因为没地方放字段。只能用于 struct variant 或 unit variant。

**这正是 async-openai 用的方式**。你可以在它源码里看到：

```rust
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ChatCompletionRequestMessage { System(..), User(..), .. }
```

对应 JSON 就是 `{"role":"user", "content":"hi"}` 这种带判别键的形状。

### 3. 相邻标签 `tag` + `content`

内容单独放一个字段，适合 tuple variant：

```rust
#[serde(tag = "t", content = "c")]
```

```json
{ "t": "Circle", "c": 1.0 }
```

### 4. 无标签 `#[serde(untagged)]`

不写任何判别键，靠**字段形状**尝试匹配。顺序很重要，先匹配到哪个算哪个：

```rust
#[serde(untagged)]
enum Id {
    Num(u64),
    Text(String),
}
```

```json
1          → Id::Num
"abc"      → Id::Text
```

⚠️ **坑**：untagged 的错误信息极差——失败时只说"data did not match any variant"，不告诉你是哪个 variant 的哪个字段错了。只在确实需要宽松解析时用。

---

## 六、`serde_json` 常用操作

| 需求 | 函数 |
|---|---|
| struct → 紧凑 JSON 字符串 | `serde_json::to_string(&v)?` |
| struct → 带缩进 JSON | `serde_json::to_string_pretty(&v)?` |
| struct → `Vec<u8>` | `serde_json::to_vec(&v)?` |
| struct → `Value` | `serde_json::to_value(&v)?` |
| JSON 字符串 → struct | `serde_json::from_str::<T>(&s)?` |
| JSON 字节 → struct | `serde_json::from_slice::<T>(&b)?` |
| `Value` → struct | `serde_json::from_value::<T>(v)?` |
| 手写 JSON 字面量 | `serde_json::json!({...})` |
| 读写文件 | `serde_json::from_reader` / `to_writer` |

### `json!` 宏

```rust
use serde_json::json;

let payload = json!({
    "model": "gpt-4o-mini",
    "temperature": 0.7,
    "messages": [
        { "role": "user", "content": "你好" }
    ],
    "stream": false
});
```

注意 `json!` 里**不能直接塞任意 struct**——除非它实现了 `Serialize`（此时会经 `to_value` 走一遍）。也没法在里面写 `if`，条件字段要事后插入。

### `Value`：类型不确定时的逃生舱

```rust
use serde_json::Value;

let v: Value = serde_json::from_str(r#"{"a":{"b":[1,2,3]}}"#)?;

v["a"]["b"][0].as_i64();          // Some(1)
v.get("a").and_then(|x| x.get("b"));
v.pointer("/a/b/1").as_i64();     // Some(2)，JSON Pointer 语法
v["missing"].is_null();           // true（索引不存在返回 Value::Null，不 panic）
```

`Value` 的索引访问**不会 panic**，越界返回 `Null`。这既是优点（安全）也是坑（拼错字段名不报错，你只会拿到 `null`）。

### 保留未知字段原样透传

有时你只需要读几个字段，其余要原样传下去：

```rust
#[derive(Serialize, Deserialize)]
struct Envelope {
    #[serde(flatten)]
    extra: serde_json::Map<String, Value>,   // 吃掉所有未识别的字段
    id: u64,
}
```

⚠️ `flatten` 与 `deny_unknown_fields` **不能共存**，同时写会编译报错或行为异常。

---

## 七、实战：给 OpenAI 请求建模

下面这个例子用到了表里绝大多数属性，对照着看就通了：

```rust
use serde::{Deserialize, Serialize};

/// 发给 API 的请求
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,

    // 不传就不出现在 JSON 里，而不是传 null
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,

    // 服务端可能返回额外字段，宽松解析
    #[serde(default, flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]   // "user" / "assistant" / "system"
enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
enum Message {
    System { content: String },
    User { content: String },
    Assistant { content: Option<String> },
}

/// 从 API 收到的响应：只关心自己需要的字段
#[derive(Debug, Deserialize)]
struct ChatResponse {
    id: String,
    #[serde(default)]
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: RespMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RespMessage {
    #[serde(default)]
    content: Option<String>,   // 工具调用时可能没有正文
}
```

关键设计决策：

- **请求用 `Serialize`，响应用 `Deserialize`**：不必两边都 derive，能减少耦合。需要双向才两个都写。
- **响应字段尽量 `#[serde(default)]`**：API 会加字段、也可能省略字段，宽容解析比严格解析更耐用。
- **`finish_reason` 这类枚举别急着建模成 `enum`**：API 新增一个取值，你的程序就会整个解析失败。先用 `Option<String>`，稳定后再收紧。

---

## 八、属性对比：宽松 vs 严格

| 目标 | 做法 |
|---|---|
| 尽量不因 API 变动崩 | 字段全 `Option` 或 `default`，加 `flatten` 吃未知字段 |
| 尽早暴露契约不符 | `#[serde(deny_unknown_fields)]`，必填字段不加 `default` |
| 兼容旧版本字段名 | `#[serde(alias = "oldName")]` |

没有绝对正确的选择：

- **对外部 API 的响应** → 偏宽松（不然对方一改你就挂）；
- **对自己写的配置文件** → 偏严格（拼错字段名应该立刻报错）。

---

## 九、错误信息怎么读

`serde_json::Error` 带有精确位置：

```rust
match serde_json::from_str::<ChatResponse>(&body) {
    Err(e) => {
        eprintln!("第 {} 行第 {} 列: {e}", e.line(), e.column());
        eprintln!("分类: {:?}", e.classify());  // Io / Syntax / Data / Eof
        return Err(e).context("解析 API 响应失败");
    }
    Ok(v) => { /* ... */ }
}
```

搭配 anyhow 用时，**把原始错误保留在链里**（`.context()` 而不是 `map_err`），这样日志里既有"哪一步"又有"哪个字段"：

```
解析 API 响应失败

Caused by:
    missing field `choices` at line 1 column 87
```

---

## 十、坑位清单

| 坑 | 说明 |
|---|---|
| 忘了 `features = ["derive"]` | `#[derive(Serialize)]` 直接找不到 |
| 以为 `Option` 字段缺失会报错 | 不会，自动 `None`（见第四节） |
| 以为 `u32` 缺失会给 0 | 不会，报 `missing field`，要加 `default` |
| `flatten` + `deny_unknown_fields` | 不兼容 |
| `flatten` 影响 `i128`/`u128` 与枚举解析 | 内部换成 `Content` 缓冲，个别类型会有性能与兼容问题 |
| 内部标签 `tag` 配 tuple variant | 不支持，编译报错 |
| `untagged` 报错信息无用 | 只给 "did not match any variant" |
| 字段名大小写不一致 | JSON 常是 `camelCase`，要 `rename_all = "camelCase"` |
| 整数类型选太窄 | API 的 `created` 是 Unix 时间戳，用 `u32` 到 2106 年才溢出，暂时安全；但 ID 类字段要用 `u64` |
| 直接 `unwrap()` 解析外部输入 | 一行坏数据就 panic，用 `?` + `context` |
| `to_string` 里含非 UTF-8 转义 | JSON 输出始终是合法 UTF-8 文本，不存在这问题；但 `to_vec` 才是字节路径 |

---

## 十一、速查

| 我想…… | 写法 |
|---|---|
| 让字段名变 camelCase | `#[serde(rename_all = "camelCase")]` |
| 反序列化时兼容多个名字 | `#[serde(alias = "old")]` |
| 不输出 `None` 字段 | `#[serde(skip_serializing_if = "Option::is_none")]` |
| 缺失时给默认值 | `#[serde(default)]` |
| 缺失时调函数 | `#[serde(default = "path")]` |
| 枚举带判别键 | `#[serde(tag = "type")]` |
| 内联子结构 | `#[serde(flatten)]` |
| 严格拒绝未知字段 | `#[serde(deny_unknown_fields)]` |
| 手搓 JSON | `json!({ "a": 1 })` |
| 取嵌套值 | `v.pointer("/a/b")` |
| struct ↔ 字符串 | `to_string` / `from_str` |

**下一步**：去看 [`03-tracing.md`](./03-tracing.md)，把 `?` 传上来的错误和调试信息打进日志。
