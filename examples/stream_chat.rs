use std::{io::{self, Write}, result};
use async_openai::types::chat::Prompt;
use futures::StreamExt;
use tokio::task::JoinSet;
use tracing::{Instrument, Level, span};
use tracing_subscriber::FmtSubscriber;

use ai_agent::{
    constant::DEEPSEEK_FLASH_4_1,
    llm::{
        semaphore::get_semphore,
        stream::{chat_stream_with_retry},
    },
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv()?;

    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    let prompts = vec![
        "欧洲有什么发展比较好的国家",
        "什么是async函数",
        "什么时候写await",
        "Rust的优点有哪些",
        "讲讲主流分布式存储系统",
        "讲讲网络,I/O相关知识以及在哪些工程中使用",
    ];

    let mut set = JoinSet::new();
    for prompt in prompts {
        let span = tracing::info_span!("Chat", prompt = prompt);
        set.spawn(
            async move {
                tracing::info!("\n\n{prompt}");
                let permit = get_semphore().acquire().await?;
                let output =
                    chat_stream_with_retry(DEEPSEEK_FLASH_4_1, Some("你是我的助手"), prompt)
                        .await?;
                drop(permit);
                Ok::<_, anyhow::Error>((prompt, output))
            }
            .instrument(span),
        );
    }

    while let Some(result) = set.join_next().await {
        match result {
            Ok(Ok((prompt, result))) => tracing::info!("\n{prompt}\n{result}"),
            Ok(Err(err)) => tracing::error!("Task panicked: {err}"),
            Err(err) => tracing::error!("Task panicked: {err}"),
        }
    }

    Ok(())
}
