# 09 · rmcp 与 chrono —— 把工具暴露成 MCP 服务、以及处理时间

> 版本：`rmcp 3.6.0`（本机 feature：`client` `server` `macros` `transport-child-process` `transport-io`）、`rmcp-macros 3.6.0`、`schemars 1.2.3`、`chrono 0.4.45`
> 环境：`rustc 1.97.1`、`edition 2024`；**rmcp 自己的 MSRV 是 1.88**
> 源码核对自本地 registry（`rmcp-3.6.0`、`rmcp-macros-3.6.0`、`chrono-0.4.45`）。

---

## 一、两个 crate 的定位

| crate | 一句话 | 你现在的情况 |
|---|---|---|
| **rmcp** | MCP（Model Context Protocol）的官方 Rust SDK：把你的工具/资源/提示词按 MCP 协议暴露出去，或作为客户端去连别的 MCP server | 已在 `Cargo.toml`，**还没在 `src/` 里用过** |
| **chrono** | 日期时间类型库：带时区的时间点、日历计算、格式化/解析 | 已是直接依赖（`0.4.45`，默认 feature 含 `clock`）；`serde` feature 目前靠 `rmcp` 合并带来（见第九节） |

两者在这个项目里会碰头的地方很具体：**MCP 工具的入参可以用 `chrono::DateTime<Utc>`**，因为 `rmcp` 给 `schemars` 开了 `chrono04` feature，`DateTime` 自带 JSON Schema。

---

## 二、MCP 的三层结构

```
Host（宿主，比如 Claude Desktop / 你的 agent 程序）
 └── Client（协议客户端，rmcp 的 RoleClient）
      ⇅  传输：stdio（子进程管道） 或 Streamable HTTP
     Server（协议服务端，rmcp 的 RoleServer）
      ├── tools       可被调用的函数（有 JSON Schema 入参）
      ├── resources   可读取的数据（文件、记录……）
      ├── prompts     可复用的提示词模板
      └── 其它：sampling / roots / logging / completion
```

和上一份 [`08-async-trait.md`](./08-async-trait.md) 的关系：你的 `Tool` trait 是**进程内**抽象；MCP 是**跨进程**协议。两者可以并存（见第八节）。

---

## 三、feature 对照（为什么你开了这 5 个）

`rmcp` 的默认 feature 是 `base64` + `macros` + `server`，而 `server` 又会打开 `schemars` 和 `transport-async-rw`。

| 你写的 feature | 提供什么 | 缺了会怎样 |
|---|---|---|
| `server` | `ServerHandler`、工具路由、`schemars` | 写不了服务端 |
| `client` | `ClientHandler`、`ClientServiceExt` | 连不了别人的 server |
| `macros` | `#[tool]` / `#[tool_router]` / `#[tool_handler]`（来自 `rmcp-macros`） | 只能手写 `Tool` 结构体和路由 |
| `transport-io` | 服务端 stdio 传输 `transport::stdio()` | 服务端没法和宿主通信 |
| `transport-child-process` | 客户端 stdio 传输 `TokioChildProcess` | 客户端无法拉起子进程 server |

⚠️ 注意方向：`transport-io` 是**服务端**用的（从 stdin 读、往 stdout 写），`transport-child-process` 是**客户端**用的（spawn 一个子进程当 server）。两个都开才叫“既能当 server 也能当 client”。

没开的（按需再加）：`auth`（OAuth）、`transport-streamable-http-server` / `-client`（HTTP 传输）、`elicitation`。

---

## 四、最小 MCP server：四段式

标准写法就是四块，缺一不可：

