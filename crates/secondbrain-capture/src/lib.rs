//! secondbrain capture loop.
//!
//! Polls every visible window across every monitor, runs Apple's
//! on-device Vision OCR, and writes results into the secondbrain
//! store as text-only extractions. Screenshots are never persisted —
//! they're decoded, OCR'd, and dropped.
//!
//! This is the *TextOnly* mode of the eventual capture director.
//! Audio, video, and image-vision-fallback modes will be added as
//! the director comes online.

use anyhow::{Context, Result};
use chrono::Utc;
use screenpipe_core::Language;
use screenpipe_screen::capture_screenshot_by_window::{
    capture_all_visible_windows, CapturedWindow, WindowFilters,
};
use screenpipe_screen::monitor::{list_monitors_detailed, SafeMonitor};
use secondbrain_store::models::{NewExtraction, NewSegment};
use secondbrain_store::repo;
use secondbrain_store::Store;
use serde::Serialize;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{sleep, Instant};
use tracing::{debug, info, warn};

const SOURCE_OCR: &str = "ocr";
/// Minimum text length we bother persisting. Avoids spamming the store
/// with two-character titlebar OCR noise.
const MIN_TEXT_LEN: usize = 8;

/// Per-window key used to dedupe segments and extractions.
/// `(process_id, window_name, monitor_id)` — same window across runs
/// gets the same key.
type WindowKey = (i32, String, u32);

#[derive(Clone)]
pub struct CaptureConfig {
    pub interval: Duration,
    pub capture_unfocused_windows: bool,
    pub languages: Vec<Language>,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_millis(1000),
            capture_unfocused_windows: true,
            languages: vec![Language::English],
        }
    }
}

#[derive(Debug, Default, Clone, Serialize)]
struct OcrPayload {
    process_id: i32,
    monitor_id: u32,
    monitor_name: String,
    is_focused: bool,
    confidence: Option<f64>,
}

/// Open segment + last-seen text for an active window.
struct ActiveWindow {
    segment_id: i64,
    last_text_hash: u64,
}

/// Cache of currently-open segments. A segment lives as long as the
/// (pid, window_name, monitor) tuple keeps appearing in successive
/// frames; once the window disappears we close it (set ended_at) on
/// the next sweep.
#[derive(Default)]
pub struct SegmentTracker {
    open: HashMap<WindowKey, ActiveWindow>,
}

impl SegmentTracker {
    pub fn new() -> Self {
        Self::default()
    }
}

pub async fn run(store: Store, cfg: CaptureConfig) -> Result<()> {
    let monitors = list_monitors_detailed()
        .await
        .map_err(|e| anyhow::anyhow!("listing monitors: {e}"))?;
    if monitors.is_empty() {
        anyhow::bail!("no monitors found");
    }
    info!(
        n_monitors = monitors.len(),
        interval_ms = cfg.interval.as_millis() as u64,
        capture_unfocused = cfg.capture_unfocused_windows,
        "starting capture loop"
    );

    let filters = Arc::new(WindowFilters::new(&[], &[], &[]));
    let mut tracker = SegmentTracker::new();
    let pool = store.pool().clone();

    loop {
        let tick_start = Instant::now();
        for monitor in &monitors {
            if let Err(e) =
                sweep_monitor(&pool, &mut tracker, monitor, &filters, &cfg).await
            {
                warn!(monitor = monitor.id(), error = ?e, "sweep failed");
            }
        }
        let elapsed = tick_start.elapsed();
        if elapsed < cfg.interval {
            sleep(cfg.interval - elapsed).await;
        } else {
            debug!(
                tick_ms = elapsed.as_millis() as u64,
                interval_ms = cfg.interval.as_millis() as u64,
                "tick exceeded interval"
            );
        }
    }
}

