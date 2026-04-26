//! Read/write helpers over the secondbrain pool.
//!
//! Extractors call into here rather than touching sqlx directly so that
//! schema changes stay contained.

use crate::models::{NewExtraction, NewSegment, Watermark};
use anyhow::Result;
use sqlx::SqlitePool;

pub async fn insert_segment(pool: &SqlitePool, seg: &NewSegment) -> Result<i64> {
    let row = sqlx::query(
        "INSERT INTO segments (started_at, app_bundle, app_name, window_title, url, monitor_id, focused) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(seg.started_at)
    .bind(&seg.app_bundle)
    .bind(&seg.app_name)
    .bind(&seg.window_title)
    .bind(&seg.url)
    .bind(seg.monitor_id)
    .bind(seg.focused)
    .execute(pool)
    .await?;
    Ok(row.last_insert_rowid())
}

pub async fn close_segment(pool: &SqlitePool, segment_id: i64, ended_at: i64) -> Result<()> {
    sqlx::query("UPDATE segments SET ended_at = ?1 WHERE id = ?2 AND ended_at IS NULL")
        .bind(ended_at)
        .bind(segment_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn insert_extraction(pool: &SqlitePool, ext: &NewExtraction) -> Result<i64> {
    let row = sqlx::query(
        "INSERT INTO extractions (segment_id, captured_at, source, text, confidence, raw_json) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(ext.segment_id)
    .bind(ext.captured_at)
    .bind(&ext.source)
    .bind(&ext.text)
    .bind(ext.confidence)
    .bind(&ext.raw_json)
    .execute(pool)
    .await?;
    Ok(row.last_insert_rowid())
}

pub async fn get_watermark(pool: &SqlitePool, extractor: &str) -> Result<Option<i64>> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT watermark FROM extractor_watermarks WHERE extractor = ?1")
            .bind(extractor)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|r| r.0))
}

pub async fn set_watermark(pool: &SqlitePool, extractor: &str, watermark: i64) -> Result<()> {
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query(
        "INSERT INTO extractor_watermarks (extractor, watermark, updated_at) VALUES (?1, ?2, ?3) \
         ON CONFLICT(extractor) DO UPDATE SET watermark = excluded.watermark, updated_at = excluded.updated_at",
    )
    .bind(extractor)
    .bind(watermark)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_watermarks(pool: &SqlitePool) -> Result<Vec<Watermark>> {
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT extractor, watermark, updated_at FROM extractor_watermarks ORDER BY extractor",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(extractor, watermark, updated_at)| Watermark {
            extractor,
            watermark,
            updated_at,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;
    use tempfile::tempdir;

    #[tokio::test]
    async fn segment_extraction_round_trip() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db")).await.unwrap();
        let pool = store.pool();

        let seg_id = insert_segment(
            pool,
            &NewSegment {
                started_at: 1,
                app_bundle: "com.test".into(),
                app_name: "Test".into(),
                window_title: Some("hello".into()),
                url: None,
                monitor_id: Some(0),
                focused: true,
            },
        )
        .await
        .unwrap();

        insert_extraction(
            pool,
            &NewExtraction {
                segment_id: seg_id,
                captured_at: 2,
                source: "ocr".into(),
                text: "lorem ipsum dolor".into(),
                confidence: Some(0.91),
                raw_json: None,
            },
        )
        .await
        .unwrap();

        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM extractions WHERE segment_id = ?1")
                .bind(seg_id)
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(count, 1);

        let fts_hits: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM extractions_fts WHERE text MATCH 'lorem'")
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(fts_hits, 1);

        close_segment(pool, seg_id, 99).await.unwrap();
        let ended: Option<i64> =
            sqlx::query_scalar("SELECT ended_at FROM segments WHERE id = ?1")
                .bind(seg_id)
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(ended, Some(99));
    }

    #[tokio::test]
    async fn watermark_upsert() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db")).await.unwrap();
        let pool = store.pool();

        assert_eq!(get_watermark(pool, "whatsapp").await.unwrap(), None);
        set_watermark(pool, "whatsapp", 12345).await.unwrap();
        assert_eq!(get_watermark(pool, "whatsapp").await.unwrap(), Some(12345));
        set_watermark(pool, "whatsapp", 67890).await.unwrap();
        assert_eq!(get_watermark(pool, "whatsapp").await.unwrap(), Some(67890));
    }
}
