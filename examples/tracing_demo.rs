//! tracing 分步实验：一次只演示一个概念。
//!
//! 用法（按顺序跑一遍，对比输出）：
//!
//!   cargo run --example tracing_demo -- 1   # 没有 subscriber → 一条都不打印
//!   cargo run --example tracing_demo -- 2   # 装上 subscriber → 但只剩 info 及以上
//!   cargo run --example tracing_demo -- 3   # 把级别调到 DEBUG → debug 也出来了
//!   cargo run --example tracing_demo -- 4   # 结构化字段长什么样
//!   cargo run --example tracing_demo -- 5   # span：给日志加上下文前缀
//!
//! 关键前提：全局 subscriber 一个进程只能装一次，所以这里用命令行参数
//! 让每个步骤各跑一次独立进程。这也正是真实程序的形态——
//! 程序启动时装一次，之后全程复用。

use tracing::{debug, error, info, instrument, warn};

fn main() {
    let step: u32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    match step {
        // ── 步骤 1：最核心的困惑 ──────────────────────────────
        // 什么初始化都不做，直接打日志。
        1 => {
            println!("--- 没有初始化 subscriber ---");
            emit_all();
            println!("--- 结束：注意到上面一条日志都没有 ---");
        }

        // ── 步骤 2：装上 subscriber，但只放行 INFO 及以上 ──────
        2 => {
            init_subscriber(tracing::Level::INFO);
            emit_all();
        }

        // ── 步骤 3：把门槛降到 DEBUG ─────────────────────────
        3 => {
            init_subscriber(tracing::Level::DEBUG);
            emit_all();
        }

        // ── 步骤 4：字段 vs 消息 ──────────────────────────────
        4 => {
            init_subscriber(tracing::Level::DEBUG);
            show_fields();
        }

        // ── 步骤 5：span 的上下文继承 ─────────────────────────
        5 => {
            init_subscriber(tracing::Level::INFO);
            show_spans();
        }

        _ => {
            println!("用法: cargo run --example tracing_demo -- <1|2|3|4|5>");
        }
    }
}

/// 唯一的初始化点。真实项目里这个函数只在 main 开头调一次。
///
/// - `with_max_level`：静态门槛，低于它的日志根本不产生
/// - `with_ansi(false)`：关掉颜色，方便复制到文档里对比
/// - `with_target(true)`：显示日志来自哪个模块（默认就开）
fn init_subscriber(level: tracing::Level) {
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_ansi(false)
        .with_target(true)
        .init();
}

/// 五个级别各打一条。级别从高到低：ERROR WARN INFO DEBUG TRACE
fn emit_all() {
    error!(order_id = 1, "这是 error");
    warn!(user_id = 7, "这是 warn");
    info!(port = 8080, "这是 info");
    debug!(sql = "SELECT 1", "这是 debug");
    tracing::trace!("这是 trace");
}

/// 同一句话，用不同方式携带数据。
fn show_fields() {
    // 1) 裸字段：值必须实现 tracing::Value（数字/字符串/bool）
    info!(order_id = 42, amount = 99.5, currency = "CNY", "下单成功");

    // 2) % 前缀 = 用 Display 格式化后当字段
    info!(status = %404, "请求完成");

    // 3) ? 前缀 = 用 Debug 格式化后当字段
    debug!(payload = ?vec![1, 2, 3], "收到数据");

    // 4) 对照：把数据拼进消息里（错误示范）
    //    这样它是"文本的一部分"，机器读不出来，也没法按字段过滤
    let order_id = 42;
    info!("下单成功 order_id={}", order_id);
}

/// span：一段代码的标签，其内部所有日志自动继承。
fn show_spans() {
    // 手动 span：进入作用域后，里面打的日志都会带上前缀
    let span = tracing::info_span!("request", request_id = 1001);
    let _guard = span.enter();

    info!("收到请求");

    // 这里调用 #[instrument] 的函数，会再嵌一层子 span
    let result = handle_order(42);

    info!(result, "请求处理完毕");
    // _guard 在这里 drop，span 结束
}

/// #[instrument] 自动创建 span，名字是函数名，参数自动变成字段。
#[instrument]
fn handle_order(order_id: u64) -> u64 {
    info!("开始处理");
    order_id * 2
}
