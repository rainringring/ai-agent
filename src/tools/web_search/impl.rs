use async_openai::types::chat::{ChatCompletionTool, ChatCompletionTools};
use schemars::schema_for;

use crate::tools::{tool::Tool, web_search::{execute::{WebSearchArgs, search_web}}};

use crate::agent::ExecutionContext;

pub struct WebSearchTool;

#[async_trait::async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web for current information on a given query."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::to_value(schema_for!(WebSearchArgs)).expect("Failed to serialize WebSearchArgs schema")
    }

    async fn execute(&self, args_json: &str, _context: &ExecutionContext) -> anyhow::Result<String> {
        let args: WebSearchArgs = serde_json::from_str(args_json)?; 
        match search_web(args).await {
            Ok(output) => Ok(serde_json::to_string(&output)?),
            Err(err) => Ok(format!("Error: {err}")),
        }
    }
}
