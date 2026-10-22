use async_openai::types::chat::{
    ChatCompletionRequestAssistantMessage, ChatCompletionRequestAssistantMessageArgs,
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
    CreateChatCompletionRequestArgs, ResponseFormat,
};
use schemars::schema_for;

use crate::models::action_plan::{self, ActionPlan};

pub async fn chat_complete_structured_ds(
    model: &str,
    prompt: &str,
) -> anyhow::Result<ActionPlan> {
    let client = async_openai::Client::new();
    let mut message = vec![
        ChatCompletionRequestSystemMessageArgs::default()
                .content(build_system_prompt())
                .build()?
                .into(),
        ChatCompletionRequestUserMessageArgs::default()
            .content(prompt)
            .build()?
            .into(),
    ];

    let schema = schemars::schema_for!(ActionPlan);
    let schema_json = schema.as_value().clone();
    let format_setting = ResponseFormat::JsonObject;

    let request = CreateChatCompletionRequestArgs::default()
        .model(model)
        .messages(message)
        .response_format(format_setting)
        .max_tokens(2048u32)
        .build()?;

    let reponse = client.chat().create(request).await?;

    tracing::info!("Reponse: {:#?}", reponse);

    let plan: ActionPlan = reponse
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .ok_or_else(|| anyhow::anyhow!("No content in response!"))
        .and_then(|s| serde_json::from_str(&s).map_err(Into::into))?;

    Ok(plan)
}

fn build_system_prompt() -> String {
    let schema = schema_for!(ActionPlan);
    let schema_str = serde_json::to_string_pretty(&schema).unwrap();

    format!(
            r#"You are a planning assistant. Analyze the user's request and respond with a JSON object.
    The output must be valid JSON that strictly conforms to this JSON Schema:

    {schema_str}

    Rules:
    -Output ONLY the raw JSON object, no markdown fences, no explanation
    -All required fields must be present
    -'difficulty' must be exactly one of: "Easy", "Medium", "Hard"
    -'steps' must be a non-empty array
    -Respond with JSON only"#
    )
}
