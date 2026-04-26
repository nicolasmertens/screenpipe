//! WhatsApp native macOS extractor.
//!
//! Reads from `~/Library/Group Containers/group.net.whatsapp.WhatsApp.shared/ChatStorage.sqlite`.
//! WhatsApp Desktop holds an exclusive lock on the live DB while running,
//! so we copy it to a temp file before reading.
//!
//! Segment model: one segment per (chat session, UTC day). Each WhatsApp
//! message becomes an extraction inside that segment with `source =
//! "whatsapp"`. Segments outlive the wall-clock day they refer to —
//! `started_at` and `ended_at` are set to the day's UTC bounds.

use crate::apple_to_unix_ms;
use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use secondbrain_store::models::{NewExtraction, NewSegment};
use secondbrain_store::repo;
use secondbrain_store::Store;
use serde::Serialize;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tracing::{debug, info, warn};

const EXTRACTOR_NAME: &str = "whatsapp";
const APP_BUNDLE: &str = "net.whatsapp.WhatsApp";
const APP_NAME: &str = "WhatsApp";
const SOURCE: &str = "whatsapp";
const DAY_MS: i64 = 86_400_000;

/// Returns the default ChatStorage.sqlite path on macOS.
pub fn default_chat_storage_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not resolve home dir")?;
    Ok(home
        .join("Library")
        .join("Group Containers")
        .join("group.net.whatsapp.WhatsApp.shared")
        .join("ChatStorage.sqlite"))
}

/// Copy ChatStorage.sqlite (and its WAL/SHM siblings) to a temp dir
/// so we can read while WhatsApp Desktop holds the lock.
pub fn copy_to_temp(source: &Path) -> Result<tempfile::TempDir> {
    let tmp = tempfile::tempdir().context("creating temp dir")?;
    let dest = tmp.path().join("ChatStorage.sqlite");
    std::fs::copy(source, &dest)
        .with_context(|| format!("copying {source:?} -> {dest:?}"))?;

    for ext in ["-wal", "-shm"] {
        let src = source.with_file_name(format!(
            "{}{}",
            source.file_name().unwrap().to_string_lossy(),
            ext
        ));
        if src.exists() {
            let dest = tmp.path().join(format!("ChatStorage.sqlite{ext}"));
            std::fs::copy(&src, &dest)
                .with_context(|| format!("copying {src:?} -> {dest:?}"))?;
        }
    }
    Ok(tmp)
}

#[derive(Debug, Clone, Serialize)]
struct MessagePayload {
    is_from_me: bool,
    from_jid: Option<String>,
    to_jid: Option<String>,
    push_name: Option<String>,
    chat_jid: Option<String>,
    chat_partner: Option<String>,
    is_group: bool,
    message_type: i64,
    stanza_id: Option<String>,
}

#[derive(Debug)]
struct RawMessage {
    apple_date: f64,
    text: Option<String>,
    is_from_me: bool,
    from_jid: Option<String>,
    to_jid: Option<String>,
    push_name: Option<String>,
    message_type: i64,
    stanza_id: Option<String>,
    chat_jid: Option<String>,
    chat_partner: Option<String>,
    is_group: bool,
}

pub struct SyncReport {
    pub messages_ingested: usize,
    pub segments_touched: usize,
    pub new_watermark_unix_ms: Option<i64>,
}

/// The WhatsApp extractor stores its watermark as **Apple-microseconds**
/// (= ZMESSAGEDATE * 1_000_000). This preserves sub-millisecond precision
/// of the source TIMESTAMP REAL column; storing Unix-ms watermarks loses
/// the fractional milliseconds and causes a small re-ingest tail on each
/// run.
fn apple_to_microseconds(apple_seconds: f64) -> i64 {
    (apple_seconds * 1_000_000.0).round() as i64
}

fn microseconds_to_apple(micro: i64) -> f64 {
    (micro as f64) / 1_000_000.0
}

/// Open the ChatStorage copy read-only.
async fn open_chat_storage(path: &Path) -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
        .read_only(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .with_context(|| format!("opening ChatStorage at {path:?}"))?;
    Ok(pool)
}

