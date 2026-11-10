use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AppPhase {
    #[default]
    NoSession,
    Identifying,
    ResolvingLyrics,
    CandidateRequired,
    Analyzing,
    Ready,
    LyricsOnly,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackCapabilities {
    pub can_play_pause: bool,
    pub can_skip_next: bool,
    pub can_skip_previous: bool,
    pub can_seek: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub source_app: String,
    pub title: String,
    pub artist: String,
    pub is_current: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaState {
    pub session_id: String,
    pub source_app: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub position_ms: u64,
    pub sampled_at_ms: i64,
    pub playback_rate: f64,
    pub status: String,
    pub thumbnail_data_url: Option<String>,
    pub timeline_available: bool,
    pub position_estimated: bool,
    pub capabilities: PlaybackCapabilities,
}

impl MediaState {
    pub fn track_signature(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.source_app,
            normalize_key(&self.title),
            normalize_key(&self.artist),
            normalize_key(&self.album)
        )
    }

    pub fn same_track_as(&self, other: &Self) -> bool {
        let artist = normalize_key(&self.artist);
        let other_artist = normalize_key(&other.artist);
        let album = normalize_key(&self.album);
        let other_album = normalize_key(&other.album);
        self.source_app == other.source_app
            && normalize_key(&self.title) == normalize_key(&other.title)
            && (artist.is_empty() || other_artist.is_empty() || artist == other_artist)
            && (album.is_empty() || other_album.is_empty() || album == other_album)
            && (self.duration_ms == 0
                || other.duration_ms == 0
                || self.duration_ms.abs_diff(other.duration_ms) <= 8_000)
    }
}

fn normalize_key(value: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    value
        .nfkc()
        .flat_map(char::to_lowercase)
        .filter(|character| character.is_alphanumeric())
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TrackIdentity {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub source_app: String,
    pub duration_ms: u64,
    pub normalized_title: String,
    pub normalized_artist: String,
    pub normalized_album: String,
    pub versions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WordTiming {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LyricLine {
    pub id: String,
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
    pub text: String,
    pub words: Vec<WordTiming>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SyncType {
    Word,
    Line,
    #[default]
    Plain,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScoreBreakdown {
    pub title: f64,
    pub artist: f64,
    pub duration: f64,
    pub album: f64,
    pub source_affinity: f64,
    pub sync_quality: f64,
    pub penalties: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LyricCandidate {
    pub candidate_id: String,
    pub provider: String,
    pub provider_track_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub instrumental: bool,
    pub sync_type: SyncType,
    pub lines: Vec<LyricLine>,
    pub score: f64,
    pub score_breakdown: ScoreBreakdown,
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artwork_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LyricsDocument {
    pub cache_key: String,
    pub track: TrackIdentity,
    pub provider: String,
    pub provider_track_id: String,
    pub sync_type: SyncType,
    pub instrumental: bool,
    pub offset_ms: i64,
    pub lines: Vec<LyricLine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artwork_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GrammarPoint {
    pub surface: String,
    pub pattern: String,
    pub level: String,
    pub meaning_zh: String,
    pub explanation_zh: String,
    pub conjugation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RubySegment {
    pub text: String,
    pub reading: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LineAnalysis {
    pub line_id: String,
    pub translation_zh: String,
    pub reading: String,
    #[serde(default)]
    pub furigana: Vec<RubySegment>,
    pub literal_structure: String,
    pub grammar_points: Vec<GrammarPoint>,
    pub omitted_elements: Vec<String>,
    pub nuance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PublicSettings {
    pub deepseek_model: String,
    pub learning_level: String,
    pub reduce_motion: bool,
    pub ambient_motion_enabled: bool,
    pub lyrics_furigana_enabled: bool,
    pub mini_always_on_top: bool,
    pub monitoring_enabled: bool,
    pub experimental_netease: bool,
    pub experimental_qq: bool,
    pub provider_contact: String,
    pub has_api_key: bool,
}

impl Default for PublicSettings {
    fn default() -> Self {
        Self {
            deepseek_model: "deepseek-v4-flash".into(),
            learning_level: "N4-N2".into(),
            reduce_motion: false,
            ambient_motion_enabled: true,
            lyrics_furigana_enabled: false,
            mini_always_on_top: true,
            monitoring_enabled: true,
            experimental_netease: true,
            experimental_qq: true,
            provider_contact: String::new(),
            has_api_key: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub phase: AppPhase,
    pub media: Option<MediaState>,
    pub sessions: Vec<SessionSummary>,
    pub lyrics: Option<LyricsDocument>,
    pub candidates: Vec<LyricCandidate>,
    pub analyses: HashMap<String, LineAnalysis>,
    pub settings: PublicSettings,
    pub message: Option<String>,
    pub generation: u64,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub selected_source: Option<String>,
    #[serde(default)]
    pub notices: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub deepseek_model: Option<String>,
    pub learning_level: Option<String>,
    pub reduce_motion: Option<bool>,
    pub ambient_motion_enabled: Option<bool>,
    pub lyrics_furigana_enabled: Option<bool>,
    pub mini_always_on_top: Option<bool>,
    pub monitoring_enabled: Option<bool>,
    pub experimental_netease: Option<bool>,
    pub experimental_qq: Option<bool>,
    pub provider_contact: Option<String>,
}

#[cfg(test)]
mod settings_tests {
    use super::PublicSettings;

    #[test]
    fn older_saved_settings_enable_ambient_motion_by_default() {
        let json = r#"{
            "deepseekModel":"deepseek-v4-flash",
            "learningLevel":"N4-N2",
            "reduceMotion":false,
            "miniAlwaysOnTop":true,
            "monitoringEnabled":true,
            "experimentalNetease":true,
            "experimentalQq":true,
            "hasApiKey":false
        }"#;

        let settings: PublicSettings = serde_json::from_str(json).unwrap();

        assert!(settings.ambient_motion_enabled);
        assert!(!settings.lyrics_furigana_enabled);
        let enabled = PublicSettings {
            lyrics_furigana_enabled: true,
            ..settings
        };
        let restored: PublicSettings =
            serde_json::from_str(&serde_json::to_string(&enabled).unwrap()).unwrap();
        assert!(restored.lyrics_furigana_enabled);
    }
}
