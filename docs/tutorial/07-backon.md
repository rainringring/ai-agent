# 07 · backon —— 重试与退避

> 版本：`backon 1.6.0`（源码核对自本地 registry）
> 它是 `backoff` crate 的后继，`backoff` 已停止维护。

---

## 一、它解决什么问题

调用外部服务（LLM API、数据库、HTTP）时，失败分两类：

| 类型 | 例子 | 该怎么办 |
|---|---|---|
| **可重试** | 429 限流、502/503、连接超时、DNS 抖动 | 等一会儿再试 |
| **不可重试** | 400 参数错、401 key 错、422 校验失败、上下文超长 | 立刻报错，重试一万次也一样 |

手写重试很容易写成这样：

```rust
loop {
    match call().await {
        Ok(v) => break Ok(v),
        Err(_) => tokio::time::sleep(Duration::from_secs(1)).await,  // ❌ 固定间隔
    }                                                               // ❌ 没上限
}                                                                   // ❌ 不区分错误
```

三个毛病：**固定间隔**（下游被打得更惨）、**没有上限**（死循环）、**不区分错误**（参数错了也重试）。

`backon` 提供的就是这三件事的正确答案：**退避策略 + 次数上限 + 条件重试**。

它还有两个工程上的好处：`no_std` 友好；通过 `Sleeper` 抽象支持 tokio / std / gloo / embassy，不绑死运行时。

---

## 二、三个核心概念

```
BackoffBuilder  ──build()──►  Backoff（一个 Iterator<Item = Duration>）
                                    │
Retryable::retry(闭包, builder) ────┘──► Retry（一个 Future）
```

| 概念 | 是什么 |
|---|---|
| `BackoffBuilder` | 策略的配置对象，`build()` 消耗自身生成策略 |
| `Backoff` | `Iterator<Item = Duration>`，每轮产出"这次该等多久" |
| `Retryable` | 扩展 trait，给"能产出 Future 的闭包"加上 `.retry()` |

注意 `BackoffBuilder::build(self)` 取的是 `self`（不是 `&self`），所以 builder 会被消耗；不过它对 `&ExponentialBuilder` 也实现了 `BackoffBuilder`，传引用也行。

---

## 三、快速开始

```rust
use anyhow::Result;
use backon::{ExponentialBuilder, Retryable};

async fn fetch() -> Result<String> {
    // 可能失败的真实调用
    Ok("ok".to_string())
}

#[tokio::main]
async fn main() -> Result<()> {
    let content = fetch
        .retry(ExponentialBuilder::default())
        .await?;

    println!("{content}");
    Ok(())
}
```

`fetch` 是**函数项**，它实现了 `FnMut() -> Fut`，所以能直接 `.retry(...)`。

⚠️ 关键点：`Retryable` 是给**闭包**实现的，不是给 Future。所以你必须把"调用"包起来，而不是先调用再重试那一个 Future（Future 只能用一次）。

```rust
// ✅ 函数项
fetch.retry(builder).await

// ✅ 闭包：注意整个闭包要加括号，否则 .retry 会被解析到 async 块外
(|| async { call_api(req.clone()).await }).retry(builder).await

// ❌ Future 不能重试
let fut = call_api(req);
fut.retry(builder)          // 编译不过
```

---

## 四、三种策略

### ExponentialBuilder（默认选它）

```rust
ExponentialBuilder::default()
    .with_min_delay(Duration::from_millis(500))
    .with_max_delay(Duration::from_secs(30))
    .with_factor(2.0)
    .with_max_times(5)
    .with_jitter()
```

默认值（源码 `new()`）：

| 参数 | 默认值 | 含义 |
|---|---|---|
| `min_delay` | 1s | 第一次退避 |
| `max_delay` | 60s | 单次退避上限 |
| `factor` | 2.0 | 每次翻倍 |
| `max_times` | **3** | **总尝试次数**（不是重试次数） |
| `jitter` | **false** | 抖动，默认关闭 |

⚠️ 两个默认值有坑：

- **`max_times = 3` 是"总共尝试 3 次"**（文档原文：maximum number of **attempts**），也就是最多重试 2 次。想重试 5 次要设 `with_max_times(6)`。
- **`jitter` 默认关**。多实例同时失败会同时重试（惊群），生产环境**一定**开 `.with_jitter()`。

