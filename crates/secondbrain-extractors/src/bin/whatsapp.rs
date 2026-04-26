//! `secondbrain-extract-whatsapp`
//!
//! Reads WhatsApp Desktop's local ChatStorage.sqlite and writes new
//! messages into the secondbrain store. Idempotent: runs from a stored
//! per-extractor watermark.

use anyhow::Result;
use chrono::{TimeZone, Utc};
use clap::Parser;
use secondbrain_extractors::whatsapp;
use secondbrain_store::Store;
use tracing::info;

#[derive(Parser)]
#[command(version, about = "Sync WhatsApp messages into secondbrain store")]
struct Args {
    /// Override the secondbrain store path. Defaults to
    /// ~/Library/Application Support/secondbrain/store.db.
    #[arg(long)]
    store: Option<std::path::PathBuf>,

    /// Print what would be ingested without writing.
    #[arg(long)]
    dry_run: bool,
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

    if args.dry_run {
        let chat_path = whatsapp::default_chat_storage_path()?;
        info!(?chat_path, "dry run: would copy and read");
        let tmp = whatsapp::copy_to_temp(&chat_path)?;
        info!(?tmp, "copied; re-run without --dry-run to ingest");
        return Ok(());
    }

    let store = match &args.store {
        Some(p) => Store::open(p).await?,
        None => Store::open_default().await?,
    };

    let report = whatsapp::sync(&store).await?;

    let watermark_human = report
        .new_watermark_unix_ms
        .and_then(|ms| Utc.timestamp_millis_opt(ms).single())
        .map(|d| d.to_rfc3339())
        .unwrap_or_else(|| "<none>".into());

    println!(
        "ingested {} messages across {} segments (watermark: {})",
        report.messages_ingested, report.segments_touched, watermark_human,
    );
    Ok(())
}
