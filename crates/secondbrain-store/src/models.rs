//! Row types matching the schema in `migrations/0001_initial.sql`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub id: i64,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub app_bundle: String,
    pub app_name: String,
    pub window_title: Option<String>,
    pub url: Option<String>,
    pub monitor_id: Option<i64>,
    pub focused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewSegment {
    pub started_at: i64,
    pub app_bundle: String,
    pub app_name: String,
    pub window_title: Option<String>,
    pub url: Option<String>,
    pub monitor_id: Option<i64>,
    pub focused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Extraction {
    pub id: i64,
    pub segment_id: i64,
    pub captured_at: i64,
    pub source: String,
    pub text: String,
    pub confidence: Option<f64>,
    pub raw_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewExtraction {
    pub segment_id: i64,
    pub captured_at: i64,
    pub source: String,
    pub text: String,
    pub confidence: Option<f64>,
    pub raw_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: i64,
    pub slug: String,
    pub title: String,
    pub body_md: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub source_segment_ids: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watermark {
    pub extractor: String,
    pub watermark: i64,
    pub updated_at: i64,
}
