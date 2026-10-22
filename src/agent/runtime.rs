use async_openai::types::chat::{
    ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls,
    ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestToolMessageArgs,
    ChatCompletionRequestUserMessageArgs, ChatCompletionTools, CreateChatCompletionRequestArgs,
    FunctionCall,
};

use crate::{
    agent::{ContentItem, Event, ExecutionContext, ToolResultStatus},
    tools::ToolBox,
};

#[derive(Debug)]
pub struct AgentResult {
    pub output: String,
    pub context: ExecutionContext,
}

pub struct Agent<'a> {
    model: &'a str,
    instructions: Option<&'a str>,
    toolbox: &'a ToolBox,
    max_steps: u32,
}

impl<'a> Agent<'a> {
    pub fn new(model: &'a str, instructions: Option<&'a str>, toolbox: &'a ToolBox) -> Self {
        Self {
            model,
            instructions,
            toolbox,
            max_steps: 10, 
        }
    }

    pub fn with_max_steps(mut self, max_steps: u32) -> Self {
        self.max_steps = max_steps;
        self
    }

    pub async fn run(&self, user_input: &str) -> anyhow::Result<AgentResult> {
        let mut context = ExecutionContext::new();

        context.add_event(Event::new(
            context.execution_id.clone(),
            "user",
            vec![ContentItem::Message {
                role: "user".to_string(),
                content: user_input.to_string(),
            }],
        ));

        let client = async_openai::Client::new();

        let tool_definitions: Vec<ChatCompletionTools> = self
            .toolbox
            .values()
            .filter_map(|t| match t.definition() {
                Ok(def) => Some(def),
                Err(e) => {
                    tracing::warn!("Skip tool {}, failed to get its definition: {e}", t.name());
                    None
                }
            })
            .collect();

        loop {
            if context.current_step >= self.max_steps {
                anyhow::bail!(
                    "Agent exceeded the maximum of {} steps without a final answer",
                    self.max_steps
                );
            }

            let messages = self.build_messages(&context)?;

            // 空 tools 数组有的服务端会拒绝，所以非空才设置
            let mut args = CreateChatCompletionRequestArgs::default();
            args.model(self.model)
                .messages(messages)
                .max_tokens(2048u32);
            if !tool_definitions.is_empty() {
                args.tools(tool_definitions.clone());
            }
            let request = args.build()?;

            let response = client.chat().create(request).await?;
            let message = response
                .choices
                .into_iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("No choices in response"))?
                .message;

            // Some(vec![]) 也要当成“没有工具调用”，否则会空转一轮
            if let Some(tool_calls) = message.tool_calls.filter(|calls| !calls.is_empty()) {
                self.record_tool_calls(&mut context, &tool_calls);
                self.execute_tool_calls(&mut context, &tool_calls).await?;
            } else {
                let content = message
                    .content
                    .ok_or_else(|| anyhow::anyhow!("No content in final response"))?;

                context.add_event(Event::new(
                    context.execution_id.clone(),
                    "agent",
                    vec![ContentItem::Message {
                        role: "assistant".to_string(),
                        content: content.clone(),
                    }],
                ));
                context.final_result = Some(content.clone());
                return Ok(AgentResult {
                    output: content,
                    context,
                });
            }

            context.increment_step();
        }
    }

    fn record_tool_calls(
        &self,
        context: &mut ExecutionContext,
        tool_calls: &[ChatCompletionMessageToolCalls],
    ) {
        let mut call_items = Vec::new();
        for tool_call in tool_calls {
            if let ChatCompletionMessageToolCalls::Function(function_call) = tool_call {
                // 解析失败时保留原始字符串，避免把参数悄悄变成 null
                let arguments: serde_json::Value =
                    serde_json::from_str(&function_call.function.arguments).unwrap_or_else(|_| {
                        serde_json::Value::String(function_call.function.arguments.clone())
                    });
                call_items.push(ContentItem::ToolCall {
                    tool_call_id: function_call.id.clone(),
                    name: function_call.function.name.clone(),
                    arguments,
                });
            }
        }
        context.add_event(Event::new(
            context.execution_id.clone(),
            "agent",
            call_items,
        ));
    }

    async fn execute_tool_calls(
        &self,
        context: &mut ExecutionContext,
        tool_calls: &[ChatCompletionMessageToolCalls],
    ) -> anyhow::Result<()> {
        let mut result_items = Vec::new();

        for tool_call in tool_calls {
            // custom tool 目前没有对应的回填逻辑，显式失败比让下一轮请求 400 更好
            let ChatCompletionMessageToolCalls::Function(function_call) = tool_call else {
                anyhow::bail!("模型返回了暂不支持的 custom tool call，本轮中止");
            };
            let function_name = &function_call.function.name;
            let arguments = &function_call.function.arguments;

            tracing::info!("Tool call: {function_name} ({arguments})");

            let (status, content) = match self.toolbox.get(function_name) {
                Some(tool) => match tool.execute(arguments, context).await {
                    Ok(result) => {
                        tracing::info!("Tool result: {result}");
                        (ToolResultStatus::Success, result)
                    }
                    Err(err) => {
                        let msg = format!("Tool execution error: {err}");
                        tracing::error!("{msg}");
                        (ToolResultStatus::Error, msg)
                    }
                },
                None => {
                    let msg = format!("Tool execution error: unknown tool {function_name}");
                    tracing::error!("{msg}");
                    (ToolResultStatus::Error, msg)
                }
            };

            result_items.push(ContentItem::ToolResult {
                tool_call_id: function_call.id.clone(),
                name: function_name.clone(),
                status,
                content,
            });
        }

        // execution_id 不能从 &mut context 里 move 出来，要 clone
        context.add_event(Event::new(
            context.execution_id.clone(),
            "tool",
            result_items,
        ));

        Ok(())
    }

    fn build_messages(
        &self,
        context: &ExecutionContext,
    ) -> anyhow::Result<Vec<ChatCompletionRequestMessage>> {
        let mut messages = Vec::new();

        if let Some(system) = self.instructions {
            messages.push(
                ChatCompletionRequestSystemMessageArgs::default()
                    .content(system)
                    .build()?
                    .into(),
            );
        }

        for event in &context.events {
            for item in &event.content {
                match item {
                    ContentItem::Message { role, content } => {
                        let message: ChatCompletionRequestMessage = if role == "user" {
                            ChatCompletionRequestUserMessageArgs::default()
                                .content(content.clone())
                                .build()?
                                .into()
                        } else {
                            ChatCompletionRequestAssistantMessageArgs::default()
                                .content(content.clone())
                                .build()?
                                .into()
                        };
                        messages.push(message);
                    }
                    ContentItem::ToolCall {
                        tool_call_id,
                        name,
                        arguments,
                    } => {
                        let tool_call = ChatCompletionMessageToolCalls::Function(
                            ChatCompletionMessageToolCall {
                                id: tool_call_id.clone(),
                                function: FunctionCall {
                                    name: name.clone(),
                                    arguments: arguments.to_string(),
                                },
                            },
                        );

                        if let Some(ChatCompletionRequestMessage::Assistant(last)) =
                            messages.last_mut()
                        {
                            last.tool_calls
                                .get_or_insert_with(|| Vec::new())
                                .push(tool_call);
                        } else {
                            messages.push(
                                ChatCompletionRequestAssistantMessageArgs::default()
                                    .tool_calls(vec![tool_call])
                                    .build()?
                                    .into(),
                            );
                        }
                    }

                    ContentItem::ToolResult {
                        tool_call_id,
                        content,
                        ..
                    } => {
                        messages.push(
                            ChatCompletionRequestToolMessageArgs::default()
                                .tool_call_id(tool_call_id.clone())
                                .content(content.clone())
                                .build()?
                                .into(),
                        );
                    }
                }
            }
        }
        Ok(messages)
    }
}
