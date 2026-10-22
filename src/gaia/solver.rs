use anyhow::Ok;
use async_openai::types::chat::{
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
    CreateChatCompletionRequestArgs, FinishReason, ResponseFormat,
};
use backon::{BackoffBuilder, ExponentialBuilder, Retryable};

use crate::gaia::models::{GaiaOutput, HfResponse};

pub const GAIA_PROMPT: &'static str = r#"
    You are a general AI assistant. I will ask you a question.
    First, determine if you can solve this problem with your current capabilities and set "is_solvable" accordingly.
    If you can solve it, set "is_solvable" to true and provide your answer in "final_answer".
    If you cannot solve it, set "is_solvable" to false and explain why in "unsolvable_reason".
    Your final answer should be a number OR as few words as possible OR a comma-separated list of numbers and/or strings.
    If you are asked for a number, don't use a comma to write your number neither use units such as $ or percent sign unless specified otherwise.
    If you are asked for a string, don't use articles, neither abbreviations (e.g., for cities), and write the digits in plain text unless specified otherwise.
    If you are asked for a comma-separated list, apply the above rules depending on whether the element is a number or a string.

    You MUST respond with a single raw JSON object (no markdown fences, no prose)
    that strictly matches this JSON Schema:
    {"type":"object","properties":{"is_solvable":{"type":"boolean"},"unsolvable_reason":{"type":"string"},"final_answer":{"type":"string"}},"required":["is_solvable","unsolvable_reason","final_answer"],"additionalProperties":false}
    All three fields are required; use an empty string for unsolvable_reason when the problem is solvable.
    "#;

pub async fn solve_problem_with_retry(
    model: &str,
    system: &str,
    prompt: &str,
) -> anyhow::Result<GaiaOutput> {
    let op = || async { solve_problem(model, system, prompt).await };
    op.retry(ExponentialBuilder::default().with_max_times(3)).await
}

async fn solve_problem(model: &str, system: &str, prompt: &str) -> anyhow::Result<GaiaOutput> {
    // DeepSeek 只支持 json_object，不支持 OpenAI 的 json_schema / strict structured outputs；
    // 另外启用 json_object 时要求 prompt 中出现 "json" 字样（见 GAIA_PROMPT 末段）。
    let format_setting = ResponseFormat::JsonObject;

    let client = async_openai::Client::new();
    let request = CreateChatCompletionRequestArgs::default()
        .model(model)
        .messages([
            ChatCompletionRequestSystemMessageArgs::default()
                .content(system)
                .build()?
                .into(),
            ChatCompletionRequestUserMessageArgs::default()
                .content(prompt)
                .build()?
                .into(),
        ])
        .response_format(format_setting)
        .build()?;

    let response = client.chat().create(request).await?;

    let choice = response
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("No choices in response"))?;

    if choice.finish_reason == Some(FinishReason::ContentFilter) {
        return Ok(GaiaOutput {
            is_solvable: false,
            unsolvable_reason: format!("Model refuse to answer"),
            final_answer: String::new(),
        });
    }

    let content = choice
        .message
        .content
        .ok_or_else(|| anyhow::anyhow!("No content in response"))?;
    let output: GaiaOutput = serde_json::from_str(&content)?;
    Ok(output)
}
