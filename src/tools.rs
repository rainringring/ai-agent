use std::{collections::HashMap, sync::Arc};

use async_openai::types::chat::ChatCompletionTools;

use crate::tools::{
    calculator::{definition::calculator_tool_definition, r#impl::CalculatorTool},
    mcp::{client::McpClient, tool::McpTool},
    tool::Tool,
    web_search::{definition::web_search_tool_definition, r#impl::WebSearchTool},
};

pub mod calculator;
pub mod mcp;
pub mod tool;
pub mod web_search;

pub type ToolBox = HashMap<String, Box<dyn Tool>>;

pub async fn build_tool_box() -> anyhow::Result<ToolBox> {
    let mut tools: Vec<Box<dyn Tool>> = vec![Box::new(CalculatorTool), Box::new(WebSearchTool)];

    let mcp_client = Arc::new(McpClient::connect().await?);
    for tool in mcp_client.list_tools().await? {
        tools.push(Box::new(McpTool::new(mcp_client.clone(), tool)));
    }

    Ok(tools
        .into_iter()
        .map(|t| (t.name().to_string(), t))
        .collect())
}
