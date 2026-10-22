# 06 · futures 依赖与流式输出（stream）

> 基准：`futures 0.3.34`、`async-openai 0.41.3`、edition 2024
> 文中 API 都对着本地 `~/.cargo/registry/src/` 的源码核对过。

---

## 第一部分：futures 依赖

### 1. 它和 tokio 是什么关系

很多人以为这两个是竞争关系，其实是**分工**：

| | tokio | futures |
|---|---|---|
| 提供什么 | **运行时**：调度器、定时器、网络 IO、通道、文件 | **组合工具**：trait 定义与一堆组合器 |
| 能不能单独跑 | 能，`#[tokio::main]` 就是运行时 | 不能，它不管调度 |
| 类比 | 发动机 | 变速箱和传动轴 |

`tokio` 提供 `spawn`、`sleep`、`TcpStream`、`mpsc`；`futures` 提供 `.next()`、`.map()`、`join!`、`buffer_unordered()` 这些**把异步任务拼起来**的东西。

实际项目里两个都要：tokio 跑起来，futures 负责组织。

### 2. 它是个"门面 crate"，内部拆成好几个包

`futures` 自己几乎没有代码，它只是把下面这些重新导出：

| 子 crate | 内容 |
|---|---|
| `futures-core` | 核心 trait：`Stream`、`Sink`、`FusedStream` |
| `futures-util` | **绝大部分你用的东西**：`StreamExt`、`FutureExt`、`TryStreamExt`、`join!`、`select!`、各种组合器 |
| `futures-channel` | `mpsc`、`oneshot` |
| `futures-executor` | `block_on`、线程池（在 tokio 项目里一般不用） |
| `futures-io` | `AsyncRead`/`AsyncWrite` 老接口（tokio 用自己的） |
| `futures-task` | `Waker` 相关底层 |

所以 `futures = "0.3"` 拉进来的是一整包。想更精简可以只依赖 `futures-util`：

```toml
futures-util = "0.3"          # 只要 StreamExt / 组合器
# 或者
futures = "0.3"               # 方便，all in one
```

你项目里用的是后者，写 `use futures::StreamExt;` 就能用。

### 3. 三个核心 trait

| trait | 语义 | 你熟悉的对应物 |
|---|---|---|
| `Future` | 一个将来会产出一个值的东西 | `async fn` 的返回值 |
| `Stream` | 一个将来会产出**一系列**值的东西 | `Iterator` 的异步版 |
| `Sink` | 异步地"接收"值 | 很少直接用 |

关键区别：`Iterator::next()` 是**同步阻塞**的，`Stream::next()` 是**异步**的，必须 `.await`。

而 `.next()` 本身不在 `Stream` 上，在 `StreamExt` 上——这是 Rust 的惯例：核心 trait 只放必需方法，便捷方法放 `Ext` 扩展 trait。所以：

```rust
use futures::StreamExt;      // 不引入这行就没有 .next()
```

### 4. `StreamExt` 常用方法（背这张表就够了）

**取值**

| 方法 | 作用 |
|---|---|
| `.next().await` | 取下一个，返回 `Option<Item>` |
| `.try_next().await` | 同上，但 Item 是 `Result`，自动 `?`（来自 `TryStreamExt`） |
| `.collect::<Vec<_>>().await` | 全部收集 |
| `.fold(init, f).await` | 累积 |
| `.for_each(f).await` | 逐个处理 |

**变换**

| 方法 | 作用 |
|---|---|
| `.map(f)` | 逐项变换（f 是同步闭包） |
| `.filter(f)` | 过滤（f 返回 bool，可 async） |
| `.filter_map(f)` | 变换 + 过滤 |
| `.then(f)` | 逐项异步变换（f 返回 future，**串行**） |
| `.flat_map(f)` | 展开 |
| `.take(n)` / `.skip(n)` | 取前 n / 跳过 n |
| `.enumerate()` | 带上索引 |
| `.inspect(f)` | 偷看，不改变内容（调试用） |

**聚合与拼接**

