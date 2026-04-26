//! `secondbrain-capture-daemon` — TextOnly screen capture loop.
//!
//! Polls every visible window across every monitor, OCRs with Apple's
//! on-device Vision framework (or Tesseract on Linux), and writes new
//! text into the secondbrain store. Screenshots are not persisted.

use anyhow::Result;
use clap::Parser;
use secondbrain_capture::{run, CaptureConfig};
use secondbrain_store::Store;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(version, about = "secondbrain text-only capture daemon")]
struct Args {
    /// Override the store path. Defaults to
    /// ~/Library/Application Support/secondbrain/store.db.
    #[arg(long)]
    store: Option<PathBuf>,

    /// Capture interval in milliseconds.
    #[arg(long, default_value_t = 1000)]
    interval_ms: u64,

    /// Capture all visible windows (true) or just the topmost on each
    /// monitor (false).
    #[arg(long, default_value_t = true)]
    all_windows: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let store = match &args.store {
        Some(p) => Store::open(p).await?,
        None => Store::open_default().await?,
    };

    let cfg = CaptureConfig {
        interval: Duration::from_millis(args.interval_ms),
        capture_unfocused_windows: args.all_windows,
        languages: vec![screenpipe_core::Language::English],
    };

    run(store, cfg).await
}
