//! Sandooo - MEV Sandwich Attack Bot
use dotenv::dotenv;
use ethers::providers::{Middleware, Provider, Ws};
use log::info;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::time;

use sandooo::common::streams::{stream_new_blocks, stream_pending_txs, Event, NewBlock};
use sandooo::common::utils;
use sandooo::sandwich::strategy::run_sandwich_strategy;

use fern::colors::ColoredLevelConfig;
use colored::Colorize;
use chrono::Local;

/// Default buffer size for event channels
const DEFAULT_BUFFER_SIZE: usize = 2048;

/// Sets up colored logging for all events
fn setup_logger() -> Result<(), Box<dyn std::error::Error>> {
    // Configure colors for log levels
    let colors = ColoredLevelConfig::new()
        .error(fern::colors::Color::Red)
        .warn(fern::colors::Color::Yellow)
        .info(fern::colors::Color::Green)
        .debug(fern::colors::Color::Blue)
        .trace(fern::colors::Color::Magenta);

    fern::Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "{} [{}] {}",
                Local::now().format("%Y-%m-%d %H:%M:%S"),
                colors.color(record.level()),
                message.to_string().white(),
            ));
        })
        .level(log::LevelFilter::Debug)
        .chain(std::io::stdout())
        .apply()?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize colored logger
    setup_logger()?;

    // Load environment variables from .env file.
    dotenv().ok();

    info!("Starting Sandooo");

    // Create a broadcast channel with a reasonable buffer size.
    let (event_sender, _) = broadcast::channel::<Event>(DEFAULT_BUFFER_SIZE);

    // Create an Ethereum provider.
    let env = sandooo::common::constants::Env::new();
    let ws = Ws::connect(&env.wss_url).await?;
    let provider = Arc::new(Provider::new(ws));

    // Start a manual block polling mechanism as fallback
    let fallback_sender = event_sender.clone();
    let fallback_provider = provider.clone();
    tokio::spawn(async move {
        let mut last_block_number = ethers::types::U64::zero();
        loop {
            match fallback_provider.get_block_number().await {
                Ok(block_number) => {
                    if block_number > last_block_number
                        && last_block_number != ethers::types::U64::zero()
                    {
                        eprintln!("DEBUG: Manual polling detected new block: {}", block_number);

                        // Get the block details
                        if let Ok(Some(block)) = fallback_provider.get_block(block_number).await {
                            let base_fee = block.base_fee_per_gas.unwrap_or_default();
                            let next_base_fee = utils::calculate_next_block_base_fee(
                                block.gas_used,
                                block.gas_limit,
                                base_fee,
                            );

                            let event = Event::Block(NewBlock {
                                block_number,
                                base_fee,
                                next_base_fee,
                            });

                            if fallback_sender.send(event).is_err() {
                                eprintln!("DEBUG: Failed to send fallback block event");
                            } else {
                                eprintln!(
                                    "DEBUG: Successfully sent fallback block event for #{}",
                                    block_number
                                );
                            }
                        }
                    }
                    last_block_number = block_number;
                }
                Err(e) => {
                    eprintln!("DEBUG: Error in fallback block polling: {:?}", e);
                }
            }

            // Wait a few seconds before polling again
            time::sleep(time::Duration::from_secs(5)).await;
        }
    });

    // Start pending transactions stream.
    let tx_sender = event_sender.clone();
    let provider_clone1 = provider.clone();
    tokio::spawn(async move {
        if let Err(e) = stream_pending_txs(provider_clone1, tx_sender).await {
            eprintln!("Error in pending tx stream: {:?}", e);
        }
    });

    // Start new blocks stream.
    let block_sender = event_sender.clone();
    let provider_clone2 = provider.clone();
    tokio::spawn(async move {
        if let Err(e) = stream_new_blocks(provider_clone2, block_sender).await {
            eprintln!("Error in new blocks stream: {:?}", e);
        }
    });

    // Run the sandwich trading strategy.
    // This function never returns normally (it has the "never" type '!' as return type)
    // Any code after this line is unreachable
    run_sandwich_strategy(provider, event_sender).await
}