### ConstantBuilder（固定间隔）

```rust
ConstantBuilder::default()
    .with_delay(Duration::from_secs(2))
    .with_max_times(3)
    .with_jitter()
```

适合"下游恢复时间是固定的"或调试期。

### FibonacciBuilder（斐波那契）

```rust
FibonacciBuilder::default()
    .with_min_delay(Duration::from_millis(100))
    .with_max_times(8)
```

增长比指数慢，适合"要多试几次但不想等太久"。

### 三者对比

| | 首次 | 后续 | 适合 |
|---|---|---|---|
| `Constant` | 固定 | 固定 | 简单场景、测试 |
| `Exponential` | 小 | 翻倍 | **通用首选** |
| `Fibonacci` | 小 | 慢增长 | 多次轻量重试 |

---

## 五、五个常用操作

### ① `.when()` —— 只对某些错误重试（最重要）

```rust
use async_openai::error::OpenAIError;

fetch
    .retry(ExponentialBuilder::default().with_max_times(4))
    .when(|e: &anyhow::Error| {
        // 只重试 429 / 5xx / 超时
        match e.downcast_ref::<OpenAIError>() {
            Some(OpenAIError::Reqwest(re)) => re.is_timeout() || re.is_connect(),
            _ => e.to_string().contains("429") || e.to_string().contains("rate limit"),
        }
    })
    .await?;
```

没有 `.when()` 时**任何错误都会重试**，包括 401（白等几轮）。这个方法是生产环境的必备项。

### ② `.notify()` —— 每次重试前打个日志

```rust
fetch
    .retry(builder)
    .notify(|err, dur| {
        tracing::warn!(error = %err, wait_secs = dur.as_secs_f32(), "重试中");
    })
    .await?;
```

签名是 `FnMut(&E, Duration)`，比你自己在循环里插日志省事。

### ③ `.adjust()` —— 自定义这次要等多久

典型用途：**尊重服务端的 `Retry-After`**。

```rust
fetch
    .retry(builder)
    .adjust(|err, dur| {
        // 服务端说等 10 秒，就听它的；否则用策略给的 dur
        server_retry_after(err).or(dur)
    })
    .await?;
```

签名：`FnMut(&E, Option<Duration>) -> Option<Duration>`，返回 `None` 表示放弃。

### ④ `.sleep()` —— 换睡眠实现

默认已经按 feature 选好（tokio 项目自动用 tokio）。想在测试里加速：

```rust
fetch
    .retry(builder)
    .sleep(backon::NoopSleeper)   // 不真睡，测试用
```

（`NoopSleeper` 需要开对应 feature，或自己实现 `Sleeper`。）

### ⑤ 同步版 `.call()`

```rust
use backon::{BlockingRetryable, ExponentialBuilder};

let result = (|| std::fs::read_to_string("a.txt"))
    .retry(ExponentialBuilder::default())
    .call()?;
```

同步方法用 `BlockingRetryable`，最后调 `.call()` 而不是 `.await`。

---

## 六、手动控制：只用策略，不要 Retryable

有时你想自己写循环（比如需要在重试之间做别的事）。那就只用 `build()` 拿迭代器：

```rust
use backon::{BackoffBuilder, ExponentialBuilder};

let mut backoff = ExponentialBuilder::default()
    .with_max_times(5)
    .build();

let mut attempt = 0;
loop {
    attempt += 1;
    match call().await {
        Ok(v) => break Ok(v),
        Err(e) if attempt >= 5 => break Err(e),
        Err(e) => match backoff.next() {
            Some(dur) => {
                tracing::warn!("第 {attempt} 次失败，等 {:?}", dur);
                tokio::time::sleep(dur).await;
            }
            None => break Err(e),   // 策略耗尽
        },
    }
}
```

这样你能在等待前后插入自己的逻辑（上报、切备用 endpoint 等）。

---

## 七、在 LLM 调用里的实践

### 该重试什么

| 情况 | 重试？ |
|---|---|
| 429 限流 | ✅ 必须，且用 `adjust` 听 `Retry-After` |
| 500 / 502 / 503 | ✅ |
| 连接超时、TLS 失败 | ✅ |
| 400 参数错 | ❌ 立刻报错 |
| 401 / 403 | ❌ key 错了，重试没用 |
| 422 校验失败 | ❌ |
| 上下文超长 | ❌ 重试也一样超长 |

