# 08 · async-trait —— 让 trait 里的 async 方法能当 `dyn` 用

> 版本：`async-trait 0.1.92`（源码核对自本地 registry，与你 `Cargo.toml` 里锁的一致）
> 环境：`rustc 1.97.1`、`edition 2024`
> 它是**纯过程宏**：没有运行时、没有类型包装、没有额外依赖，只在编译期把 `async fn` 改写成普通方法。

---

## 一、它解决什么问题

Rust 1.75 起已经可以原生写「trait 里的 async 方法」（AFIT）：

```rust
trait Tool {
    fn name(&self) -> &'static str;
    async fn call(&self, input: &str) -> String;
}
```

但这样写出来的 trait **不是 dyn compatible**，一旦想放进 `Box<dyn Tool>` 就编译不过。实测报错（本项目环境下真跑出来的）：

```
error[E0038]: the trait `Tool` is not dyn compatible
 --> _afit.rs:7:26
7 | fn dispatch(tools: &[Box<dyn Tool>]) {
  |                          ^^^^^^^^ `Tool` is not dyn compatible
note: for a trait to be dyn compatible it needs to allow building a vtable
 --> _afit.rs:4:14
4 |     async fn call(&self, input: &str) -> String;
  |              ^^^^ ...because method `call` is `async`
```

原因很朴素：`async fn` 的返回类型是**匿名的 `impl Future`**，编译器不知道它有多大，就没法为 vtable 计算偏移。

三种调用方式对照：

| 写法 | 需要 async-trait 吗 | 说明 |
|---|---|---|
| `fn f<T: Tool>(t: T)`（单态/泛型） | ❌ 不需要 | 原生 AFIT 就够，编译期单态化，零分配 |
| `Box<dyn Tool>` / `Vec<Box<dyn Tool>>` / `Arc<dyn Tool>` | ✅ 需要 | 运行时多态，vtable 必须有具体签名 |
| 想把 `dyn` 的 future 丢进 `tokio::spawn` | ✅ 需要（或手写 `+ Send` 签名） | spawn 要求 `Send + 'static` |

`async-trait` 干的事就一件：**把 `async fn` 改写成返回 `Pin<Box<dyn Future + Send>>` 的普通 `fn`**，于是返回类型变具体、vtable 能建、future 也自动带上 `Send`。

---

## 二、它把代码改成了什么

```rust
// 你写的
#[async_trait]
pub trait Tool: Send + Sync {
    async fn call(&self, input: &str) -> Result<String, String>;
}
```

```rust
// 展开后（等价语义，省略路径前缀）
pub trait Tool: Send + Sync {
    fn call<'life0, 'life1, 'async_trait>(
        &'life0 self,
        input: &'life1 str,
    ) -> Pin<Box<
            dyn Future<Output = Result<String, String>> + Send + 'async_trait,
        >>
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait;
}
```

三个改写点，正好对应三个好处：

| 改写 | 好处 |
|---|---|
| 返回 `Pin<Box<dyn Future>>` | 返回类型**具体、定大小** → vtable 可建 → `dyn` 可用 |
| 自动加 `+ Send` | future 可以跨线程，能 `tokio::spawn` |
| 自动生成 `'async_trait` 生命周期 + `where` 约束 | 解决「future 同时借用 `&self` 和参数」的生命周期问题（手写这堆约束非常痛苦） |

想不用宏也可以，就是自己写上面那段签名——宏只是替你写样板代码。

---

## 三、快速开始（这份代码实测编译并运行过）

```rust
use async_trait::async_trait;

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;

    async fn call(&self, input: &str) -> Result<String, String>;

    // 默认方法也可以
    async fn describe(&self) -> String {
        format!("tool:{}", self.name())
    }
}

pub struct Echo;

#[async_trait]                    // ← impl 上必须也标注
impl Tool for Echo {
    fn name(&self) -> &'static str {
        "echo"
    }

    async fn call(&self, input: &str) -> Result<String, String> {
        Ok(format!("echo:{input}"))
    }
}

fn main() {
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(Echo)];
    for t in &tools {
        println!("name = {}", t.name());
        println!("call = {:?}", futures::executor::block_on(t.call("hi")));
        println!("desc = {}", futures::executor::block_on(t.describe()));
    }
}
```