```rust
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router,
    ServiceExt,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ① 入参类型：必须 Deserialize + JsonSchema
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddArgs {
    pub a: f64,
    pub b: f64,
}

// ② 工具函数放在 impl 块里，用 #[tool] 标记
#[derive(Clone)]
pub struct CalcServer {
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl CalcServer {
    fn new() -> Self {
        Self { tool_router: Self::tool_router() }
    }

    /// Add two numbers.                ← 文档注释会成为工具的 description
    #[tool]
    async fn add(&self, Parameters(args): Parameters<AddArgs>) -> Result<String, String> {
        Ok((args.a + args.b).to_string())
    }
}

// ③ 让 ServerHandler 知道你有哪些工具
#[tool_handler]
impl ServerHandler for CalcServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("Call `add` to sum two numbers.")
    }
}

// ④ 跑起来：stdio 传输 + 等它结束
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = CalcServer::new().serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
```

要点：

| 点 | 说明 |
|---|---|
| `ToolRouter<Self>` 字段 | `#[tool_router]` 生成的 `Self::tool_router()` 汇总了本 impl 里所有 `#[tool]`；`#[tool_handler]` 默认调它，所以你也可以不存字段、直接让宏去调 |
| `#[tool]` 的 `name` | 不写就用函数名；写了要保证全局唯一（客户端按名字调用） |
| `description` | 不写就取**文档注释**（所以 `/// Add two numbers.` 那行是有用的，不是装饰） |
| `Parameters<T>` | 解包器：框架会把 tool_call 的 `arguments` 反序列化成 `T`；`args.0` 拿内层值 |
| 返回类型 | 见下一节，支持多种 |
| `ServerConfig` | 就是 `InitializeResult` 的别名（协议 `initialize` 的响应）；`ServerInfo` 已被 **deprecated**，改用 `ServerConfig` |

### 工具返回值的四种形态

| 返回类型 | 结果 | 什么时候用 |
|---|---|---|
| `()` | 只有 `isError: false`，无内容 | 副作用型工具 |
| `String` | 一段纯文本 | 最简单、最通用 |
| `Result<T, E>`（`T: IntoCallToolResult`，`E: Into<ErrorData>`） | 成功/失败自动转成 `isError` | **推荐**：失败能让模型看见 |
| `rmcp::handler::server::wrapper::Json<T>` | 写进 `structuredContent`（带 schema） | 想让客户端拿结构化数据时 |

`CallToolResult` 还有便捷构造：`CallToolResult::structured(value)`、以及设置 `is_error = Some(true)`。

### 想改服务端名字/版本？

覆写 `get_info`，用 `Implementation` 改身份（`ServerConfig` 是 `InitializeResult`，里面有 `server_info` 字段）：

```rust
fn get_info(&self) -> ServerConfig {
    ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
        .with_instructions("...")
        // .with_server_info(Implementation::new("my-calc", "1.2.3"))
}
```

---

## 五、最小 MCP client：连上别人的 server

客户端要自己“拉起”一个 server 进程时，用 `TokioChildProcess`：

```rust
use rmcp::{model::CallToolRequestParams, transport::TokioChildProcess, ServiceExt};
use tokio::process::Command;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ① 拉起子进程当 server（stdio 管道）
    let transport = TokioChildProcess::new(
        Command::new("npx").args(["-y", "@modelcontextprotocol/server-filesystem", "."]),
    )?;

    // ② () 是最简单的 ClientHandler 实现
    let client = ().serve(transport).await?;

    // ③ 发现工具
    let tools = client.list_tools(None).await?;
    for t in &tools.tools {
        println!("- {}: {:?}", t.name, t.description);
    }

    // ④ 调用工具（arguments 是 JsonObject）
    let result = client
        .call_tool(
            CallToolRequestParams::new("add")
                .with_arguments(serde_json::json!({"a": 1, "b": 2}).as_object().cloned().unwrap()),
        )
        .await?;
    println!("{:?}", result);

    client.cancel().await?;      // 或直接 drop
    Ok(())
}
```

