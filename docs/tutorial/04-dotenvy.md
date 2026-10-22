# 04 · dotenvy —— 从 `.env` 加载环境变量

> 版本：`0.15.7`（feature 只有一个 `cli`，即 `dotenv` 命令行工具）

## 一、它解决什么问题，边界在哪

**解决的**：开发时不想把 `OPENAI_API_KEY=sk-xxx` 硬编码进代码，也不想每次都 `set-env` 一下。把值放 `.env` 文件，程序启动时读进进程环境变量，代码里统一用 `std::env::var` 取。

**不解决的**：

| 不要指望它 | 原因 |
|---|---|
| 在生产环境当配置中心 | 生产应该用真正的环境变量（容器 env、systemd、K8s Secret） |
| 提供类型/校验 | 它只做 `String → 环境变量`，全部是字符串 |
| 热更新 | 只在调用时读一次，改文件要重启 |
| 安全 | `.env` 是明文，权限就是文件系统权限 |

一句话：**`.env` 是开发期的便利，不是配置系统**。需要类型化、分层、校验的配置，用 `config` / `figment` 那类 crate。

---

## 二、核心 API

`dotenvy` 的 API 极少，全部是自由函数：

| 函数 | 返回 | 说明 |
|---|---|---|
| `dotenv()` | `Result<PathBuf>` | 从**当前目录向上**找 `.env` 并加载 |
| `dotenv_override()` | `Result<PathBuf>` | 同上，但**覆盖**已有环境变量 |
| `dotenv_iter()` | `Result<Iter<File>>` | 不写入环境，只逐行迭代 |
| `from_path(p)` | `Result<()>` | 加载指定路径 |
| `from_path_override(p)` | `Result<()>` | 同上 + 覆盖 |
| `from_filename(name)` | `Result<PathBuf>` | 从当前目录向上找**指定文件名** |
| `from_filename_override(name)` | `Result<PathBuf>` | 同上 + 覆盖 |
| `from_read(reader)` | `Result<()>` | 从任意 `Read` 加载（测试友好） |
| `from_read_iter(reader)` | `Iter<R>` | 迭代而不写入 |
| `var(key)` | `Result<String>` | 读一个变量（**等价于 `std::env::var`**，只是错误类型统一） |
| `vars()` | `Vars` | 迭代全部变量（跟 `std::env::vars()` 一样是**全进程**的） |

⚠️ `var()` / `vars()` 读的是**进程环境**，不是只读 `.env` 文件。因此必须先成功调过 `dotenv()`，否则读不到。

### 最小的正确用法

```rust
use anyhow::{Context, Result};

fn main() -> Result<()> {
    // 1) 最先做：把 .env 灌进进程环境
    dotenvy::dotenv().ok();          // 没有 .env 也不算致命，用 ok() 忽略

    // 2) 之后 std::env 或 dotenvy::var 都能读到
    let key = std::env::var("OPENAI_API_KEY")
        .context("OPENAI_API_KEY 未设置（检查 .env 或环境变量）")?;

    println!("key 长度 = {}", key.len());
    Ok(())
}
```

**顺序是硬要求**：任何 `std::env::var` 必须在 `dotenv()` 之后。这也包括 `async-openai::Client::new()`——它在内部读 `OPENAI_API_KEY`，所以 `dotenv()` 必须写在建 Client 之前。

---

## 三、覆盖语义：「不覆盖已有值」是特性，不是 bug

```rust
// 环境里已有 FOO=shell
dotenvy::dotenv()?;               // .env 里 FOO=file → 最终仍是 "shell"
dotenvy::dotenv_override()?;      // .env 里 FOO=file → 最终是 "file"
```

**默认不覆盖**的设计意图：让**真实环境变量优先于文件**。这样部署时 `FOO=prod cargo run` 能盖掉 `.env` 里的开发值，符合 12-factor 的优先级直觉：

```
进程环境（最高）
   ↑
.env 文件
   ↑
代码里的默认值（最低）
```

需要反过来（比如测试里强制用文件值）才用 `_override` 版本。

