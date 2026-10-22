# 01 · anyhow —— 应用层错误处理

> 版本：`1.0.104`（`Cargo.toml` 写 `1.0.102`，caret 规则升上来的）

## 一、它到底解决什么问题

Rust 标准库的错误处理有个著名的分裂：

- **库**应该暴露具体错误类型（`io::Error`、`ParseIntError`……），调用方才能针对性处理；
- **应用**（`main`、handler、业务逻辑）不关心错误有几种，只关心"哪一步失败了、为什么失败"。

如果你有 8 个函数、每个都可能返回 3 种错误类型，把它们串起来会变成这样：

```rust
// 没有 anyhow 的典型写法：要么层层自定义 enum，要么 map_err 到手软
fn load() -> Result<Config, MyError> {
    let text = std::fs::read_to_string("config.json").map_err(|e| MyError::Io(e.to_string()))?;
    let cfg = serde_json::from_str(&text).map_err(|e| MyError::Parse(e.to_string()))?;
    Ok(cfg)
}
```

问题不是难写，而是**信息在退化**：`MyError::Io(String)` 丢了原始错误类型，也丢了"是读 config.json 时失败的"这个上下文。

`anyhow` 给的方案是**类型擦除 + 上下文链**：

```rust
use anyhow::{Context, Result};

fn load() -> Result<Config> {
    let text = std::fs::read_to_string("config.json")
        .with_context(|| "读取 config.json 失败")?;
    let cfg = serde_json::from_str(&text)
        .with_context(|| "解析 config.json 失败")?;
    Ok(cfg)
}
```

现在错误打印出来是：

```
解析 config.json 失败

Caused by:
    expected value at line 3 column 12
```

原始错误保留了，人读的上下文也有了，函数签名还只有一个错误类型。

---

## 二、三个核心概念

| 概念 | 是什么 |
|---|---|
| `anyhow::Error` | 一个**类型擦除**的错误包装。内部装着 `Box<dyn Error + Send + Sync + 'static>` + 上下文链 + 可选回溯 |
| `anyhow::Result<T>` | `type Result<T, E = anyhow::Error> = std::result::Result<T, E>;` 的别名，只写一个泛型参数 |
| `anyhow::Context` | 一个 **trait**，给 `Result` 和 `Option` 加 `.context()` / `.with_context()` 方法 |

因为 `anyhow::Result<T>` 的 `E` 有默认值，你可以写 `Result<T>` 也能写 `Result<T, anyhow::Error>`，两者等价。

---

## 三、最小可用骨架

```rust
use anyhow::{Context, Result};

fn main() -> Result<()> {
    let port = std::env::var("PORT")
        .context("环境变量 PORT 未设置")?
        .parse::<u16>()
        .context("PORT 不是合法端口号")?;

    println!("listening on {port}");
    Ok(())   // 成功路径
}
```

要点：

- `main` 返回 `Result<()>` 时，`Err` 会被自动 `Debug` 打印到 stderr，**进程退出码变成 1**。
- `Debug` 打印会包含 `Caused by:` 链和回溯（若开启），比你自己 `eprintln!("{e}")` 信息多得多。
- `?` 能工作的前提：被转换的错误类型实现了 `std::error::Error + Send + Sync + 'static`。`anyhow` 为这类类型提供了 `impl From<E> for Error`。

---

## 四、加上下文：`context` vs `with_context`

两者都在 `Context` trait 上，都同时实现了 `Result` 和 `Option`：

| 方法 | 参数 | 什么时候用 |
|---|---|---|
| `.context("msg")` | 立即求值的 `Display` 值 | 消息是静态字符串，或格式化代价可忽略 |
| `.with_context(|| format!("..."))` | 闭包，惰性求值 | 消息里要拼接变量、或格式化有开销 |

**成功路径不会调用这段格式化**——这是 `with_context` 存在的全部意义。别小看这一点：在热路径里、或者错误很少发生但消息要 `format!` 大对象时，`context` 会白白付出代价。

```rust
use anyhow::Context;

// 静态消息：context 就够
let cfg = std::fs::read_to_string(path).context("读不到配置文件")?;

// 消息里带变量：用 with_context，避免成功时也做格式化
let cfg = std::fs::read_to_string(&path)
    .with_context(|| format!("读不到配置文件: {}", path.display()))?;
```

### 对 `Option<T>` 用

`Option` 的 `None` 不带任何信息，`.context()` 能把它变成带人话的错误：

```rust
use anyhow::{Context, Result};

fn find_user(id: u32) -> Result<String> {
    let name = USERS
        .iter()
        .find(|u| u.id == id)
        .map(|u| u.name.clone())
        .with_context(|| format!("找不到 id={id} 的用户"))?;
    Ok(name)
}
```

这是 `anyhow` 最常用的操作之一：**把 `Option` 链尾的 `?` 变成可诊断的错误**，而不是到处 `ok_or_else`。

