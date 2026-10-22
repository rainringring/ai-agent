use std::{sync::{Arc, Mutex}, time::Duration};

use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Expense {
    id: String,
    amount_cents: i64,
    category: String,
    note: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct AddExpenseArgs {
    /// 金额，单位：分
    amount_cents: i64,
    category: String,
    #[serde(default)]
    note: String,
}

/// 注意：#[tool_router] 要求结构体有 ToolRouter<Self> 字段且可 Clone
#[derive(Clone)]
struct ExpenseServer {
    tool_router: ToolRouter<Self>,
    expenses: Arc<Mutex<Vec<Expense>>>,
}

#[tool_router]
impl ExpenseServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
            expenses: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// 记录一笔支出
    #[tool]
    async fn add_expense(&self, Parameters(args): Parameters<AddExpenseArgs>) -> Result<String, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let item = Expense {
            id: id.clone(),
            amount_cents: args.amount_cents,
            category: args.category,
            note: args.note,
        };
        self.expenses.lock().unwrap().push(item);
        Ok(format!("recorded {id}"))
    }

    /// 列出所有支出
    #[tool]
    async fn list_expenses(&self) -> Result<String, String> {
        let list = self.expenses.lock().unwrap().clone();
        serde_json::to_string(&list).map_err(|e| e.to_string())
    }
}

#[tool_handler]
impl ServerHandler for ExpenseServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("记账工具：add_expense / list_expenses")
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ⚠️ stdio 传输下 stdout 是协议通道，日志必须走 stderr
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing::Level::INFO)
        .init();

    let server = ExpenseServer::new()
        .serve(rmcp::transport::stdio())
        .await?;
    server.waiting().await?;
    Ok(())
}