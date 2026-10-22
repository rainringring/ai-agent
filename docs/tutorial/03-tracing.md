# 03 · tracing —— 结构化日志与调用追踪

> 版本：`tracing 0.1.44` + `tracing-subscriber 0.3.23`

## 一、它和 `log` 的区别：多了"时间区间"

`log` 只有**事件**（一条日志）。`tracing` 有**事件 + span**：

| 概念 | 含义 | 类比 |
|---|---|---|
| **Event** | 某一刻发生的事 | `println!` 的结构化版本 |
| **Span** | 一段有起止的时间区间，可嵌套 | 一次请求、一个事务 |
| **Subscriber** | 消费事件和 span 的实现 | 决定"打到哪里、长什么样" |

为什么 span 重要？想象一次请求里 10 个函数各打一条日志：

- 用 `log`：10 行互不相干的文本，并发时完全分不清哪行属于哪个请求。
- 用 `tracing`：10 行都自动带上 `request_id=abc123`（因为它在 span 的字段里），一搜就归组了。

**这就是它存在的唯一理由**：把日志从"文本行"变成"带上下文的结构化记录"。

### 三个 crate 的分工

```
tracing              ← 门面：宏、Span、Event（你代码里只接触它）
  └ tracing-core     ← 底层数据结构（一般不用直接引）
  └ tracing-attributes ← #[instrument] 宏（传递依赖）
tracing-subscriber   ← 实现：把事件输出到终端/文件/JSON（程序启动时初始化一次）
  └ tracing-log      ← 把老 log 宏也接到 tracing 上（默认开）
```

⚠️ **关键点：`tracing` 本身什么都不输出。** 只 `use tracing::info;` 然后 `info!("x")`，没有初始化 subscriber 就**一条也看不到**。这是新手最常见的问题。

---

## 二、五个级别 + 字段语法

```rust
use tracing::{trace, debug, info, warn, error};

info!("服务启动");
warn!("重试第 {} 次", n);
error!("连接失败");
```

级别从高到低：`ERROR` > `WARN` > `INFO` > `DEBUG` > `TRACE`。

### 结构化字段

引号里的是**消息**，`=` 前面的是**字段名**。字段会成为机器可读的键值对：

```rust
info!(user_id = 42, action = "login", "用户登录");

// 带格式化参数（推荐这种，字段化）
info!(user_id = %user.id, "用户登录");     // % → 用 Display 格式化
debug!(payload = ?req, "收到请求");        // ? → 用 Debug 格式化
```

| 语法 | 含义 |
|---|---|
| `key = value` | 值必须实现 `Value`（`u64`/`&str`/`bool` 等直接可用） |
| `key = %expr` | 用 `Display` 转成字符串字段 |
| `key = ?expr` | 用 `Debug` 转成字符串字段 |
| `key = tracing::field::display(x)` | 同上，函数式写法 |
| `key = tracing::field::debug(x)` | 同上 |

`%` 和 `?` 的区别很实用：

- 给**人**看的数字/ID → `%`
- 给**开发者**看的复杂结构 → `?`

```rust
info!(status = %404, "请求完成");        // status=404
debug!(headers = ?headers, "详情");      // headers=HeaderMap { ... }
```

---

## 三、Span：给一段代码打上标签

### 方式一：`#[instrument]`（最推荐）

```rust
use tracing::instrument;

#[instrument]
async fn fetch_user(id: u64) -> anyhow::Result<User> {
    // 进入函数自动创建 span，退出自动关闭
    Ok(do_http(id).await?)
}
```

这一行做的事：

1. 创建 span，名字是函数名 `fetch_user`；
2. **自动把每个参数记成字段**（`id=42`）；
3. 函数体内所有日志自动继承这个 span；
4. async 函数也正确（跨 `.await` 不丢失）。

参数里有敏感信息或不可 `Debug` 的类型，必须排除，否则**编译不过**：

```rust
#[instrument(skip(password, client), fields(user = %username))]
async fn login(username: String, password: String, client: &Client) -> anyhow::Result<Token> {
    // ...
}
```

- `skip(password)`：不记录该参数
- `skip_all`：一个都不记
- `fields(...)`：手动指定要记的字段（可以和 skip 配合）

### 方式二：手动 span

```rust
use tracing::{info_span, field};

let span = info_span!("db_query", table = "users", rows = field::Empty);
let _guard = span.enter();      // 进入；离开作用域自动退出
```

异步代码里**不要**这么用（下面第四节解释）。要手动给 future 包 span，用 `Instrument` trait：

```rust
use tracing::Instrument;

let span = tracing::info_span!("outer_job", job_id = 7);
some_future.instrument(span).await;   // 正确：span 跟着 future 走
```

