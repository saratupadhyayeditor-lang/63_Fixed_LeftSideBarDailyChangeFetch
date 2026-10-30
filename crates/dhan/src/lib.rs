//! # dhan-hq
//!
//! A fresh Rust client for the **DhanHQ v2** REST and Live Market Feed APIs,
//! written against the official documentation (https://dhanhq.co/docs/v2/).
//!
//! ```no_run
//! # async fn demo() -> Result<(), dhan_hq::DhanError> {
//! use dhan_hq::DhanClient;
//! let dhan = DhanClient::new("1000000001", "eyJ...");
//! let profile = dhan.profile().await?;
//! println!("connected as {}", profile.dhan_client_id);
//! # Ok(()) }
//! ```
//!
//! The crate is broker-agnostic: it owns no global state and stores no
//! credentials on disk. The server layer decides how a session is held.

pub mod client;
pub mod error;
pub mod feed;
pub mod models;

pub use client::{DhanClient, DEFAULT_BASE};
pub use error::{DhanError, DhanErrorBody, Result};
pub use feed::{
    disconnect_message, disconnect_reason, parse_packet, subscribe_message, subscribe_messages,
    unsubscribe_message, unsubscribe_messages, DepthLevel, FeedCommand, FeedMode, FeedPacket,
    FeedSubscription, MarketFeed,
};
pub use models::*;