async fn sweep_monitor(
    pool: &SqlitePool,
    tracker: &mut SegmentTracker,
    monitor: &SafeMonitor,
    filters: &Arc<WindowFilters>,
    cfg: &CaptureConfig,
) -> Result<()> {
    let captured = match capture_all_visible_windows(monitor, filters, cfg.capture_unfocused_windows)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "window capture failed");
            return Ok(());
        }
    };

    let now_ms = Utc::now().timestamp_millis();
    let mut seen_keys = Vec::with_capacity(captured.len());

    for window in captured {
        let key: WindowKey = (
            window.process_id,
            window.window_name.clone(),
            monitor.id(),
        );
        seen_keys.push(key.clone());

        if let Err(e) = handle_window(pool, tracker, monitor, key, window, now_ms, cfg).await {
            warn!(error = ?e, "handle_window failed");
        }
    }

    // Close segments for windows that disappeared this tick.
    let stale: Vec<WindowKey> = tracker
        .open
        .keys()
        .filter(|k| !seen_keys.contains(k))
        .filter(|(_, _, mon_id)| *mon_id == monitor.id())
        .cloned()
        .collect();
    for key in stale {
        if let Some(active) = tracker.open.remove(&key) {
            if let Err(e) = repo::close_segment(pool, active.segment_id, now_ms).await {
                warn!(error = ?e, segment = active.segment_id, "close_segment failed");
            }
        }
    }

    Ok(())
}

fn hash_text(s: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

async fn handle_window(
    pool: &SqlitePool,
    tracker: &mut SegmentTracker,
    monitor: &SafeMonitor,
    key: WindowKey,
    window: CapturedWindow,
    now_ms: i64,
    cfg: &CaptureConfig,
) -> Result<()> {
    let (text, _json, confidence) = ocr_apple(&window, &cfg.languages);
    let trimmed = text.trim();
    if trimmed.len() < MIN_TEXT_LEN {
        return Ok(());
    }
    let text_hash = hash_text(trimmed);

    if !tracker.open.contains_key(&key) {
        let segment_id = repo::insert_segment(
            pool,
            &NewSegment {
                started_at: now_ms,
                app_bundle: window.app_name.clone(),
                app_name: window.app_name.clone(),
                window_title: Some(window.window_name.clone()),
                url: None,
                monitor_id: Some(monitor.id() as i64),
                focused: window.is_focused,
            },
        )
        .await
        .context("insert_segment")?;
        tracker.open.insert(
            key.clone(),
            ActiveWindow {
                segment_id,
                last_text_hash: 0,
            },
        );
        info!(
            segment = segment_id,
            app = %window.app_name,
            title = %window.window_name,
            focused = window.is_focused,
            "opened segment"
        );
    }

    let active = tracker.open.get_mut(&key).expect("just inserted");
    if active.last_text_hash == text_hash {
        return Ok(());
    }
    active.last_text_hash = text_hash;

    let payload = OcrPayload {
        process_id: window.process_id,
        monitor_id: monitor.id(),
        monitor_name: monitor.name().to_string(),
        is_focused: window.is_focused,
        confidence,
    };
    let payload_json = serde_json::to_string(&payload).ok();

    repo::insert_extraction(
        pool,
        &NewExtraction {
            segment_id: active.segment_id,
            captured_at: now_ms,
            source: SOURCE_OCR.into(),
            text: trimmed.to_string(),
            confidence,
            raw_json: payload_json,
        },
    )
    .await
    .context("insert_extraction")?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn ocr_apple(window: &CapturedWindow, languages: &[Language]) -> (String, String, Option<f64>) {
    screenpipe_screen::perform_ocr_apple(&window.image, languages)
}

#[cfg(not(target_os = "macos"))]
fn ocr_apple(window: &CapturedWindow, languages: &[Language]) -> (String, String, Option<f64>) {
    let (text, json, conf) = screenpipe_screen::perform_ocr_tesseract(
        &window.image,
        languages.to_vec(),
    );
    (text, json, conf)
}