| 步骤 | API | 说明 |
|---|---|---|
| 建传输 | `TokioChildProcess::new(cmd)` | 也可以 `.builder(cmd)` 精细控制 stdio；返回 `io::Result` |
| 建服务 | `().serve(transport).await?` | `()` 已实现 `ClientHandler`；要缓存/能力协商就换成自己的 `ClientConfig` |
| 发现 | `client.list_tools(None)` | 参数是分页 cursor（`None` = 第一页） |
| 调用 | `client.call_tool(params)` | `CallToolRequestParams { name, arguments, meta }`，`with_arguments(JsonObject)` |
| 收尾 | `client.cancel()` / `waiting()` | `waiting()` 等服务端结束 |

`ServiceExt::serve` 是**两端通用**的：server 端 `Handler::serve(stdio())`，client 端 `Handler::serve(transport)`，返回的都是 `RunningService<Role, Handler>`。

---

## 六、`ServerHandler` 除了 tools 还能做什么

`ServerHandler: Sized + Send + Sync + 'static`，默认方法全都有实现，按需覆写：

| 方法 | 对应协议能力 |
|---|---|
| `get_info() -> ServerConfig` | 声明能力（`enable_tools()` / `enable_resources()` / `enable_prompts()`…） |
| `list_tools` / `call_tool` | 工具（`#[tool_handler]` 自动实现） |
| `list_resources` / `read_resource` | 资源读取 |
| `list_resource_templates` | 参数化资源 |
| `list_prompts` / `get_prompt` | 提示词模板 |
| `complete` | 参数补全 |
| `set_level` / `logging` | 日志级别 |
| `on_progress` / `on_cancelled` / `on_custom_notification` | 进度、取消、自定义通知 |

`Sized + Send + Sync + 'static` 这三个约束在 `local` feature 下会放宽成 `Sized + 'static`（单线程 WASM 用）。

---

## 七、chrono：先说它是什么

chrono 解决的**不是**“怎么格式化字符串”，而是“**时间点怎么表示才不出错**”：

| 类型 | 表示什么 | 例子 |
|---|---|---|
| `DateTime<Utc>` | 绝对时间点（UTC），最推荐的存储形式 | `Utc::now()` |
| `DateTime<Local>` | 绝对时间点（本地时区） | `Local::now()` |
| `DateTime<FixedOffset>` | 带固定偏移 | 解析带 `+08:00` 的字符串 |
| `NaiveDateTime` / `NaiveDate` | **没有时区**的日历时间 | 生日、账期 |
| `Duration` | 时间差（chrono 自己的，和 `std::time::Duration` 不是一个类型） | `Duration::minutes(5)` |

核心原则：**存储和传输用 `DateTime<Utc>`，只在展示时转本地时区。** 用 `NaiveDateTime` 存“发生过的事”会丢掉时区信息，跨时区/夏令时必错。

### feature 表

| feature | 提供 | 说明 |
|---|---|---|
| `clock`（默认） | `Local::now()`、`Utc::now()` 的本地时区查询 | 依赖 `iana-time-zone` |
| `now` | `Utc::now()` | `rmcp` 只开了 `serde` + `now` |
| `std`（默认） | 标准库支持 | |
| `serde` | `DateTime` 的序列化 | ⚠️ 默认**没有**，要显式开 |

### 常用 API

```rust
use chrono::{DateTime, Duration, Local, TimeZone, Utc};

let now: DateTime<Utc> = Utc::now();

// 格式化
now.format("%Y-%m-%d %H:%M:%S").to_string();
now.to_rfc3339();                       // "2026-10-21T09:35:00.123456789+00:00"

// 解析
let dt = DateTime::parse_from_rfc3339("2026-10-21T09:35:00+08:00")?;  // DateTime<FixedOffset>
let dt_utc = dt.with_timezone(&Utc);

// 加减
let later = now + Duration::minutes(30);
let ok = now.checked_add_signed(Duration::days(1));   // 溢出返回 None

// 时间戳
let ts = now.timestamp();                             // 秒
let back = Utc.timestamp_opt(ts, 0).single();         // 注意 timestamp() 已 deprecated

// 本地展示
println!("{}", now.with_timezone(&Local));
```

