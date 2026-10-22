# 05 · 相关依赖：tokio / async-openai / thiserror

> 本文把前三篇的东西串起来，并交代两个绕不开的底层依赖。

## 一、tokio —— 异步运行时

`tokio 1.53.1`，本项目开了 `features = ["full"]`（运行时、宏、时间、IO、同步原语全都有）。

### 为什么需要它

Rust 标准库只有 `async` **语法**，没有**运行时**——`async fn` 返回的 future 没人去轮询就永远不会执行。tokio 提供执行器（executor）、反应器（reactor）、定时器。

### 入口

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {   // 注意：可以返回 Result
    Ok(())
}
```

展开后大致是「建运行时 → `block_on(main 的 future)`」。需要自定义线程数：

```rust
#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
```

测试用 `#[tokio::test]`：

```rust
#[tokio::test]
async fn it_works() {
    assert_eq!(2 + 2, 4);
}
```

⚠️ `#[tokio::test]` 默认是**单线程**运行时（`current_thread`）。要测并发行为得写 `#[tokio::test(flavor = "multi_thread")]`。

### 并发：`join!` 与 `select!`

```rust
use tokio::time::{sleep, Duration};

// 全部完成（并发执行，不是顺序）
let (a, b) = tokio::join!(task_a(), task_b());

// 谁先完成用谁，其余取消
tokio::select! {
    v = task_a() => println!("a 先完成: {v:?}"),
    v = task_b() => println!("b 先完成: {v:?}"),
}
```

`join!` 是等待全部；`select!` 是等第一个。两者都在**同一个任务**里并发，不开新线程。

`try_join!` 会在任一失败时提前返回错误。

### 开新任务：`spawn`

```rust
let handle = tokio::spawn(async move {
    // 需要 'static + Send，因为可能在别的线程跑
    42
});
let v = handle.await?;   // JoinHandle 的 await 返回 Result<T, JoinError>
```

⚠️ **`spawn` 的闭包不能借用栈上数据**。要传进去就用 `move` + `Arc`：

```rust
let shared = std::sync::Arc::new(config);
let c = shared.clone();
tokio::spawn(async move { use_config(&c).await });
```

⚠️ **`JoinHandle` 被丢弃不等于任务被取消**：任务会继续跑。想取消要显式 `handle.abort()`。

### 超时

```rust
use tokio::time::{timeout, Duration};

match timeout(Duration::from_secs(30), call_api()).await {
    Ok(Ok(resp)) => println!("成功"),
    Ok(Err(e))   => eprintln!("业务失败: {e}"),
    Err(_)       => eprintln!("超时"),
}
```

两层 `Result` 是常态：外层是"有没有超时"，内层是"业务成功没有"。用 anyhow 时：

```rust
let resp = timeout(Duration::from_secs(30), call_api())
    .await
    .context("请求超时（30s）")?    // 外层：超时
    .context("请求失败")?;          // 内层：业务错误
```

### 通道

```rust
use tokio::sync::mpsc;

let (tx, mut rx) = mpsc::channel::<String>(100);

tokio::spawn(async move {
    tx.send("hello".into()).await.ok();
});

while let Some(msg) = rx.recv().await {
    println!("{msg}");
}
```

| 场景 | 类型 |
|---|---|
| 多生产者单消费者 | `mpsc::channel` |
| 一次性回包 | `oneshot::channel` |
| 广播给多个消费者 | `broadcast::channel` |
| 只保留最新值（配置热更） | `watch::channel` |

### Mutex：选 `std` 还是 `tokio`

这是个高频问题，规则很简单：

| 情况 | 用哪个 |
|---|---|
| 拿到锁后**不跨 `.await`** | `std::sync::Mutex`（更快） |
| 拿到锁后**要 `.await`** | `tokio::sync::Mutex`（必需） |

```rust
// ✅ 锁内没有 await，用 std
{
    let mut g = std_mutex.lock().unwrap();
    g.push(1);
}

// ✅ 锁内要 await，必须用 tokio
{
    let mut g = tokio_mutex.lock().await;
    g.write_all(b"x").await?;
}
```

在 `std::sync::Mutex` 的 guard 上跨 `.await`，编译器**可能不报错**（如果 guard 在 await 前被 drop，或者 future 还是 `Send`），但一旦 `Send` 检查失败会给你一个很难读的报错。记住规则能省很多时间。

### 阻塞操作：`spawn_blocking`

tokio 的工作线程不能被阻塞，否则会卡住整个运行时：

```rust
// ❌ 阻塞工作线程
let s = std::fs::read_to_string("big.txt")?;

// ✅ 丢给阻塞线程池
let s = tokio::task::spawn_blocking(|| std::fs::read_to_string("big.txt"))
    .await??;
```

