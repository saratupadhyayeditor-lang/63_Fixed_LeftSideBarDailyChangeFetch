//! Diagnostic: open one Dhan live-feed handshake and print the exact result.
//!
//! Usage: cargo run -p dhan-hq --example feed_probe -- <client_id> <access_token>
//!
//! Useful when the feed never ticks: it separates a rejected handshake (HTTP
//! status + body) from an accepted socket that immediately disconnects (error
//! code on the binary channel).

use dhan_hq::{FeedMode, FeedSubscription, MarketFeed};
use tokio::sync::mpsc;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(client_id), Some(token)) = (args.next(), args.next()) else {
        eprintln!("usage: feed_probe <client_id> <access_token>");
        std::process::exit(2);
    };
    let feed = MarketFeed::new(
        client_id,
        token,
        vec![FeedSubscription::with_mode(
            dhan_hq::ExchangeSegment::IdxI,
            13,
            FeedMode::Ticker,
        )],
    );
    let (tx, mut rx) = mpsc::channel(64);
    let (_cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let runner = tokio::spawn(feed.run(tx, cmd_rx));
    for _ in 0..5 {
        match tokio::time::timeout(std::time::Duration::from_secs(8), rx.recv()).await {
            Ok(Some(pkt)) => println!("packet: {pkt:?}"),
            Ok(None) => {
                println!("feed closed");
                break;
            }
            Err(_) => println!("no packet within 8s"),
        }
    }
    runner.abort();
}
