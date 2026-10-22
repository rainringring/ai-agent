use anyhow::Ok;
use tracing::{Level, instrument::WithSubscriber};
use tracing_subscriber::FmtSubscriber;

use ai_agent::{constant::DEEPSEEK_FLASH_4_1, llm::{complete::chat_complete, structured::chat_complete_structured, structured_ds::{chat_complete_structured_ds}}};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv()?;

    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    let answer = chat_complete_structured_ds(
        DEEPSEEK_FLASH_4_1,
        "瑞士在欧洲吗，有什么好吃的",
    )
    .await?;

    println!("Request: {:?}", answer);

    Ok(())
}