| 方法 | 作用 |
|---|---|
| `.chain(other)` | 接上另一个流 |
| `.zip(other)` | 配对 |
| `.ready_chunks(n)` | 攒够 n 个一起给（**流式批量处理常用**） |
| `.fuse()` | 结束后再取返回 None 而不是 panic |

**并发控制（重点）**

| 方法 | 作用 |
|---|---|
| `.buffer_unordered(n)` | 同时最多跑 n 个内部 future，谁先完谁先出 |
| `.buffered(n)` | 同上但保持顺序 |
| `.buffer_unordered(n)` + `stream::iter` | 典型的"并发抓 N 个 URL" |

**装箱**

| 方法 | 作用 |
|---|---|
| `.boxed()` | 变成 `BoxStream<'static, T>`（要求 Send） |
| `.boxed_local()` | 不要 Send，单线程用 |

### 5. 怎么造一个流

```rust
use futures::stream::{self, StreamExt};

stream::iter(vec![1, 2, 3])         // 从集合
stream::once(async { 42 })          // 只有一个元素
stream::empty::<i32>()              // 空流
stream::repeat(1)                   // 无限重复
stream::unfold(state, |s| async { .. })  // 用状态机手工造流
```

`stream::unfold` 值得单独记，async-openai 内部就是用它把 SSE 事件流变成类型流：

```rust
Box::pin(futures::stream::unfold(
    (event_stream, event_mapper),
    |(mut event_stream, event_mapper)| async move {
        // 取一个 SSE event，映射成目标类型，返回 (item, next_state)
    },
))
```

### 6. 组合多个 future：`join!` / `try_join!` / `select!`

```rust
use futures::{join, try_join, select};

// 全部完成，取所有结果（并发，同一任务）
let (a, b) = join!(f1(), f2());

// 全部成功，任一失败立刻返回
let (a, b) = try_join!(f1(), f2())?;

// 谁先完成要谁，其余取消
select! {
    v = f1() => println!("f1: {v:?}"),
    v = f2() => println!("f2: {v:?}"),
}
```

和 tokio 的区别：`tokio::join!`/`tokio::select!` 功能一样，**都能用**。tokio 的还支持 `select!` 里的 `else` 分支和 biased 模式。项目里已经用 tokio 的话，用 tokio 的版本没必要再引入 futures 的。

futures 独有的是 `FuturesUnordered`——动态集合的并发：

```rust
use futures::stream::FuturesUnordered;
use futures::StreamExt;

let mut set = FuturesUnordered::new();
for url in urls { set.push(fetch(url)); }

while let Some(result) = set.next().await {
    // 完成一个处理一个，顺序不确定
}
```

`buffer_unordered` 就是它的流式封装。

### 7. Pin 和 BoxStream：为什么流要装箱

多数流类型不是 `Unpin`（内部有自引用），而 `StreamExt::next(&mut self)` 要求 `Self: Unpin`。所以有两种处理：

```rust
// 方案一：栈上 pin
use futures::pin_mut;
let s = some_stream();
pin_mut!(s);          // 之后可以 s.next().await

// 方案二：堆上装箱（库的常规做法）
type BoxStream<T> = Pin<Box<dyn Stream<Item = T> + Send>>;
```

**第二种就是 async-openai 的做法**，它把流定义成：

```rust
pub type StreamResponse<T> =
    Pin<Box<dyn futures::Stream<Item = Result<T, OpenAIError>> + Send>>;
```

好处：`Pin<Box<...>>` 本身是 `Unpin`（装箱的是指针），所以调用方**直接** `.next().await`，不需要 `pin_mut!`。代价是一次堆分配。

看到 `impl Stream` 返回类型时记住：**它可能是 `!Unpin` 的**，需要 `pin_mut!` 才能在栈上用；而 `Pin<Box<dyn Stream>>` 不用。

### 8. futures 的坑