输出：

```
name = echo
call = Ok("echo:hi")
desc = tool:echo
```

`Box<dyn Tool>` 能用了、trait 里的默认 async 方法也能用了——这两点正是没有宏时做不到的。

---

## 四、常用用法

### 1. trait 和每个 impl 都要标注

```rust
#[async_trait]
pub trait Tool { async fn call(&self) -> String; }

#[async_trait]                          // ← 漏了这个必炸
impl Tool for WebSearch { async fn call(&self) -> String { ... } }
```

漏了就报 `method 'call' has an incompatible type for trait: expected 'Pin<Box<...>>', found 'impl Future<...>'`。

### 2. `?Send`：不需要跨线程时

```rust
#[async_trait(?Send)]
pub trait LocalTool { async fn call(&self); }
```

用于单线程 runtime（`tokio::task::spawn_local`）、WASM，或 future 里捕获了 `Rc` / `RefCell` 的场景。**trait 与 impl 两侧必须一致**。

### 3. trait object 的三种常见形态

```rust
Box<dyn Tool>                          // 独占
Arc<dyn Tool + Send + Sync>            // 共享给多个 task
Vec<Box<dyn Tool>> / HashMap<&str, Arc<dyn Tool>>   // 工具表 / 按名分发
```

`call(&self)` 的 future 借用了 `self`，所以要 `tokio::spawn` 就必须把 `Arc` clone 进任务：

```rust
let me = Arc::clone(&tool);
tokio::spawn(async move { me.call(args).await });
```

### 4. 参数上的限制：`dyn` 里不能有泛型方法

```rust
#[async_trait]
pub trait Tool {
    async fn call<T: Into<u32>>(&self, t: T);   // ❌ 又变成 non-dyn-compatible
}
```

这是 `dyn` 的固有规则（vtable 只能放具体签名），**不是 async-trait 的限制**。要泛型就走单态调用，或在 trait 里收窄成具体类型（`serde_json::Value`、`&dyn Any`）。

### 5. `&mut self` 也能用

```rust
#[async_trait]
pub trait Counter { async fn bump(&mut self) -> u32; }
```

但 `Arc<dyn Counter>` 拿不到 `&mut`，共享可变状态要用 `Mutex`/`RwLock`/原子类型，这是借用规则的事，跟宏无关。

---

## 五、代价

| 项 | 说明 |
|---|---|
| 每次调用一次堆分配 | `Pin<Box<dyn Future>>` 每次调用都 box 一个 future |
| 一次虚调用 | 走 vtable，比单态调慢（但远不到数量级） |
| 二进制体积 | 每个 impl 多一份 box 化的函数体 |

对「调 LLM / 发 HTTP」这种毫秒级操作完全无感；如果是每秒百万次的高频小函数，才需要考虑换成 `enum` 分发。

---

## 六、什么时候**不用**它

| 场景 | 建议 |
|---|---|
| 工具集固定、数量少 | 用 `enum ToolKind { Calculator, WebSearch }` + `match`：零分配、无 `dyn`、编译器帮你穷尽检查 |
| 只在泛型里用 | 原生 `async fn` in trait（1.75+）就够 |
| 需要运行时注册、可插拔、`Vec<Box<dyn>>` | `async-trait` 是对的 |
| 必须 `tokio::spawn` | `async-trait`（自动 `Send`）或手写 `+ Send` 的 boxed 签名 |

`enum` 分发的样子（仅供参考，本项目现用 `Vec<ChatCompletionTools>` + 名字分发更灵活）：

