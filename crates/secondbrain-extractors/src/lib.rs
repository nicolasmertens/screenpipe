//! secondbrain extractors.
//!
//! Each extractor is a small task that reads from one external source
//! (WhatsApp, iMessage, Mail, ...) and writes new rows into the
//! secondbrain store. Extractors are incremental: they track a
//! per-source watermark in `extractor_watermarks` and only ingest
//! data created after it.

pub mod whatsapp;

/// Apple Core Data reference date: 2001-01-01 00:00:00 UTC.
/// WhatsApp on macOS stores timestamps as seconds since this epoch
/// (matching iOS Core Data behavior).
pub const APPLE_EPOCH_OFFSET_SECS: i64 = 978_307_200;

/// Convert an Apple Core Data timestamp (seconds since 2001-01-01)
/// to a Unix timestamp in milliseconds.
pub fn apple_to_unix_ms(apple_seconds: f64) -> i64 {
    ((apple_seconds + APPLE_EPOCH_OFFSET_SECS as f64) * 1000.0) as i64
}

/// Inverse of `apple_to_unix_ms`. Used when we need to compare a stored
/// Unix watermark back against ZMESSAGEDATE.
pub fn unix_ms_to_apple(unix_ms: i64) -> f64 {
    (unix_ms as f64) / 1000.0 - APPLE_EPOCH_OFFSET_SECS as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_round_trip() {
        let apple = 798_838_909.0;
        let unix_ms = apple_to_unix_ms(apple);
        let back = unix_ms_to_apple(unix_ms);
        assert!((back - apple).abs() < 0.001);
    }

    #[test]
    fn known_apple_timestamp_decodes() {
        // Apple epoch == Unix 978307200
        let unix_ms = apple_to_unix_ms(0.0);
        assert_eq!(unix_ms, 978_307_200_000);
    }
}
