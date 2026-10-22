# Rust 四件套学习笔记：anyhow / serde / tracing / dotenvy

本目录围绕 `ai-agent` 项目实际用到的依赖展开，讲清楚**每件工具解决什么问题、怎么用、常见操作怎么组合、哪里会踩坑**。

内容不是从零教 Rust，而是假设你已经能写 `fn main()` 和 `Result`，想把这几个 crate 用对。

---

## 版本基准

以下版本来自本项目 `Cargo.lock` 的实际解析结果（不是 `Cargo.toml` 里写的版本号）。写代码时以这里为准。

| crate | Cargo.toml 写的 | 实际解析 | 备注 |
|---|---|---|---|
| `anyhow` | `1.0.102` | **1.0.104** | 语义化版本自动升级 |
| `serde` | `1.0.228` | **1.0.229** | 必须开 `derive` feature |
| `serde_json` | `1.0.150` | **1.0.151** | 被 anyhow 之外的 async-openai 也间接使用 |
| `tracing` | `0.1.44` | 0.1.44 | 默认 feature: `std` + `attributes` |
| `tracing-subscriber` | `0.3.23` | 0.3.23 | **默认不含 `env-filter` / `json`** |
| `tracing-core` | —— | 0.1.36 | 传递依赖，一般不用直接引用 |
| `tracing-attributes` | —— | 0.1.31 | 传递依赖，提供 `#[instrument]` |
| `dotenvy` | `0.15.7` | 0.15.7 | feature 只有 `cli` |
| `tokio` | `1.52.3` | **1.53.1** | 开了 `full` |
| `async-openai` | `0.41.0` | **0.41.3** | 用 `derive_builder` 构造请求 |
| `async-trait` | `0.1.92` | 0.1.92 | 纯 proc-macro，让 trait 里的 `async fn` 能当 `dyn` 用 |
| `rmcp` | `3.6.0` | 3.6.0 | MCP 官方 SDK；MSRV **1.88**；`server` feature 会带上 `schemars` |
| `chrono` | `0.4.45` | 0.4.45 | 默认 feature 已含 `clock`（`Local::now()` 可用）；`serde` 是靠 `rmcp` 合并进来的，建议显式写 |
| `uuid` | `1.28.0`（features = `v4`） | 1.28.0 | 只有 `v4`；要用 `v7` 或 serde 需另开 feature |
| `rustc` | —— | 1.97.1 | `edition = "2024"` 要求 ≥ 1.85 |

### 关于 `1.0.102` → `1.0.104`

`Cargo.toml` 里写 `"1.0.102"` 在 Cargo 里等价于 `^1.0.102`，即 **允许 1.x 内任意更高版本**。所以真正装的是 1.0.104。

想要完全锁定，必须提交 `Cargo.lock`（本项目已有）。这也是为什么删掉 `Cargo.lock` 后下次编译还得重新联网解析——见本目录 `05-related-crates.md` 的排错章节。

---

## 四件套各自的位置

一句话版本：

| crate | 一句话定位 | 你什么时候碰它 |
|---|---|---|
| **anyhow** | 应用层的错误包装：一个统一的错误类型 + 人话上下文链 | 所有 `Result` 返回值、`?` 传播 |
| **serde** | 序列化框架：让你的 struct ↔ JSON 互转 | 定义 API 请求/响应模型、配置文件 |
| **tracing** | 结构化日志 + 时间区间（span），异步友好 | 打日志、追踪一次请求的全过程 |
| **dotenvy** | 从 `.env` 文件读环境变量到进程环境 | 程序启动第一行，加载 API key |

它们不是竞争关系，而是**分工覆盖一个程序从启动到报错的完整链路**：

```
                    ┌──────────────┐
  程序启动 ────────► │  dotenvy     │  把 .env 里的 OPENAI_API_KEY 读进进程环境
                    └──────┬───────┘
                           ▼
                    ┌──────────────┐
                    │  tracing     │  初始化 subscriber，之后所有日志走它
                    │  (-subscriber)│
                    └──────┬───────┘
                           ▼
                    ┌──────────────┐
                    │  async-openai│  读 OPENAI_API_KEY 建 Client
                    └──────┬───────┘
                           ▼
                    ┌──────────────┐
                    │  serde       │  请求体序列化成 JSON、响应反序列化回 struct
                    └──────┬───────┘
                           ▼
                    ┌──────────────┐
                    │  anyhow      │  任何一步出错，加一层上下文往上抛
                    └──────────────┘
```

---

## 依赖关系（谁依赖谁）

```
ai-agent (你的二进制 crate)
├── anyhow                 直接依赖，应用层错误
├── serde        [derive]  直接依赖，数据结构 ↔ JSON
├── serde_json             直接依赖，JSON 编解码引擎
├── tokio        [full]    直接依赖，异步运行时
├── tracing                直接依赖，日志门面
├── tracing-subscriber     直接依赖，日志实现（默认无 env-filter）
├── dotenvy                直接依赖，.env 加载
└── async-openai           直接依赖，OpenAI 客户端
    ├── reqwest              HTTP 客户端
    ├── rustls / aws-lc-rs   TLS 实现（不是 openssl）
    ├── serde + serde_json  内部用它描述所有 API 类型
    ├── thiserror           内部错误类型
    └── derive_builder      请求构造器（`XxxArgs::default()...build()`）
```