| 坑 | 说明 |
|---|---|
| 忘了 `use futures::StreamExt` | `no method named 'next'`，最常见的报错 |
| 在 tokio 项目里用 `futures::executor::block_on` | 会阻塞 tokio 工作线程，甚至死锁。用 `tokio::main` / `.await` |
| `join!` 不等于并行 | 它在**当前任务**里并发轮询，不开新线程；要真并行得 `tokio::spawn` |
| `.then` 是串行 | 想并发用 `.buffer_unordered(n)` |
| `stream::iter` 里的 `async` 块不会被 await | `iter` 产出的是值，要 `then` 才展开 |
| 忘了 `.await` 一个 future | 编译器会警告 `unused_must_use`，不会执行 |
| 用 `collect()` 吃无限流 | OOM，用 `take(n)` 先截断 |

---

## 第二部分：流式输出（async-openai）

### 1. 为什么用流式

| | 非流式 `create` | 流式 `create_stream` |
|---|---|---|
| 首个字符何时到 | 整段生成完 | 生成第一个字就到 |
| 体验 | 等 5-10 秒后一次性出现 | 打字机式涌现 |
| 复杂度 | 低 | 高（增量、拼接、中断） |

关键指标 **TTFT**（Time To First Token）：流式把用户感知延迟从"总时长"降到"第一个 token 的时长"。

### 2. 两个方法互相拦截

```rust
client.chat().create(req).await?          // 一次性
client.chat().create_stream(req).await?   // 流式
```

源码里的校验：

```rust
// create() 里
if request.stream.is_some() && request.stream.unwrap() {
    return Err(OpenAIError::InvalidArgument("When stream is true, use Chat::create_stream".into()));
}

// create_stream() 里
if request.stream.is_some() && !request.stream.unwrap() {
    return Err(OpenAIError::InvalidArgument("When stream is false, use Chat::create".into()));
}
request.stream = Some(true);   // 它自己设
```

结论：**不用手写 `.stream(true)`**；但显式写 `.stream(false)` 再调 `create_stream` 会报错。

### 3. 返回类型

```rust
pub type ChatCompletionResponseStream =
    StreamResponse<CreateChatCompletionStreamResponse>;
```

三个要点：

- 是 `futures::Stream`，用 `.next().await`
- 每一项是 `Result<_, OpenAIError>`——**流中途也可能出错**
- 服务器的 `[DONE]` 由内部消化，流自然返回 `None`

### 4. 一个 chunk 的结构（三层）

顶层 `CreateChatCompletionStreamResponse`：

| 字段 | 说明 |
|---|---|
| `id` | 每块相同 |
| `choices` | `Vec<ChatChoiceStream>` |
| `created` / `model` | 元信息 |
| `object` | 固定 `"chat.completion.chunk"` |
| `usage` | 只有开了 `include_usage` 才有，且只在最后一块 |

中层 `ChatChoiceStream`：

| 字段 | 说明 |
|---|---|
| `index` | 第几个候选 |
| `delta` | **增量内容** |
| `finish_reason` | 最后一块才非 None |
| `logprobs` | 概率 |

底层 `ChatCompletionStreamResponseDelta`：

| 字段 | 说明 |
|---|---|
| `content` | 文本片段（可能 None） |
| `tool_calls` | 分片的工具调用 |
| `role` | 通常只在第一块 |
| `refusal` | 拒答片段 |
| `function_call` | 已废弃 |

**最要紧一条：`delta.content` 是增量，不是累计。** 全文得自己拼。

### 5. 最小消费代码

```rust
use anyhow::{Context, Result};
use futures::StreamExt;                 // 必须有
use std::time::Duration;

async fn stream_once(
    client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    req: async_openai::types::chat::CreateChatCompletionRequest,
) -> Result<String> {
    let mut stream = client
        .chat()
        .create_stream(req)
        .await
        .context("发起流式请求失败")?;

    let mut out = String::new();

    loop {
        // 超时包在"每一块"上，而不是整个流
        let next = tokio::time::timeout(Duration::from_secs(30), stream.next()).await;

        let item = match next {
            Ok(v) => v,
            Err(_) => break,                       // 30 秒没新数据，放弃
        };

        let Some(chunk) = item else { break };     // 流结束
        let chunk = chunk?;                        // 流中途错误

        if let Some(text) = chunk
            .choices
            .first()
            .and_then(|c| c.delta.content.as_deref())
        {
            print!("{text}");                      // 给人看
            out.push_str(text);                    // 给程序用
        }

        if let Some(usage) = chunk.usage {
            tracing::info!(
                prompt = usage.prompt_tokens,
                completion = usage.completion_tokens,
                "token 用量"
            );
        }
    }

    Ok(out)
}
```