规则：**任何超过约 10 微秒的同步 CPU 活或阻塞 IO，都进 `spawn_blocking`**。文件读写、大 JSON 解析、图片处理都算。

---

## 二、tracing-subscriber / tracing-core / tracing-log

这三个是 `tracing` 的**实现侧**，一般不用直接 `use`，但知道它们存在能解释一些现象：

| crate | 角色 | 你要知道的事 |
|---|---|---|
| `tracing-subscriber` | 真正的输出实现 | 默认 feature 不含 `env-filter`/`json`，要用得显式开（见 `03-tracing.md`） |
| `tracing-core` | 底层数据结构与 `Subscriber` trait | 只有自己写 subscriber 才会碰 |
| `tracing-log` | 把老 `log` 宏转发到 tracing | 默认开启，所以你依赖的库用 `log::info!` 也能被你的 subscriber 看到 |

最后一条很实用：`reqwest`、`hyper` 这些库内部用的是 `log`，但因为 `tracing-log` 桥接了，`RUST_LOG=hyper=debug` 能直接看到它们的日志。

---

## 三、serde_json 的角色

你已经直接依赖了它，但很多人没意识到它在项目里出现**两处**：

1. **你自己**：`serde_json::to_string` / `from_str`；
2. **`async-openai` 内部**：它所有 API 类型都是 `#[derive(Serialize, Deserialize)]`，请求体由 `serde_json` 编码。

这意味着：**`serde_json` 的版本必须和 async-openai 期望的兼容**。Cargo 会自动用同一个 1.0.x，一般不用管；但如果出现"类型不匹配 `Value`"的诡异错误，第一步看是不是有多个 `serde_json` 版本被解析出来：

```powershell
cargo tree -i serde_json          # 谁依赖了它
cargo tree -d                     # 列出重复版本
```

---

## 四、async-openai 0.41 —— OpenAI 客户端

### 类型与构造

```rust
use async_openai::{Client, config::OpenAIConfig};

// 1) 最简：从环境变量读 OPENAI_API_KEY / OPENAI_BASE_URL
let client = Client::new();                        // 类型是 Client<OpenAIConfig>

// 2) 显式配置
let config = OpenAIConfig::new()
    .with_api_key("sk-xxx")
    .with_api_base("https://api.openai.com/v1");
let client = Client::with_config(config);

// 3) 换 HTTP 客户端（加代理、超时、中间件）
let http = reqwest::Client::builder()
    .timeout(std::time::Duration::from_secs(60))
    .build()?;
let client = Client::new().with_http_client(http);
```

`Client<C>` 是泛型的，`C: Config`。`Client::new()` 特化到 `OpenAIConfig`——所以函数签名里要写 `&Client<OpenAIConfig>`。

⚠️ **`Client::new()` 立刻读环境变量**，所以 `dotenvy::dotenv()` 必须在它之前（见 `04-dotenvy.md`）。

### 发一次 chat completion

0.41 用 `derive_builder` 构造请求。所有请求类型都有对应的 `XxxArgs`：

```rust
use async_openai::types::chat::{
    ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestUserMessageArgs,
    CreateChatCompletionRequestArgs,
};

let req = CreateChatCompletionRequestArgs::default()
    .model("gpt-4o-mini")
    .temperature(0.7)
    .messages(vec![
        ChatCompletionRequestSystemMessageArgs::default()
            .content("你是一个简洁的助手")
            .build()?          // 返回 Result<_, OpenAIError>
            .into(),           // 转成 ChatCompletionRequestMessage
        ChatCompletionRequestUserMessageArgs::default()
            .content("介绍一下 Rust")
            .build()?
            .into(),
    ])
    .build()?;

let resp = client.chat().create(req).await?;

// content 是 Option<String>，且 choices 可能为空 —— 别用 choices[0] 直接索引
let text = resp
    .choices
    .into_iter()
    .next()
    .and_then(|c| c.message.content);
```

关键点（都对着 0.41.3 源码核实过）：

- **`Args` 后缀**是 builder，`build()` 返回 `Result<_, OpenAIError>`，所以函数要能接 `OpenAIError`（或直接 `?` 进 anyhow）。
- **`.content("str")` 能直接接 `&str`**：因为 `impl From<&str> for ChatCompletionRequestUserMessageContent`。
- **`.into()` 是必需的**：`ChatCompletionRequestUserMessage` 不等于 `ChatCompletionRequestMessage`（后者是 enum），存在 `From` 转换。
- **响应的 `content` 是 `Option<String>`**：工具调用、或 `finish_reason = "length"` 时可能没有正文。**别直接 `unwrap()`**。
- **消息枚举用内部标签** `#[serde(tag = "type")]`，正是 `02-serde.md` 讲的第 2 种表示法。