### 请求需要 Clone

因为闭包每轮都要构造一个新的 future，所以 request 必须能重建：

```rust
let req = CreateChatCompletionRequestArgs::default()
    .model("gpt-4o-mini")
    .messages(messages)
    .build()?;

let resp = (|| {
    let req = req.clone();          // 每轮克隆一份
    async move { client.chat().create(req).await }
})
.retry(ExponentialBuilder::default().with_max_times(4).with_jitter())
.when(|e| is_retryable(e))
.await?;
```

`CreateChatCompletionRequest` 本身 `derive(Clone)`，所以这条路走得通。注意闭包必须是 `move` 拿走 `client` 的引用或克隆。

### 流式：重试要谨慎

流式请求的重试有个陷阱：**流已经开始 yield 了才失败，不能"续传"**。

- 只能在**建立连接阶段**重试（`create_stream(...).await` 这一步）。
- 一旦开始收 chunk，出错就只能整个重启，而用户已经看到了半截文字——体验上会重复。

所以实践做法：

```rust
// 只重试"拿到流"这一步
let stream = (|| {
    let req = req.clone();
    let c = client.clone();
    async move { c.chat().create_stream(req).await }
})
.retry(builder)
.when(is_retryable)
.await?;

// 之后的 chunk 消费不重试，出错就报给用户
while let Some(item) = stream.next().await { ... }
```

### 和超时配合

`backon` 只管"失败后等多久"，**不管单次要超时**。两者要一起用：

```rust
let resp = (|| async {
    tokio::time::timeout(Duration::from_secs(60), client.chat().create(req.clone()))
        .await
        .map_err(|_| anyhow::anyhow!("请求超时"))?
        .map_err(Into::into)
})
.retry(builder)
.when(|e| is_retryable(e))
.await?;
```

顺序是**内层超时、外层重试**。

---

## 八、坑位清单

| 坑 | 说明 |
|---|---|
| 以为 `max_times` 是重试次数 | 它是**总尝试次数**，默认 3 = 最多重试 2 次 |
| 忘了 `.with_jitter()` | 默认关，多实例会同步重试 |
| 不写 `.when()` | 401/400 也会重试，白等 |
| 对 Future 调 `.retry()` | `Retryable` 实现给 `FnMut() -> Fut`，必须先包闭包 |
| 闭包忘了括号 | `\|\| async {} .retry(..)` 会被解析成 `async {}` 内部调用 |
| 闭包里移动了 `req` | 第二轮拿不到，要 `clone()` |
| 用 `backoff` crate | 已弃用，用 `backon` |
| 重试流式的中途错误 | 无法续传，会重复输出 |
| 重试没有总时长限制 | 用 `with_total_delay` 或在最外层加 `tokio::time::timeout` |

---

## 九、速查

```rust
use backon::{ExponentialBuilder, BackoffBuilder, Retryable};

// 最常用组合
fetch
    .retry(
        ExponentialBuilder::default()
            .with_min_delay(Duration::from_millis(500))
            .with_max_delay(Duration::from_secs(20))
            .with_max_times(5)
            .with_jitter(),
    )
    .when(|e| is_retryable(e))
    .notify(|e, d| tracing::warn!(%e, wait = ?d, "retry"))
    .await?;
```

| 想做什么 | 写法 |
|---|---|
| 指数退避重试 | `.retry(ExponentialBuilder::default())` |
| 固定间隔 | `.retry(ConstantBuilder::default().with_delay(..))` |
| 限制次数 | `.with_max_times(n)`（n 是总尝试次数） |
| 加抖动 | `.with_jitter()` |
| 只重试某些错误 | `.when(\|e\| bool)` |
| 打日志 | `.notify(\|e, dur\| ..)` |
| 听 Retry-After | `.adjust(\|e, dur\| ..)` |
| 同步函数 | `BlockingRetryable` + `.call()` |
| 自己控制循环 | `builder.build()` 得到 `Iterator<Duration>` |

**一句话**：`backon` 把"失败了该等多久、最多试几次、哪种错才值得试"这三件事标准化了；对 LLM 调用，记得配 `.when()` 区分 429/5xx 与 4xx，并且流式只重试"建流"那一步。