### 事后补字段：`field::Empty`

有时字段值要等计算完才知道：

```rust
use tracing::{info_span, field};

let span = info_span!("request", user_id = field::Empty, elapsed_ms = field::Empty);
let _g = span.enter();

let id = resolve_user().await;
span.record("user_id", id);            // 之后再填
```

---

## 四、异步的坑：别让 guard 跨 `.await`

这段代码**能编译**，但是错的：

```rust
async fn bad() {
    let span = tracing::info_span!("job");
    let _guard = span.enter();     // ❌ guard 活过了 await
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
}
```

为什么错：`Span::enter()` 是基于**线程局部**的。guard 存活期间，这个线程上发生的一切都被归进这个 span。但 `.await` 会让出线程，同一个线程可能去跑**别的任务**——那些任务的日志就被错误地记到你的 span 下了。

正确做法，二选一：

```rust
// 1) #[instrument]（最简单）
#[instrument]
async fn good() { /* ... */ }

// 2) .instrument(span)
use tracing::Instrument;
async fn also_good() {
    let span = tracing::info_span!("job");
    async { /* ... */ }.instrument(span).await;
}
```

**规则：async 函数里永远用 `#[instrument]` 或 `.instrument()`，不要手动 `enter()`。**

---

## 五、初始化 subscriber（⚠️ 本项目必须先改 Cargo.toml）

### 先看这个坑

你现在的 `Cargo.toml` 是：

```toml
tracing-subscriber = "0.3.23"
```

`tracing-subscriber` 的**默认 feature** 只有：

```
default = ["smallvec", "fmt", "ansi", "tracing-log", "std"]
```

**不含 `env-filter`，也不含 `json`。**

所以如果你照着网上的教程写：

```rust
use tracing_subscriber::EnvFilter;   // ❌ 编译错误：no `EnvFilter` in the root
```

会直接失败——不是代码错，是 feature 没开。

### 改法

```toml
tracing-subscriber = { version = "0.3.23", features = ["env-filter"] }
```

需要 JSON 输出再加 `"json"`：

```toml
tracing-subscriber = { version = "0.3.23", features = ["env-filter", "json"] }
```

### 最小可用初始化

```rust
fn init_tracing() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(true)        // 显示日志来自哪个模块
        .with_thread_ids(false)
        .init();                  // 已初始化过会 panic；不想 panic 用 try_init()
}
```

### 推荐模板：`RUST_LOG` 可调

这是实际项目里最常用的形态，**前提是开了 `env-filter` feature**：

```rust
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(true))
        .init();
}
```

为什么要 `registry()`：它把「过滤」和「输出」拆成两层 `Layer` 叠加，方便后面再加文件输出、再加 JSON 层。

### `RUST_LOG` 语法

```powershell
$env:RUST_LOG = "info"                              # 全局 info
$env:RUST_LOG = "ai_agent=debug"                    # 只让自己 crate 调低级别
$env:RUST_LOG = "warn,ai_agent=debug,hyper=off"     # 混合：默认 warn，自己 debug，hyper 静音
$env:RUST_LOG = "ai_agent::http=trace"              # 按模块路径精确到子模块
```

格式：`目标=级别`，逗号分隔，逗号前的裸级别是**默认值**。最后一个匹配的规则生效。

⚠️ 目标名默认是**模块路径**（crate 名用下划线），不是包名。`ai-agent` → `ai_agent`。

---

## 六、输出到文件 / JSON

### 文件（需要 `tracing-appender`，另一个 crate）

```rust
let file_appender = tracing_appender::rolling::daily("./logs", "app.log");
tracing_subscriber::registry()
    .with(fmt::layer().with_writer(file_appender))
    .init();
```

### JSON（需要 `json` feature）

```rust
use tracing_subscriber::{fmt, prelude::*};

tracing_subscriber::registry()
    .with(fmt::layer().json())     // 需要 features = ["json"]
    .init();
```

输出变成一行一个 JSON 对象，字段都在，适合 ELK / Loki 采集：

```json
{"timestamp":"2026-10-18T04:00:00Z","level":"INFO","fields":{"user_id":42,"message":"用户登录"},"target":"ai_agent"}
```

**什么时候要 JSON**：日志要被机器聚合。什么时候不要：只是本地开发看，颜色和缩进更舒服。

---

## 七、和 anyhow 怎么配合（很重要）

`tracing` 和 `anyhow` 的组合有个细节：**`%e` 只打最外层，`?e` 才打完整链**。