### 流式输出

```rust
use futures_util::StreamExt;   // ⚠️ 需要额外加 futures-util 依赖

let mut stream = client.chat().create_stream(req).await?;
while let Some(chunk) = stream.next().await {
    let chunk = chunk?;
    if let Some(c) = chunk.choices.first() {
        if let Some(text) = &c.delta.content {
            print!("{text}");
        }
    }
}
```

⚠️ **本项目 `Cargo.toml` 里没有 `futures-util`**，`.next()` 会编译失败。要用流式就先加：

```toml
futures-util = "0.3"
```

### 错误类型

`OpenAIError` 是 `thiserror` 定义的 enum：

```rust
pub enum OpenAIError {
    #[error("http error: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("...")]
    InvalidArgument(String),
    // ...
}
```

它实现了 `std::error::Error + Send + Sync + 'static`，所以能直接 `?` 进 `anyhow::Result`。想区分处理就下转型：

```rust
match err.downcast_ref::<OpenAIError>() {
    Some(OpenAIError::InvalidArgument(m)) => eprintln!("参数错: {m}"),
    Some(OpenAIError::Reqwest(e)) if e.is_timeout() => eprintln!("超时，可重试"),
    _ => {}
}
```

---

## 五、thiserror —— 什么时候替代 anyhow

| | anyhow | thiserror |
|---|---|---|
| 类型 | 一个擦除后的 `Error` | 每个错误一个具体类型 |
| 适合 | 应用层 | 库 / 模块边界 |
| 调用方能 `match` | 不能（只能下转型） | 能 |
| 能 `Clone` / `PartialEq` | 不能 | 能 |
| 写起来 | 零样板 | 要定义 enum |

`thiserror` 的写法：

```rust
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("配置缺失: {0}")]
    Config(String),

    #[error("网络请求失败")]
    Network(#[from] reqwest::Error),      // #[from] 自动生成 From 实现

    #[error("API 返回 {status}: {body}")]
    Api { status: u16, body: String },    // 具名字段

    #[error("解析失败: {source}")]
    Parse {
        #[source]                          // 标记为错误来源
        source: serde_json::Error,
        raw: String,                       // 额外上下文
    },
}
```

用法关键点：

- `#[from]`：让 `?` 自动转换（注意：**同一类型只能有一个 `#[from]`**）。
- `#[source]`：标记底层错误，`anyhow` 的链会接上它。
- `#[error("...")]`：`Display` 实现，可以用字段插值。
- `#[error(transparent)]`：完全转发底层错误的 `Display` 与 `source`。

**什么时候两边都要**：库用 thiserror 定义类型，二进制入口统一 `-> anyhow::Result<()>`，中间用 `?`——因为 `thiserror` 的类型实现了 `Error`，转 anyhow 是自动的。

⚠️ 反过来不行：`anyhow::Error` → 你的 enum 没有 `From`，一旦库内部用了 anyhow 就没法回头。

---

## 六、完整示例：全部串起来

`src/main.rs`。依赖见下方 `Cargo.toml`。

```rust
use anyhow::{bail, Context, Result};
use async_openai::{
    config::OpenAIConfig,
    types::chat::{
        ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
        CreateChatCompletionRequestArgs,
    },
    Client,
};
use tracing::{error, info, instrument};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// 日志：RUST_LOG 可调，默认自己 crate 开 debug
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,ai_agent=debug"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(true))
        .init();
}

/// 一次问答。skip(client) 因为 Client 不实现 Debug
#[instrument(skip(client), fields(model = "gpt-4o-mini"))]
async fn ask(client: &Client<OpenAIConfig>, system: &str, question: &str) -> Result<String> {
    let req = CreateChatCompletionRequestArgs::default()
        .model("gpt-4o-mini")
        .messages(vec![
            ChatCompletionRequestSystemMessageArgs::default()
                .content(system)
                .build()?
                .into(),
            ChatCompletionRequestUserMessageArgs::default()
                .content(question)
                .build()?
                .into(),
        ])
        .build()
        .context("构造 chat 请求失败")?;

    let resp = client
        .chat()
        .create(req)
        .await
        .context("调用 chat completions 失败")?;

    resp.choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .context("响应中没有文本内容（可能是工具调用或被截断）")
}

#[tokio::main]
async fn main() -> Result<()> {
    // 1) 环境变量：必须最早
    dotenvy::dotenv().ok();

    // 2) 日志：随后
    init_tracing();

    // 3) 校验配置：早点失败，别等到第一次请求
    if std::env::var("OPENAI_API_KEY").is_err() {
        bail!("OPENAI_API_KEY 未设置：在 .env 里填一行 OPENAI_API_KEY=sk-xxx");
    }

    // 4) 客户端
    let client = Client::new();
    info!("客户端已就绪");

    // 5) 干活，错误统一在这里兜底
    match ask(&client, "你是一个简洁的助手", "用一句话介绍 Rust").await {
        Ok(answer) => {
            info!(answer = %answer, "回答成功");
            println!("{answer}");
        }
        Err(e) => {
            // 单行完整链进日志，然后把错误交给 main 的 Debug 输出
            error!("{:#}", e);
            return Err(e);
        }
    }

    Ok(())
}
```

