use anyhow::Ok;
use async_openai::types::chat::{ChatCompletionTool, ChatCompletionTools, FunctionObjectArgs};
use serde_json::Value;

use crate::agent::ExecutionContext;

#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;

    fn description(&self) -> &str;

    fn parameters(&self) -> Value;

    async fn execute(&self, args_json: &str, context: &ExecutionContext) -> anyhow::Result<String>;

    fn definition(&self) -> anyhow::Result<ChatCompletionTools> {
        let function = FunctionObjectArgs::default()
            .name(self.name())
            .description(self.description())
            .parameters(self.parameters())
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build tool definition for {}:", self.name()))?;

        Ok(ChatCompletionTools::Function(ChatCompletionTool {
            function,
        }))
    }
}