```rust
pub enum ToolKind { Calculator, WebSearch }

impl ToolKind {
    pub async fn call(&self, args: serde_json::Value) -> anyhow::Result<String> {
        match self {
            ToolKind::Calculator => calculator_dispatch(args).await,
            ToolKind::WebSearch => web_search_dispatch(args).await,
        }
    }
}
```

---

## 七、接进本项目

### 7.1 现状

```
src/tools.rs                         → tools() -> Vec<ChatCompletionTools>（聚合两个工具的声明）
src/tools/tool.rs                    → 预留：trait 的落点（目前只有骨架）
src/tools/<tool>/definition.rs       → 每个工具返回自己的 ChatCompletionTools
src/tools/<tool>/execute.rs          → 每个工具真正的执行逻辑 + 参数 struct
```

`tools()` 已经能给请求填 `tools` 字段，**缺的正是「按 tool_call 的名字分发到对应 execute」这一环**——这就是 `Tool` trait 的位置。

### 7.2 trait 定义（放 `src/tools/tool.rs`）

```rust
use async_openai::types::chat::ChatCompletionTools;
use async_trait::async_trait;

#[async_trait]
pub trait Tool: Send + Sync {
    /// 必须与 definition 里的 name 一致，也是分发时的键
    fn name(&self) -> &'static str;

    /// 复用各自的 definition.rs，避免重复写 JSON Schema
    fn definition(&self) -> ChatCompletionTools;

    /// 模型给的 arguments 已解析成 Value；返回给模型的字符串
    async fn call(&self, args: serde_json::Value) -> anyhow::Result<String>;
}
```

### 7.3 两个 impl

```rust
use anyhow::Context;

pub struct Calculator;

#[async_trait]
impl Tool for Calculator {
    fn name(&self) -> &'static str { "calculator" }

    fn definition(&self) -> ChatCompletionTools {
        crate::tools::calculator::definition::calculator_tool_definition()
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<String> {
        let a: crate::tools::calculator::execute::CalculatorArgs =
            serde_json::from_value(args).context("calculator 参数不合法")?;
        // 注意：calculator() 的错是 String，要转成 anyhow
        let v = crate::tools::calculator::execute::calculator(&a.operator, a.first_number, a.second_number)
            .map_err(anyhow::Error::msg)?;
        Ok(v.to_string())
    }
}
```

```rust
pub struct WebSearch;

#[async_trait]
impl Tool for WebSearch {
    fn name(&self) -> &'static str { "web_search" }

    fn definition(&self) -> ChatCompletionTools {
        crate::tools::web_search::definition::web_search_tool_definition()
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<String> {
        let a: crate::tools::web_search::execute::WebSearchArgs =
            serde_json::from_value(args).context("web_search 参数不合法")?;
        let out = crate::tools::web_search::execute::search_web(a).await?;
        Ok(serde_json::to_string(&out)?)
    }
}
```

### 7.4 注册表 + 分发

```rust
use std::collections::HashMap;
use std::sync::Arc;
use async_openai::types::chat::{ChatCompletionMessageToolCall, ChatCompletionTools};

pub struct Tools {
    inner: HashMap<&'static str, Arc<dyn Tool>>,
}

impl Tools {
    pub fn new() -> Self {
        let mut inner: HashMap<&'static str, Arc<dyn Tool>> = HashMap::new();
        let list: Vec<Arc<dyn Tool>> = vec![Arc::new(Calculator), Arc::new(WebSearch)];
        for t in list {
            inner.insert(t.name(), t);
        }
        Self { inner }
    }

    /// 替换现在 tools.rs 里的 tools()
    pub fn definitions(&self) -> Vec<ChatCompletionTools> {
        self.inner.values().map(|t| t.definition()).collect()
    }

    /// 把模型点的"单"执行掉
    pub async fn dispatch(&self, call: &ChatCompletionMessageToolCall) -> anyhow::Result<String> {
        let tool = self.inner.get(call.function.name.as_str())
            .ok_or_else(|| anyhow::anyhow!("未知工具: {}", call.function.name))?;
        let args: serde_json::Value = serde_json::from_str(&call.function.arguments)
            .with_context(|| format!("{} 的 arguments 不是合法 JSON", call.function.name))?;
        tool.call(args).await
    }
}
```