三个设计点：

1. `Option<Result<_,_>>` 有**两种**结束方式：`None`（正常结束）和 `Some(Err)`（中途失败），都要处理。
2. 超时用 `tokio::time::timeout` 包每次 `next()`，因为长回复总时长可能几分钟。
3. 终端里 `print!` 有缓冲，想要即时显示需要 `use std::io::Write; std::io::stdout().flush()?;`。

### 6. 要 usage 统计

```rust
use async_openai::types::chat::ChatCompletionStreamOptions;

let req = CreateChatCompletionRequestArgs::default()
    .model("gpt-4o-mini")
    .stream_options(ChatCompletionStreamOptions {
        include_usage: Some(true),
        include_obfuscation: None,
    })
    .messages(messages)
    .build()?;
```

`ChatCompletionStreamOptions` 只有两个字段，都是 `Option`。`include_obfuscation` 是 OpenAI 的防窥探填充，兼容服务通常忽略。

### 7. `tool_calls` 在流里是碎片

`ChatCompletionMessageToolCallChunk`：

| 字段 | 说明 |
|---|---|
| `index` | 第几个工具调用，**归组靠它** |
| `id` | 只在第一片出现 |
| `function` | `Option<FunctionCallStream>` |

`FunctionCallStream`：`name`（通常第一片给全）+ `arguments`（**JSON 字符串碎片**）。

拼接：

```rust
use std::collections::BTreeMap;

let mut calls: BTreeMap<u32, (String, String)> = BTreeMap::new();

for call in chunk.choices.iter().flat_map(|c| c.delta.tool_calls.iter().flatten()) {
    let entry = calls.entry(call.index).or_default();
    if let Some(f) = &call.function {
        if let Some(n) = &f.name { entry.0 = n.clone(); }
        if let Some(a) = &f.arguments { entry.1.push_str(a); }
    }
}
// 流结束后 entry.1 才是完整 JSON，此时才能 serde_json::from_str
```

两个坑：

- **流没结束别解析**，碎片一定不是合法 JSON。
- **多工具并发时碎片交错到达**，必须按 `index` 分开，不能用一个 String。

### 8. 中断与资源

- 提前 `break` 或丢弃流 → 底层连接释放（库里有测试 `dropping_stream_releases_idle_upstream_response`）。
- Ctrl-C 不用手动清理。
- 放在 `tokio::spawn` 里时注意：`StreamResponse` 带 `Send`，可以移进 task。

### 9. 把协议流变成字符串流（用 async-stream）

想解耦"chunk 流"和"文本流"，用 `async-stream`（你项目已有）：

```rust
use async_stream::try_stream;
use futures::Stream;

pub fn chat_stream(
    client: async_openai::Client<async_openai::config::OpenAIConfig>,
    req: async_openai::types::chat::CreateChatCompletionRequest,
) -> impl Stream<Item = anyhow::Result<String>> {
    try_stream! {
        let mut s = client.chat().create_stream(req).await?;
        while let Some(item) = s.next().await {
            let chunk = item?;
            if let Some(t) = chunk
                .choices
                .first()
                .and_then(|c| c.delta.content.as_deref())
            {
                yield t.to_string();
            }
        }
    }
}
```

`try_stream!` 里 `yield x` 等于产出 `Ok(x)`，`?` 会把错误变成流里的一项然后结束。

**edition 2024 的差异**：`impl Stream` 现在会自动捕获所有入参生命周期；2021 及更早必须写 `+ '_`。

⚠️ 注意：`impl Stream` 返回的是 `!Unpin` 类型，调用方要 `.next()` 得先 `futures::pin_mut!`：