几个值得注意的点：

- **`serde` 被两处使用**：你自己写的 struct，和 async-openai 内部成百上千个 API 类型。所以 `serde` 属性的写法直接影响你能不能正确解析 API 响应。
- **async-openai 用的是 `rustls` 而不是 `openssl`**：这意味着 TLS 行为由纯 Rust 实现决定，不依赖系统 OpenSSL，但**依然会走系统证书验证**。
- **`tracing-subscriber` 的默认 feature 是个坑**：只有 `smallvec`/`fmt`/`ansi`/`tracing-log`/`std`。想用 `EnvFilter` 或 JSON 输出，必须显式加 feature。详见 `03-tracing.md`。

---

## 学习顺序

建议按这个顺序读，因为后面会用到前面的东西：

1. **[`01-anyhow.md`](./01-anyhow.md)** —— 错误处理。最短、最常用，先建立"错误带上下文"的习惯。
2. **[`02-serde.md`](./02-serde.md)** —— 数据建模。理解属性表，后面 async-openai 的类型一看就懂。
3. **[`03-tracing.md`](./03-tracing.md)** —— 日志。含本项目必须改的那行 `Cargo.toml`。
4. **[`04-dotenvy.md`](./04-dotenvy.md)** —— 环境变量。很短，但顺序错误会导致一个很隐蔽的 bug。
5. **[`05-related-crates.md`](./05-related-crates.md)** —— tokio / async-openai / thiserror，以及把所有东西串起来的完整 `main.rs`。
6. **[`06-futures-and-stream.md`](./06-futures-and-stream.md)** —— futures 与流式输出，写流式对话前必读。
7. **[`07-backon.md`](./07-backon.md)** —— 重试与退避，接外部服务后才需要。
8. **[`08-async-trait.md`](./08-async-trait.md)** —— trait 里的 `async fn` 与 `dyn` 分发，做工具注册表时用。
9. **[`09-rmcp-and-chrono.md`](./09-rmcp-and-chrono.md)** —— 把工具暴露成 MCP 服务（stdio 传输）、时间类型与 `DateTime` 序列化。
10. **[`10-uuid.md`](./10-uuid.md)** —— 128 位标识符：v4 / v7 的选择、feature 依赖、serde 与主键实践。

---

## 文件索引

| 文件 | 内容 |
|---|---|
| [`README.md`](./README.md) | 本文件：版本基准、依赖关系、学习顺序 |
| [`01-anyhow.md`](./01-anyhow.md) | `Result`、`Context`、`bail!`/`ensure!`、错误链、下转型、与 thiserror 的边界 |
| [`02-serde.md`](./02-serde.md) | derive、属性表、枚举的四种表示、`serde_json` 操作、缺失字段规则 |
| [`03-tracing.md`](./03-tracing.md) | 事件/span/subscriber、`#[instrument]`、`EnvFilter`、与 anyhow 配合 |
| [`04-dotenvy.md`](./04-dotenvy.md) | `.env` 语法、加载顺序、覆盖语义、测试隔离 |
| [`05-related-crates.md`](./05-related-crates.md) | tokio、async-openai 0.41、thiserror、完整可编译示例、排错清单 |
| [`06-futures-and-stream.md`](./06-futures-and-stream.md) | futures 依赖、StreamExt 常用方法、Pin/BoxStream、流式输出与 chunk 拼接 |
| [`07-backon.md`](./07-backon.md) | 重试与退避：三种策略、`when`/`notify`/`adjust`、LLM 调用的重试边界 |
| [`08-async-trait.md`](./08-async-trait.md) | `async-trait` 的作用（宏展开前后）、`?Send`、dyn 三种形态、代价、何时改用 enum 分发、本项目 `Tool` trait + 注册表分发 |
| [`09-rmcp-and-chrono.md`](./09-rmcp-and-chrono.md) | rmcp 的角色/feature/传输、四段式最小 server、客户端用法、`ServerHandler` 能力表；chrono 的 `DateTime<Utc>`、格式化解析、serde 时间戳、与 schemars `chrono04` 的结合 |
| [`10-uuid.md`](./10-uuid.md) | UUID 的版本家族（v4 随机 / v7 时间有序）、本项目 feature 现状、常用构造/解析/格式化、serde 缺失的后果、8 个踩坑、trace_id 与主键实践 |

---

## 约定

- 代码块里的 `// ...` 表示省略，不是可编译代码。
- 标注 **⚠️ 本项目** 的地方，是结合当前 `Cargo.toml` / 代码现状的具体提醒，不是通用知识。
- 所有 API 都对着本地缓存的 crate 源码核对过（`~/.cargo/registry/src/`），不是凭记忆写的。