### 配套的 `Cargo.toml`

```toml
[package]
name = "ai-agent"
version = "0.1.0"
edition = "2024"

[dependencies]
anyhow = "1.0.102"
async-openai = "0.41.0"
dotenvy = "0.15.7"
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
tokio = { version = "1.52.3", features = ["full"] }
tracing = "0.1.44"

# ⚠️ 关键：默认 feature 不含 env-filter
tracing-subscriber = { version = "0.3.23", features = ["env-filter"] }
```

再加 `.env`：

```dotenv
OPENAI_API_KEY=sk-xxx
OPENAI_BASE_URL=https://api.openai.com/v1
RUST_LOG=info,ai_agent=debug
```

---

## 七、这个错误链为什么这样设计

回头看整个示例，四种工具各管一段，**边界很清楚**：

```
dotenvy    ── 把外部输入（文件）变成进程环境
   ↓
tracing    ── 记录每一段的进退、耗时、失败
   ↓
async-openai + serde ── 把 struct 变成 HTTP 请求、把 JSON 变回 struct
   ↓
anyhow     ── 给每一步加上"人话上下文"，一路抛到 main 统一输出
```

如果 `ask()` 失败，用户看到的日志是：

```
ERROR ai_agent: 调用 chat completions 失败: http error: error sending request for url (...)

Caused by:
    0: http error: error sending request for url (...)
    1: client error (Connect)
    2: tcp connect error
```

四层信息，每一层由不同的工具提供。这就是"用对工具"的样子——不是每个 crate 单打独斗，而是错误从产生到展示全程不丢信息。

---

## 八、排错清单

### `cargo check` 报 `failed to parse manifest`

```text
Caused by: dependency (serde) specified without providing a local path, Git repository, version...
```

→ `Cargo.toml` 里有**拼写错误的键**。本项目曾把 `version` 写成 `versiong`。逐行核对依赖表。

### `use tracing_subscriber::EnvFilter;` 找不到

```text
no `EnvFilter` in the root
```

→ 没开 `env-filter` feature。改成 `features = ["env-filter"]`。

### 拉依赖时报 TLS 错误

```text
[35] SSL connect error (schannel: AcquireCredentialsHandle failed: SEC_E_NO_CREDENTIALS)
```

→ **不是代码问题**。本机直连 `crates.io` 的 TLS 会被中断（TCP 通、明文 HTTP 通、只有 HTTPS 失败）。绕过办法：

```powershell
cargo check --offline     # 用本地 registry 缓存
```

前提是缓存里有需要的 crate，且 `Cargo.lock` 已存在。想长期解决就配镜像源（项目内 `.cargo/config.toml`）：

```toml
[source.crates-io]
replace-with = "rsproxy"

[source.rsproxy]
registry = "sparse+https://rsproxy.cn/index/"
```

### 日志一条都不输出

检查三件事，按顺序：
1. 有没有调 subscriber 的 `.init()`？
2. 级别够不够（`with_max_level` 或 `RUST_LOG`）？
3. `RUST_LOG` 的 target 是不是写错了（是 `ai_agent` 不是 `ai-agent`）？

### `Client::new()` 说缺 `OPENAI_API_KEY`

→ `dotenvy::dotenv()` 在它之后，或者 `.env` 不在查找路径上。打印 `dotenvy::dotenv()` 的返回值确认实际加载了哪个文件。

### `.next()` 找不到方法

→ 缺 `futures-util` 依赖（本项目当前没有）。

```toml
futures-util = "0.3"
```

---

## 回到起点

这五个文件回答的是同一个问题的不同侧面：

- **anyhow**：错误怎么传出去而不丢信息
- **serde**：数据怎么进出
- **tracing**：程序在干什么、慢在哪
- **dotenvy**：配置怎么进来
- **tokio / async-openai**：异步执行与外部服务

任何一条报错，先判断它属于哪一层，再去对应的文件里找。这比记住 API 有用得多。