### serde：时间怎么进 JSON

默认 `DateTime<Utc>` 序列化成 RFC3339 字符串（需要 `serde` feature）。要换格式用 `with`：

```rust
#[derive(serde::Serialize, serde::Deserialize)]
struct Event {
    id: String,
    #[serde(with = "chrono::serde::ts_milliseconds")]
    created_at: DateTime<Utc>,              // 毫秒时间戳（数字）
}
```

可用的模块：`ts_seconds`、`ts_milliseconds`、`ts_seconds_option`、`ts_milliseconds_option`。

---

## 八、两者结合：MCP 工具的入参可以直接是时间

`rmcp` 对 `schemars` 的依赖是 `schemars = "1.0"` 且 **开启了 `chrono04` feature**，而这个 feature 让 `schemars` 为 `chrono` 的 `DateTime` / `NaiveDate` 等类型生成 JSON Schema。所以你写：

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RemindArgs {
    /// When to fire, RFC3339 in UTC.
    pub at: chrono::DateTime<chrono::Utc>,
}
```

`#[tool]` 生成的 input schema 里 `at` 会是一个字符串 schema（带 `format: date-time`），客户端传 `"2026-10-21T10:00:00Z"` 就能反序列化——**不需要你手写 schema，也不需要 `#[serde(with = ...)]` 的额外处理**。

⚠️ 但要注意：`chrono04` 生成的 schema 对 `DateTime<Utc>` 记录的是**原始值**（字符串），所以时区必须由调用方按约定给（推荐统一 RFC3339 带 `Z`/`+00:00`），否则 `DateTime<Utc>` 和 `DateTime<FixedOffset>` 在比较时会因时区不同而“看起来相等、实际不等”。

---

## 九、⚠️ 本项目要注意的 5 件事

1. **`chrono` 的 feature 有个隐藏依赖**：现在 `Cargo.toml` 里是 `chrono = "0.4.45"`（走默认 feature：`clock`/`std`/`oldtime`/`wasmbind`），所以 `Local::now()` / `Utc::now()` 都能用。但**`serde` 不是默认 feature**——它之所以能用，是因为 `rmcp` 依赖 chrono 时开了 `features = ["serde", "now"]`，Cargo 的 feature 合并把它带进来了。

   这意味着：哪天移除 `rmcp`，`DateTime` 的序列化会**静默失效**（报 `the trait Serialize is not implemented`）。想让依赖自洽，显式写：

   ```toml
   chrono = { version = "0.4.45", features = ["serde"] }
   ```

2. **stdio 模式下 stdout 是协议通道**：这是 MCP server 最经典的坑。你现在 `main.rs` 用 `tracing_subscriber::FmtSubscriber`，默认写 **stdout**，一旦作为 MCP server 跑，日志会直接插进 JSON-RPC 流里，客户端解析立刻炸。必须改成 stderr：

   ```rust
   let subscriber = FmtSubscriber::builder()
       .with_max_level(Level::INFO)
       .with_writer(std::io::stderr)      // ← 关键
       .finish();
   ```

   （或者干脆用 `dotenvy` 读一个开关，只在调试时输出。）

3. **两套工具名要人工对齐**：`ChatCompletionTools` 里的 `name`（给 LLM 看）和 `#[tool(name = "...")]`（给 MCP 客户端看）是两套命名，别一边叫 `web_search`、一边叫 `search_web`，否则 dispatch 时对不上。

4. **`#[tool]` 入参必须 `Deserialize + JsonSchema`**，且要被 `Parameters<T>` 包一层。直接写 `async fn add(&self, a: f64, b: f64)` 是不行的。

5. **MSRV 1.88**：`rmcp 3.6.0` 声明的 `rust-version = "1.88"`，你本机 1.97.1 没问题，但别把项目锁到更低的 toolchain。