/// Fetch all messages newer than the watermark (Apple seconds), oldest first.
/// `chat_jid` is non-null for individual chats and groups alike (a JID is
/// the WhatsApp identifier — `*@s.whatsapp.net` for users, `*@g.us` for
/// groups, `*@lid` for linked-device IDs).
async fn fetch_new_messages(
    pool: &SqlitePool,
    after_apple_seconds: f64,
) -> Result<Vec<RawMessage>> {
    let rows = sqlx::query(
        r#"
        SELECT
            m.ZMESSAGEDATE        AS msg_date,
            m.ZTEXT               AS text,
            m.ZISFROMME           AS is_from_me,
            m.ZFROMJID            AS from_jid,
            m.ZTOJID              AS to_jid,
            m.ZPUSHNAME           AS push_name,
            m.ZMESSAGETYPE        AS msg_type,
            m.ZSTANZAID           AS stanza_id,
            s.ZCONTACTJID         AS chat_jid,
            s.ZPARTNERNAME        AS chat_partner,
            s.ZSESSIONTYPE        AS session_type
        FROM ZWAMESSAGE m
        LEFT JOIN ZWACHATSESSION s ON s.Z_PK = m.ZCHATSESSION
        WHERE m.ZMESSAGEDATE IS NOT NULL
          AND m.ZMESSAGEDATE > ?1
        ORDER BY m.ZMESSAGEDATE ASC
        "#,
    )
    .bind(after_apple_seconds)
    .fetch_all(pool)
    .await
    .context("querying ZWAMESSAGE")?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let chat_jid: Option<String> = r.try_get("chat_jid").ok();
        let session_type: Option<i64> = r.try_get("session_type").ok();
        let is_group = chat_jid
            .as_deref()
            .map(|j| j.contains("@g.us"))
            .unwrap_or(false)
            || matches!(session_type, Some(1));

        // ZMESSAGEDATE is declared TIMESTAMP, but SQLite stores it as
        // INTEGER for whole-second values and REAL for fractional ones.
        // Fall through both type readers so neither path silently drops
        // the column to 0.0.
        let apple_date = r
            .try_get::<f64, _>("msg_date")
            .or_else(|_| r.try_get::<i64, _>("msg_date").map(|i| i as f64))
            .unwrap_or(0.0);

        out.push(RawMessage {
            apple_date,
            text: r.try_get("text").ok(),
            is_from_me: r.try_get::<i64, _>("is_from_me").unwrap_or(0) != 0,
            from_jid: r.try_get("from_jid").ok(),
            to_jid: r.try_get("to_jid").ok(),
            push_name: r.try_get("push_name").ok(),
            message_type: r.try_get::<i64, _>("msg_type").unwrap_or(0),
            stanza_id: r.try_get("stanza_id").ok(),
            chat_jid,
            chat_partner: r.try_get("chat_partner").ok(),
            is_group,
        });
    }
    Ok(out)
}

/// Find or create a segment for (chat_jid, UTC day).
/// Returns the segment id.
async fn ensure_segment(
    store_pool: &SqlitePool,
    chat_jid: &str,
    chat_partner: Option<&str>,
    day_start_ms: i64,
) -> Result<i64> {
    let day_end_ms = day_start_ms + DAY_MS;

    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM segments \
         WHERE app_bundle = ?1 AND url = ?2 AND started_at = ?3 \
         LIMIT 1",
    )
    .bind(APP_BUNDLE)
    .bind(format!("whatsapp://chat/{chat_jid}"))
    .bind(day_start_ms)
    .fetch_optional(store_pool)
    .await?;

    if let Some(id) = existing {
        return Ok(id);
    }

    let day_label = Utc
        .timestamp_millis_opt(day_start_ms)
        .single()
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| day_start_ms.to_string());
    let title = match chat_partner {
        Some(p) if !p.is_empty() => format!("{p} — {day_label}"),
        _ => format!("{chat_jid} — {day_label}"),
    };

    let id = repo::insert_segment(
        store_pool,
        &NewSegment {
            started_at: day_start_ms,
            app_bundle: APP_BUNDLE.into(),
            app_name: APP_NAME.into(),
            window_title: Some(title),
            url: Some(format!("whatsapp://chat/{chat_jid}")),
            monitor_id: None,
            focused: false,
        },
    )
    .await?;
    repo::close_segment(store_pool, id, day_end_ms).await?;
    Ok(id)
}

fn day_bucket_ms(unix_ms: i64) -> i64 {
    (unix_ms / DAY_MS) * DAY_MS
}