---

## 四、`.env` 文件语法

```dotenv
# 注释以 # 开头
OPENAI_API_KEY=sk-abc123

# 值里有空格或特殊字符，用引号
GREETING="hello world"
SINGLE='no expansion here'

# export 前缀是允许的（从 shell 脚本抄过来不会坏）
export MODE=dev

# 变量插值：引号内会展开 ${VAR}
BASE_URL=https://api.example.com
CHAT_URL=${BASE_URL}/v1/chat

# 空值
OPTIONAL_FLAG=
```

支持的语法（已对着 `dotenvy 0.15.7` 的 `parse.rs` 核实）：

- `#` 注释（整行或以空格开头）
- 单/双引号包裹含空格的值
- `export` 前缀
- `${VAR}` 插值（双引号内展开，单引号内不展开）
- 空值 `KEY=`

⚠️ **不支持**的常见写法：

- 嵌套/条件/多行块（不是 shell，别写 `if`）
- 逗号数组（`ARR=a,b,c` 只会得到一个字符串 `"a,b,c"`，要自己 `split(',')`）
- 类型（`PORT=8080` 读出来是 `"8080"`，`parse::<u16>()` 要自己做）

---

## 五、向上查找的机制（一个很实用的坑）

`dotenvy::dotenv()` 不是"读当前目录的 `.env`"，而是**从当前工作目录开始，逐级向上找到根**，用第一个找到的。

源码逻辑就是递归 `parent()`：

```
E:\rust-agent\ai-agent\   ← 当前目录，找 .env
E:\rust-agent\            ← 没找到，上一层
E:\                       ← 没找到，再上一层
```

**这带来一个实际影响**：`cargo run` 的 CWD 是包根目录，`cargo test` 的 CWD 也是包根目录，但 **IDE 的运行配置、或者你从别的目录起二进制时，CWD 可能不是包根**。于是同一个程序可能读到不同层级的 `.env`，或者一个都读不到。

排查这类"明明有 .env 却读不到"的问题，第一件事是打印 CWD：

```rust
println!("cwd = {:?}", std::env::current_dir()?);
println!("dotenv = {:?}", dotenvy::dotenv());
```

`dotenv()` 返回的 `PathBuf` 就是**实际加载的那个文件**，直接打印它最快定位。

### 想要确定性：用绝对路径

```rust
dotenvy::from_path("./.env").ok();          // 相对 CWD，仍不确定
dotenvy::from_filename(".env.local").ok();  // 也向上找

// 最确定的方式：自己算路径
let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env");
dotenvy::from_path(path).ok();
```

`env!("CARGO_MANIFEST_DIR")` 在编译期展开成包根目录的绝对路径，运行时不受 CWD 影响——**排查配置问题时非常好用**。

---

## 六、常见操作

### 1. 可选加载（没有 `.env` 也正常跑）

```rust
match dotenvy::dotenv() {
    Ok(path) => println!("已加载 {:?}", path),
    Err(e) => println!("未加载 .env（{e}），仅使用现有环境变量"),
}
```

注意缺文件时 `dotenv()` 返回的**不是 panic，而是 `Err(Error::Io(NotFound))`**。用 `.ok()` 静默忽略很常见，但在排查阶段建议打印出来。

### 2. 读回并带类型转换

```rust
use anyhow::{Context, Result};

fn env_u16(key: &str, default: u16) -> Result<u16> {
    match std::env::var(key) {
        Ok(v) => v.parse::<u16>()
            .with_context(|| format!("{key}={v} 不是合法端口")),
        Err(_) => Ok(default),
    }
}
```

`dotenvy::var("X")` 也一样，只是把 `VarError` 换成了 `dotenvy::Error`。**用哪个都行，`std::env::var` 更通用**（生产环境没有 dotenvy 也能跑）。

### 3. 测试隔离

测试里不要读真实 `.env`（同事机器上值不同），用字面量喂进去：

```rust
#[test]
fn parses_env() {
    let content = b"PORT=9999\nMODE=test\n";
    dotenvy::from_read(&content[..]).ok();   // 或 from_read_iter 不污染进程环境
}
```