```rust
use anyhow::Context;
use tracing::error;

if let Err(e) = do_work().await {
    // ❌ 只看到 "处理订单失败"，看不到根因
    error!("{}", e);

    // ✅ 单行完整链：处理订单失败: 连接超时: timed out
    error!("{:#}", e);

    // ✅ 多行、带 Caused by 结构
    error!("{e:?}");

    // ✅ 推荐：把错误作为结构化字段，消息保持简短
    error!(error = %format!("{e:#}"), "处理订单失败");
}
```

三种的取舍：

| 写法 | 适用 |
|---|---|
| `error!("{:#}", e)` | 终端/纯文本日志，一行看完 |
| `error!("{e:?}")` | 需要看到 `Caused by:` 分层的场合 |
| `error!(error = %format!("{e:#}"), "...")` | JSON 日志：错误进独立字段，方便按错误聚合 |

⚠️ 别用 `?e` 当字段值：`Debug` 输出是多行的，在单行 JSON 或日志采集里会被切断。要结构化就用 `%format!("{e:#}")`。

---

## 八、性能

`tracing` 的设计目标是"没开 subscriber 也几乎零成本"：

- 没有 subscriber 时，宏只做一次静态判断就返回；
- 字段值**惰性求值**：只有该级别启用时才计算 `format!`。

但下面这种写法会绕过惰性：

```rust
// ❌ 无论是否输出，msg 都已经拼好了
tracing::debug!("payload = {}", expensive_to_string(&big));

// ✅ 字段形式：级别未启用时不会格式化
tracing::debug!(payload = ?big, "收到");

// ✅ 或者显式守卫，连字段都不碰
if tracing::enabled!(tracing::Level::DEBUG) {
    tracing::debug!(payload = %expensive_to_string(&big));
}
```

还能在编译期裁掉低级日志（减小体积）：

```toml
tracing = { version = "0.1.44", features = ["release_max_level_info"] }
```

这会让 `debug!`/`trace!` 在 release 里**完全不进二进制**。

---

## 九、坑位清单

| 坑 | 现象 / 原因 |
|---|---|
| **没初始化 subscriber** | 一条日志都没有，`info!` 静默丢弃 |
| **没开 `env-filter`** | `use tracing_subscriber::EnvFilter` 编译失败（⚠️ 本项目当前状态） |
| **没开 `json`** | `fmt::layer().json()` 编译失败 |
| **`enter()` guard 跨 `.await`** | 日志串到别的任务上；用 `#[instrument]` |
| **`#[instrument]` 没 `skip` 不可 Debug 的参数** | 编译失败，报 trait bound 不满足 |
| **`#[instrument]` 记录了密码** | 日志泄露凭据，务必 `skip` |
| **重复 `.init()`** | panic（"already been set"）；测试里用 `try_init()` |
| **测试并行时 subscriber 冲突** | 用 `try_init()` 忽略错误，或每个测试独立 subscriber |
| **级别写反** | `with_max_level(ERROR)` 会把 INFO 也过滤掉 |
| **以为 `RUST_LOG` 生效** | 只有用 `EnvFilter` 时才读它；`with_max_level` 不读 |

---

## 十、完整可用的初始化 + 使用

```rust
use anyhow::{Context, Result};
use tracing::{debug, error, info, instrument, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,ai_agent=debug"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(true))
        .init();
}

#[instrument(skip(api_key), fields(model = "gpt-4o-mini"))]
async fn call_api(api_key: &str, prompt: &str) -> Result<String> {
    debug!(prompt_len = prompt.len(), "发出请求");

    if prompt.is_empty() {
        warn!("空提示词");
        anyhow::bail!("prompt 不能为空");
    }

    let resp = reqwest::get("https://example.com")
        .await
        .context("HTTP 请求失败")?
        .text()
        .await
        .context("读取响应体失败")?;

    info!(bytes = resp.len(), "请求完成");
    Ok(resp)
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    match call_api("sk-xxx", "你好").await {
        Ok(s) => info!(len = s.len(), "成功"),
        Err(e) => {
            // 单行完整错误链
            error!("{:#}", e);
            return Err(e);
        }
    }
    Ok(())
}
```

输出长相：

```
2026-10-18T04:00:00.123Z DEBUG call_api{prompt_len=6}: ai_agent: 发出请求
2026-10-18T04:00:00.456Z  INFO call_api{model=gpt-4o-mini}: ai_agent: 请求完成 bytes=1234
```

注意 span 的名字和字段被自动加在每行前面——**这就是 span 的价值**：不用你手动拼前缀。

**下一步**：去看 [`04-dotenvy.md`](./04-dotenvy.md)，把 `api_key` 从硬编码挪到 `.env`。