/// Render a message to its extraction text. For media-only messages (no
/// ZTEXT) we synthesize a short placeholder so the row carries some
/// signal even before we resolve the media file.
fn render_text(msg: &RawMessage) -> String {
    if let Some(t) = msg.text.as_deref() {
        if !t.is_empty() {
            let direction = if msg.is_from_me { "→" } else { "←" };
            let speaker = if msg.is_from_me {
                "me".to_string()
            } else {
                msg.push_name
                    .clone()
                    .or_else(|| msg.from_jid.clone())
                    .unwrap_or_else(|| "?".into())
            };
            return format!("{direction} {speaker}: {t}");
        }
    }
    let kind = match msg.message_type {
        0 => "text-empty",
        1 => "image",
        2 => "audio",
        3 => "video",
        4 => "contact",
        5 => "location",
        7 => "url",
        8 => "document",
        10 => "call",
        11 => "gif",
        14 => "deleted",
        15 => "sticker",
        n => return format!("[media: type {n}]"),
    };
    format!("[{kind}]")
}

pub async fn sync(store: &Store) -> Result<SyncReport> {
    let chat_storage = default_chat_storage_path()?;
    if !chat_storage.exists() {
        anyhow::bail!("ChatStorage not found at {chat_storage:?}");
    }

    let store_pool = store.pool();
    let prev_watermark_apple_us = repo::get_watermark(store_pool, EXTRACTOR_NAME).await?;
    let after_apple = match prev_watermark_apple_us {
        Some(us) => microseconds_to_apple(us),
        None => -1.0, // include everything
    };

    info!(
        watermark_apple_us = prev_watermark_apple_us,
        watermark_apple_seconds = after_apple,
        "starting WhatsApp sync"
    );

    let tmp = copy_to_temp(&chat_storage)?;
    let wa_pool = open_chat_storage(&tmp.path().join("ChatStorage.sqlite")).await?;

    let messages = fetch_new_messages(&wa_pool, after_apple).await?;
    debug!(count = messages.len(), "fetched messages");

    let mut segment_cache: std::collections::HashMap<(String, i64), i64> =
        std::collections::HashMap::new();
    let mut max_seen_apple_us: Option<i64> = prev_watermark_apple_us;
    let mut max_seen_unix_ms: Option<i64> = None;

    for msg in &messages {
        // Advance watermarks for every fetched row, including ones we end
        // up skipping — otherwise skipped messages keep re-flowing on each
        // run and force re-inserts of everything newer in the same fetch.
        let apple_us = apple_to_microseconds(msg.apple_date);
        let unix_ms = apple_to_unix_ms(msg.apple_date);
        max_seen_apple_us = Some(max_seen_apple_us.map_or(apple_us, |w| w.max(apple_us)));
        max_seen_unix_ms = Some(max_seen_unix_ms.map_or(unix_ms, |w| w.max(unix_ms)));

        let chat_jid = match msg.chat_jid.clone() {
            Some(j) => j,
            None => {
                warn!(?msg.from_jid, ?msg.to_jid, "message without chat_jid; skipping");
                continue;
            }
        };
        let day = day_bucket_ms(unix_ms);

        let key = (chat_jid.clone(), day);
        let segment_id = if let Some(id) = segment_cache.get(&key) {
            *id
        } else {
            let id = ensure_segment(store_pool, &chat_jid, msg.chat_partner.as_deref(), day)
                .await?;
            segment_cache.insert(key, id);
            id
        };

        let payload = MessagePayload {
            is_from_me: msg.is_from_me,
            from_jid: msg.from_jid.clone(),
            to_jid: msg.to_jid.clone(),
            push_name: msg.push_name.clone(),
            chat_jid: msg.chat_jid.clone(),
            chat_partner: msg.chat_partner.clone(),
            is_group: msg.is_group,
            message_type: msg.message_type,
            stanza_id: msg.stanza_id.clone(),
        };

        repo::insert_extraction(
            store_pool,
            &NewExtraction {
                segment_id,
                captured_at: unix_ms,
                source: SOURCE.into(),
                text: render_text(msg),
                confidence: None,
                raw_json: Some(serde_json::to_string(&payload)?),
            },
        )
        .await?;

    }

    if let Some(w) = max_seen_apple_us {
        if Some(w) != prev_watermark_apple_us {
            repo::set_watermark(store_pool, EXTRACTOR_NAME, w).await?;
        }
    }

    Ok(SyncReport {
        messages_ingested: messages.len(),
        segments_touched: segment_cache.len(),
        new_watermark_unix_ms: max_seen_unix_ms,
    })
}