`from_read_iter` 更好：**不写入进程环境**，避免测试之间互相干扰（测试默认并行，改环境变量是全局副作用）。

```rust
use dotenvy::from_read_iter;

let vars: Vec<_> = from_read_iter(b"PORT=9999\nMODE=test\n".as_slice())
    .filter_map(Result::ok)
    .collect();
```

### 4. 加载多个文件（分层配置）

```rust
// 先公共，后覆盖
dotenvy::from_filename(".env").ok();
dotenvy::from_filename_override(".env.local").ok();
```

注意第一个用非 override（不盖真实环境变量），第二个用 override（本地文件优先）——这个组合正好实现"`.env.local` 覆盖 `.env`"。

---

## 七、坑位清单

| 坑 | 说明 |
|---|---|
| **在 `dotenv()` 之前读变量** | 读到 `None`。async-openai 的 `Client::new()` 尤其容易踩，它建构造时就读了 `OPENAI_API_KEY` |
| **`.env` 提交进 git** | 密钥泄露。务必写进 `.gitignore` |
| **改了 `.env` 没生效** | 只在启动时读一次，必须重启进程 |
| **以为会覆盖已有环境变量** | 默认不覆盖。需要覆盖用 `dotenv_override` |
| **CWD 不确定** | `dotenv()` 向上查找，IDE 启动时可能找到别的 `.env`。用 `env!("CARGO_MANIFEST_DIR")` 定死 |
| **值带引号还想保留引号** | 引号会被剥掉，这是预期行为 |
| **数字直接当数字用** | 读出来永远是 `String`，要 `parse` |
| **在库代码里调 `dotenv()`** | 全局副作用，库不该控制调用方的环境。只在你自己的二进制入口调 |
| **重复调用** | 幂等，不报错，但没必要，别放循环里 |
| **找编译期 `dotenv!` 宏** | `0.15.7` 没有（feature 只有 `cli`）。要编译期嵌入得用 `dotenvy_macro` 或换 crate 版本 |
| **生产环境依赖 `.env`** | 容器里通常不挂 `.env`，代码要能在没有它时正常启动 |

---

## 八、和其他 crate 的配合顺序

正确的启动顺序（**这个顺序错了就会出现"key 明明在 `.env` 里却读不到"**）：

```rust
use anyhow::{Context, Result};

#[tokio::main]
async fn main() -> Result<()> {
    // 1) 先加载 .env —— 必须最早，因为后面可能有人读环境变量
    dotenvy::dotenv().ok();

    // 2) 再初始化日志 —— 这样第 1 步之后的一切都能被记录
    init_tracing();

    // 3) 然后才是读配置、建客户端
    let key = std::env::var("OPENAI_API_KEY")
        .context("OPENAI_API_KEY 未设置")?;
    tracing::info!(key_len = key.len(), "凭据已加载");

    // 4) 建 OpenAI 客户端（内部读 OPENAI_API_KEY / OPENAI_BASE_URL）
    let client = async_openai::Client::new();

    Ok(())
}
```

为什么第 1 步和第 2 步有先后：`dotenv()` 几乎不会失败到需要日志，反过来日志初始化可能想读 `RUST_LOG`——而 `RUST_LOG` 也可以写在 `.env` 里。**先 dotenv 再 tracing，`RUST_LOG` 才能从文件生效。**

---

## 九、`.gitignore` 与 `.env.example`

推荐做法：

```gitignore
.env
.env.local
.env.*.local
```

同时提交一份 `.env.example` 说明需要哪些变量：

```dotenv
# 复制为 .env 并填值
OPENAI_API_KEY=
OPENAI_BASE_URL=https://api.openai.com/v1
RUST_LOG=info,ai_agent=debug
```

这样新人 `cp .env.example .env` 就知道该填什么，而真实密钥永远不进仓库。

**下一步**：去看 [`05-related-crates.md`](./05-related-crates.md)，把 tokio 和 async-openai 也串进来。
