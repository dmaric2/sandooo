/// Provides alerting and notification functionality for the Sandooo project.
///
/// Integrates with Telegram via the `teloxide` crate to send alerts about important events.
use anyhow::Result;
use ethers::types::{H256, U64};
use teloxide::prelude::*;
use teloxide::types::ChatId;

use crate::common::constants::Env;

/// Handles Telegram bot setup and sending alert messages.
///
/// Fields:
/// - `bot`: Optional Telegram bot instance (None if alerting is disabled)
/// - `chat_id`: Optional Telegram chat ID (None if alerting is disabled)
pub struct Alert {
    /// Optional Telegram bot instance (None if alerting is disabled)
    pub bot: Option<Bot>,
    /// Optional Telegram chat ID (None if alerting is disabled)
    pub chat_id: Option<ChatId>,
}

impl Alert {
    /// Creates a new `Alert` instance, initializing the Telegram bot and chat ID if enabled in the environment.
    ///
    /// # Returns
    /// * `Alert` - Configured alert handler (enabled or disabled based on environment).
    ///
    /// # Panics
    /// Panics if chat ID parsing fails and alerting is enabled.
    pub fn new() -> Self {
        let env = Env::new();
        if env.use_alert {
            let bot = Bot::from_env();
            let chat_id = ChatId(env.telegram_chat_id.parse::<i64>().unwrap());
            Self {
                bot: Some(bot),
                chat_id: Some(chat_id),
            }
        } else {
            Self {
                bot: None,
                chat_id: None,
            }
        }
    }

    /// Sends a message via Telegram if alerting is enabled.
    ///
    /// # Parameters
    /// * `message`: &str - The message to send.
    ///
    /// # Returns
    /// * `Result<()>` - Ok if sent or alerting is disabled, error if sending fails.
    pub async fn send(&self, message: &str) -> Result<()> {
        match &self.bot {
            Some(bot) => {
                bot.send_message(self.chat_id.unwrap(), message).await?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Sends a detailed bundle sent alert to Telegram, including block number, tx hash, and gambit hash.
    ///
    /// # Parameters
    /// * `block_number`: U64 - The block number associated with the bundle.
    /// * `tx_hash`: H256 - The transaction hash of the bundle.
    /// * `gambit_hash`: H256 - The hash of the gambit bundle.
    ///
    /// # Returns
    /// * `Result<()>` - Ok if sent or alerting is disabled, error if sending fails.
    pub async fn send_bundle_sent(
        &self,
        block_number: U64,
        tx_hash: H256,
        gambit_hash: H256,
    ) -> Result<()> {
        let eigenphi_url = format!("https://eigenphi.io/mev/eigentx/{:?}", tx_hash);
        let gambit_url = format!("https://gmbit-co.vercel.app/auction?txHash={:?}", tx_hash);
        let mut message = format!("[Block #{:?}] Bundle sent: {:?}", block_number, tx_hash);
        message = format!("{}\n-Eigenphi: {}", message, eigenphi_url);
        message = format!("{}\n-Gambit: {}", message, gambit_url);
        message = format!("{}\n-Gambit bundle hash: {:?}", message, gambit_hash);
        self.send(&message).await?;
        Ok(())
    }
}