```rust
let s = chat_stream(client, req);
futures::pin_mut!(s);
while let Some(chunk) = s.next().await { /* ... */ }
```

这正是为什么库更愿意返回 `Pin<Box<dyn Stream>>`——省掉调用方的 pin 麻烦。

---

## 第三部分：类型层面的常见错误

想"返回一个流"，但写出这三种问题的典型：

```rust
// ❌
pub async fn chat_complete(model: &str) -> impl Stream<Item = anyhow::Result<String>> {
    let client = Client::new();
    ...
    .build()?;                                    // 问题 2
    let resp = client.chat().create(req).await?;  // 问题 3
    Ok(content)                                   // 类型不匹配
}
```

**问题 1：缺 `use futures::Stream;`**

返回类型写了 `Stream` 但 trait 不在作用域 → `error[E0405]: cannot find trait Stream in this scope`。

**问题 2：`async fn` 配 `impl Stream` 配 `?`**

- `async fn f() -> X`：调用得到 Future，`.await` 后得到 `X`。
- 返回 `impl Stream`：调用**直接**得到流。

`?` 要求函数返回 `Result`/`Option`，而这里返回的是流，没有位置接收错误。

**问题 3：返回流却调了 `create()`**

要 `create_stream()`。

正确形状（三选一）：

```rust
// ① 直接返回库的流类型（最简单）
async fn start(req: ..) -> Result<ChatCompletionResponseStream> {
    client.chat().create_stream(req).await
}

// ② 非 async + try_stream!
fn chat_stream(..) -> impl Stream<Item = Result<String>> { try_stream! { .. } }

// ③ 用 BoxStream 自己装箱
fn chat_stream(..) -> BoxStream<'static, Result<String>> { .. }
```

---

## 附录：`features` 和 `futures` 是两个词

容易混，区别如下：

| | `futures` | `features` |
|---|---|---|
| 是什么 | 一个 crate 的名字 | `Cargo.toml` 里的配置项 |
| 作用 | 提供 Stream/Future 的组合工具 | 控制依赖里编译哪些代码 |
| 写在哪 | `[dependencies] futures = "0.3"` | `features = ["derive"]` |
| 生效时机 | 编译进你的程序 | 编译期决定代码在不在 |

顺带记两条本项目相关的：

```toml
# async-openai 默认只开 rustls，不带 Client，必须显式开接口
async-openai = { version = "0.41.0", features = ["chat-completion"] }

# serde 的 derive 也是 feature
serde = { version = "1.0.228", features = ["derive"] }
```

查当前实际启用了什么：

```powershell
cargo tree -f "{p} | features: {f}" -p async-openai --depth 0
```

---

## 速查

**futures**

| 我想…… | 写法 |
|---|---|
| 取下一个 | `stream.next().await` |
| 全部收集 | `stream.collect::<Vec<_>>().await` |
| 逐项异步处理（串行） | `.then(\|x\| async { .. })` |
| 并发 N 个 | `.buffer_unordered(8)` |
| 攒批 | `.ready_chunks(10)` |
| 并行等两个 | `futures::join!` / `tokio::join!` |
| 竞速 | `futures::select!` / `tokio::select!` |
| 栈上 pin | `futures::pin_mut!(s)` |
| 装箱 | `BoxStream<'static, T>` |

**stream**

| 我想…… | 写法 |
|---|---|
| 发流式请求 | `client.chat().create_stream(req).await?` |
| 取增量文本 | `chunk.choices.first().and_then(\|c\| c.delta.content.as_deref())` |
| 要 usage | `.stream_options(ChatCompletionStreamOptions { include_usage: Some(true), .. })` |
| 单块超时 | `tokio::time::timeout(d, stream.next()).await` |
| 拼 tool_calls | 按 `index` 分组累加 `arguments` |

---

**一句话**：`futures` 给你组合异步的语法，`stream` 是其中"一系列值"的那种形态；async-openai 的流是"一堆 `Result<chunk>`"，`delta` 是增量、`tool_calls` 是碎片，都得你自己拼。