### 7.5 和请求/响应接起来

```rust
// 发请求：tools 字段
let tools = Tools::new();
let request = CreateChatCompletionRequestArgs::default()
    .model(model)
    .messages(message.clone())
    .tools(tools.definitions())
    .build()?;

// 收响应：模型要调工具时
if let Some(calls) = choice.message.tool_calls {
    for call in calls {
        let ChatCompletionMessageToolCalls::Function(c) = call else { continue };
        let result = tools.dispatch(&c).await?;
        // 然后 push 一条 assistant(tool_calls) + 一条 tool(tool_call_id = c.id, content = result)
    }
}
```

⚠️ 本项目：`response_format = JsonObject`（`solver.rs`）那种"强制吐 JSON"的函数，**不要和 `tools` 混在同一个请求里**——模型会只顾着满足 JSON 约束而不发 tool_call。工具循环单独用一个不带 `response_format` 的函数。

---

## 八、常见报错对照

| 报错 | 原因 | 解决 |
|---|---|---|
| `expected 'Pin<Box<...>>', found 'impl Future'` | impl 上漏了 `#[async_trait]` | 补上，两侧成对 |
| `future cannot be sent between threads safely` | future 捕获了非 `Send` 值（`Rc`、`RefCell` 跨 await） | `#[async_trait(?Send)]`，或换 `Arc<Mutex<..>>` |
| `the trait X is not dyn compatible ... because method is async` | 用了原生 `async fn` 却要求 `dyn` | 加 `#[async_trait]` |
| `the trait X is not dyn compatible ... because method has generic type parameters` | `dyn` 里放了泛型方法 | 去掉泛型 / 改单态调用 |
| `borrowed value does not live long enough` | future 借了局部变量跨 await（宏只管加约束，借用检查照旧） | 提前把数据取出来/`clone` |
| `tokio::spawn` 报 future 不是 `'static` | `&self` 借用活不够长 | `Arc::clone` 后 `move` 进任务 |

---

## 九、速查

```rust
// 1. 加依赖
// async-trait = "0.1"

// 2. trait 标注
#[async_trait]
pub trait Tool: Send + Sync { async fn call(&self, a: Value) -> Result<String>; }

// 3. 每个 impl 标注
#[async_trait] impl Tool for X { async fn call(&self, a: Value) -> Result<String> { .. } }

// 4. 需要 dyn
let t: Arc<dyn Tool> = Arc::new(X);
let s: String = t.call(v).await?;

// 5. 单线程 / 非 Send
#[async_trait(?Send)] pub trait T { async fn f(&self); }

// 6. 泛型方法不能进 dyn，这是 dyn 的规则，不是宏的
```

一句话总结：**`async-trait` = 把 trait 里的 `async fn` 包成 `Pin<Box<dyn Future + Send>>` 的代码生成器**；代价是每次调用一次堆分配，收益是 `dyn` 可用 + future 自动 `Send`。工具集还会变、需要运行时注册时用它；等工具固定又想省掉分配，再换 `enum` 分发。

---

## 与本系列其它文档的关系

- 与 [`06-futures-and-stream.md`](./06-futures-and-stream.md)：`Pin<Box<dyn Future>>` 就是那边讲的 `BoxFuture`，`async-trait` 只是自动帮你包；
- 与 [`07-backon.md`](./07-backon.md)：trait 方法里同样可以 `.retry(...)`，因为返回的还是普通 future；
- 与 [`05-related-crates.md`](./05-related-crates.md)：`async-openai` 的 `ChatCompletionTools` / `ChatCompletionMessageToolCall` 是这套工具抽象的输入输出类型。