---

## 五、快速失败：`bail!` 和 `ensure!`

这两个宏省掉 `return Err(anyhow!(...))` 的样板。**注意它们是"返回"语义，所以只能在返回 `Result` 的函数里用**。

```rust
use anyhow::{bail, ensure, Result};

fn withdraw(balance: i64, amount: i64) -> Result<i64> {
    ensure!(amount > 0, "取款金额必须为正，收到 {amount}");
    ensure!(amount <= balance, "余额不足：余额 {balance}，取款 {amount}");
    if balance - amount > 1_000_000 {
        bail!("单笔余额上限超限，请人工处理");
    }
    Ok(balance - amount)
}
```

- `ensure!(cond, "msg")` = `if !cond { bail!("msg") }`
- `bail!("msg")` = `return Err(anyhow!("msg"))`
- 两者都支持 `format!` 风格参数。

### 四个构造宏/函数对比

| 写法 | 用途 |
|---|---|
| `anyhow!("msg {x}")` | 从无到有造一个错误 |
| `anyhow!(some_err)` | 把已有的具体错误包进来（保留类型，可下转型） |
| `Error::msg("static")` | 静态字符串，等价 `anyhow!` 但显式 |
| `bail!` / `ensure!` | 提前返回上述任一 |

---

## 六、读错误链：四种打印方式

这是排查问题的关键，`anyhow` 的收益几乎全在这里。

```rust
let e: anyhow::Error = /* ... */;
```

| 写法 | 输出 | 用在 |
|---|---|---|
| `{}` (Display) | 只有**最外层**消息 | 给终端用户看 |
| `{:#}` | 整条链用 `: ` 连接成一行 | 日志单行输出（最实用） |
| `{:?}` (Debug) | 多行，带 `Caused by:` 缩进 + 回溯 | `main` 返回 `Err` 时默认效果、开发者看 |
| `{:#?}` | 更详细的 Debug 结构 | 极少用 |

示例输出对比：

```rust
// 假设链路是：读文件失败 ← 打开 config.json 失败
println!("{}", e);      // 打开 config.json 失败
println!("{:#}", e);    // 打开 config.json 失败: No such file or directory (os error 2)
println!("{:?}", e);    // 打开 config.json 失败
                        //
                        // Caused by:
                        //     No such file or directory (os error 2)
```

**建议：日志里用 `{:#}`，`main` 直接返回 `Err` 用默认 Debug。**

### 程序化遍历链

```rust
use std::error::Error as StdError;

// 从最外层到根因
for cause in e.chain() {
    println!("- {cause}");
}

// 只要根因
let root: &(dyn StdError + 'static) = e.root_cause();
```

`chain()` 返回 `impl Iterator<Item = &(dyn Error + 'static)>`，第一项就是 `e` 本身。写自定义错误上报、或者想把整条链序列化成 JSON 时用得到。

---

## 七、类型擦除之后怎么拿回原类型

擦除不等于丢失——`anyhow::Error` 保留了下转型能力：

```rust
use anyhow::Result;

fn run() -> Result<()> {
    match std::fs::read_to_string("x") {
        Err(e) => {
            // 判断是不是某种具体错误
            if e.is::<std::io::Error>() {
                // ...
            }
            // 借用式下转型
            if let Some(io) = e.downcast_ref::<std::io::Error>() {
                println!("io kind = {:?}", io.kind());
            }
            // 所有权式下转型（消耗 e）
            let io: std::io::Error = e.downcast::<std::io::Error>()?;
            println!("{:?}", io.kind());
            return Ok(());
        }
        Ok(_) => {}
    }
    Ok(())
}
```

| 方法 | 签名特点 | 用途 |
|---|---|---|
| `e.is::<T>()` | `T: Display + Debug + Send + Sync + 'static` | 只判断类型 |
| `e.downcast_ref::<T>()` | 返回 `Option<&T>` | 看一眼，不消耗 |
| `e.downcast::<T>()` | 返回 `Result<T, Error>` | 拿所有权处理 |

⚠️ **注意下转型只看最外层还是整条链**：`downcast_ref` 会**遍历整条上下文链**去找 `T`。所以即使 `io::Error` 被加了 5 层 `context`，`downcast_ref::<io::Error>()` 依然能拿到。这一点很反直觉但非常有用。

---

## 八、什么时候**不该**用 anyhow

这是最容易被误用的一点。

**库的 public API 不要返回 `anyhow::Error`。**

理由：

1. **调用方无法穷举处理**。类型擦除后，使用者只能下转型猜，无法 `match` 出全部错误变体。
2. **你不知道新增错误会不会破坏别人**。具体错误类型是 API 契约的一部分。
3. **`anyhow::Error` 不能 `Clone`、不能 `PartialEq`**，测试里没法断言相等。

正确分工：