---

## 十、两个 crate 怎么放进这个项目

现在项目里是**进程内**工具（`Tool` trait + `ToolBox`），MCP 是**进程外**工具。推荐分层：

```
你的 agent 循环（complete.rs）
   └── 统一的工具注册表（08 篇的 Tool trait）
        ├── 本地实现：CalculatorTool / WebSearchTool → 直接调函数
        └── MCP 适配器：McpToolProxy → 通过 rmcp client 转发给远端 server
```

`McpToolProxy` 的骨架（把远端的 `rmcp::model::Tool` 适配成你本地的 `Tool`）：

```rust
#[async_trait::async_trait]
impl Tool for McpToolProxy {
    fn name(&self) -> &str { &self.remote_name }

    fn definition(&self) -> anyhow::Result<ChatCompletionTools> {
        // 把远端 tools/list 拿到的 name / description / input_schema 直接搬过来
        Ok(ChatCompletionTools::Function(ChatCompletionTool {
            function: FunctionObjectArgs::default()
                .name(self.remote_name.clone())
                .description(self.remote_desc.clone())
                .parameters((*self.remote_schema).clone())
                .build()?,
        }))
    }

    async fn execute(&self, args_json: &str) -> anyhow::Result<String> {
        let args: serde_json::Map<String, serde_json::Value> = serde_json::from_str(args_json)?;
        let result = self.client
            .call_tool(CallToolRequestParams::new(self.remote_name.clone()).with_arguments(args))
            .await?;
        Ok(serde_json::to_string(&result)?)
    }
}
```

这样 LLM 那边的 `tools` 列表和 dispatch 完全不用改——只是多了几个“远端工具”。

---

## 十一、速查

```rust
// ── rmcp：服务端 ─────────────────────────────
#[derive(Deserialize, JsonSchema)] struct Args { q: String }
#[tool_router] impl S { #[tool] async fn f(&self, Parameters(a): Parameters<Args>) -> Result<String,String> {..} }
#[tool_handler] impl ServerHandler for S { fn get_info(&self) -> ServerConfig { .. } }
S::new().serve(rmcp::transport::stdio()).await?.waiting().await?;

// ── rmcp：客户端 ─────────────────────────────
let t = TokioChildProcess::new(Command::new("npx").args([".."]))?;
let c = ().serve(t).await?;
c.list_tools(None).await?;
c.call_tool(CallToolRequestParams::new("f").with_arguments(map)).await?;

// ── chrono ───────────────────────────────────
Utc::now(); Local::now(); now.to_rfc3339(); DateTime::parse_from_rfc3339(s)?;
now + Duration::minutes(30); now.checked_add_signed(Duration::days(1));
#[serde(with = "chrono::serde::ts_milliseconds")] created_at: DateTime<Utc>,
```

**一句话总结**：`rmcp` 把「函数 + JSON Schema」这件事从**你的进程内**升级成**一个跨进程协议**（tools / resources / prompts + stdio 或 HTTP 传输）；`chrono` 则是让时间在这条链路上不出错的基础类型（`DateTime<Utc>` 存、RFC3339 传、本地只用于展示）。两者在“工具入参含时间”这一点上交汇——而且 `rmcp` 已经替你把 `schemars` 的 chrono 支持打开了。

---

## 与本系列其它文档的关系

- 与 [`08-async-trait.md`](./08-async-trait.md)：MCP 适配器要放进 `Box<dyn Tool>`，所以 `Tool` trait 的 `Send + Sync` 与 `#[async_trait]` 是前提；
- 与 [`03-tracing.md`](./03-tracing.md)：stdio 传输下必须把日志改到 stderr，那篇里 `FmtSubscriber` 的默认行为正好是个坑；
- 与 [`05-related-crates.md`](./05-related-crates.md)：`reqwest` / `serde` / `schemars` 这条链和 MCP 的 HTTP 传输、JSON Schema 是共用的。
