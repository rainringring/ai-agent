use anyhow::Ok;
use tracing::{Level, instrument::WithSubscriber};
use tracing_subscriber::FmtSubscriber;

use ai_agent::{
    constant::DEEPSEEK_FLASH_4_1, llm::{
        complete::chat_complete, structured::chat_complete_structured,
        structured_ds::chat_complete_structured_ds,
    }, tools::build_tool_box,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv()?;

    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    let toolbox = build_tool_box().await?;

    let answer = chat_complete(
        DEEPSEEK_FLASH_4_1,
        Some(r#"你是一个全能的助手。今天的日期是2026年10月20日。
            你可以使用工具来搜索最新信息。
            重要：当工具返回搜索结果时，你必须直接使用这些结果来回答，不要说"信息尚未公布"或"我不知道"。
            你的训练数据有截止日期，可能已经过时，请始终优先信任工具返回的内容。"#),
        "2026世界杯决赛比分是多少",
        &toolbox,
    )
    .await?;

    println!("Request: {:?}", answer);

    Ok(())
}