| 场景 | 用什么 |
|---|---|
| `main`、CLI、HTTP handler、业务编排 | `anyhow` |
| 库 / 可复用模块的公开 API | `thiserror` 定义具体 enum |
| 边界处（库错误 → 应用错误） | `#[from]` 转换，或 `?` + `context` |

```rust
// 库层：具体类型
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("数据库连接失败")]
    Connect(#[source] sqlx::Error),
    #[error("记录 {id} 不存在")]
    NotFound { id: u64 },
}

// 应用层：擦除成 anyhow，加业务上下文
fn handler(id: u64) -> anyhow::Result<()> {
    let rec = store.get(id).context("查询订单时出错")?;
    Ok(())
}
```

⚠️ **反向转换是行不通的**：`anyhow::Error` 不能 `?` 成你的自定义错误类型（没有 `From` 实现）。一旦在库内部用了 anyhow，就在边界上锁死了。所以边界要提前划好。

详见 [`05-related-crates.md`](./05-related-crates.md) 里 thiserror 的部分。

---

## 九、常见操作速查

| 需求 | 写法 |
|---|---|
| 函数返回可带上下文的错误 | `fn f() -> anyhow::Result<T>` |
| 给 `Result` 加说明 | `.context("读配置失败")?` |
| 给 `Option` 加说明 | `.context("缺少字段 x")?` |
| 消息含变量 | `.with_context(\|\| format!("...{x}"))?` |
| 立刻报错返回 | `bail!("...")` |
| 条件不满足就返回 | `ensure!(cond, "...")` |
| 包装已有错误 | `anyhow!(e)` |
| 单行日志打印整条链 | `tracing::error!("{:#}", e)` / `format!("{e:#}")` |
| 判断错误类型 | `e.is::<io::Error>()` |
| 取出原始错误 | `e.downcast::<io::Error>()?` |
| 遍历链 | `e.chain()` |
| 拿到根因 | `e.root_cause()` |
| `main` 里兜底 | `fn main() -> anyhow::Result<()>` |

---

## 十、坑位清单

| 坑 | 说明 |
|---|---|
| **`String` / `&str` 不能直接 `?`** | 它们不是 `Error`。要 `anyhow!("{s}")` 或 `Error::msg(s)` |
| **`context` 丢失原始类型信息？** | 不会。链里的原错误仍在，`downcast_ref` 能穿透整条链找到它 |
| **`map_err` 和 `context` 混用会断链** | `.map_err(\|e\| anyhow!("..."))` 把 `e` 塞进字符串，原错误就没了。改用 `.context()` |
| **`anyhow::Error` 不 `Clone`** | 想保留一份得 `format!("{e:#}")` 或重新构造 |
| **`bail!`/`ensure!` 在返回非 `Result` 的函数里用** | 编译错误；它们展开成 `return Err(..)` |
| **过度加上下文** | 每层都 `context("出错了")` 会得到一串无意义的链。上下文要写**这一层独有的信息**（文件路径、ID、参数值） |
| **回溯默认不开** | 需要 `RUST_BACKTRACE=1`（或 `full`）才会捕获并打印 |

### 关于回溯

在 rustc ≥ 1.65 上，`anyhow` 直接用 `std::backtrace::Backtrace`，不需要额外 feature：

```powershell
$env:RUST_BACKTRACE = 1
cargo run
```

代码里也能显式拿：`e.backtrace()` 返回 `&std::backtrace::Backtrace`。注意**捕获时机是错误构造的那一刻**，所以环境变量必须在进程启动前设好。

---

## 十一、一个完整的"重构前后"对比

**重构前**（信息退化、样板多）：

```rust
fn load() -> Result<Config, String> {
    let text = std::fs::read_to_string("config.json")
        .map_err(|e| format!("读文件失败: {e}"))?;
    let cfg: Config = serde_json::from_str(&text)
        .map_err(|e| format!("解析失败: {e}"))?;
    Ok(cfg)
}
```

问题：错误类型是 `String`，日志里永远只有一行，拿不回 `io::ErrorKind`，也没法区分"文件不存在"和"权限不足"。

**重构后**：

```rust
use anyhow::{Context, Result};

fn load() -> Result<Config> {
    let text = std::fs::read_to_string("config.json")
        .context("读取 config.json 失败")?;
    let cfg: Config = serde_json::from_str(&text)
        .context("解析 config.json 失败")?;
    Ok(cfg)
}
```

收益：

1. `{:#}` 一次打印完整链，排查时不用猜；
2. `downcast_ref::<io::Error>()` 能拿到 `ErrorKind::NotFound`，可以针对性降级（比如首次运行自动建默认配置）；
3. 函数签名干净，任意底层错误都能 `?` 上来。

**下一步**：去看 [`02-serde.md`](./02-serde.md)，`serde_json::from_str` 那一端是怎么定义的。
