//! Pure lifecycle rules, kept independent of Tauri so races can be regression-tested.
use crate::models::{AppSnapshot, PublicSettings, SettingsPatch};
use anyhow::{bail, Result};

#[derive(Clone, Debug)]
pub struct AnalysisStamp {
    pub generation: u64,
    pub epoch: u64,
    pub document_key: String,
    pub model: String,
    pub level: String,
}

impl AnalysisStamp {
    pub fn is_current(&self, snapshot: &AppSnapshot, epoch: u64) -> bool {
        self.generation == snapshot.generation
            && self.epoch == epoch
            && snapshot
                .lyrics
                .as_ref()
                .is_some_and(|document| document.cache_key == self.document_key)
            && snapshot.settings.deepseek_model == self.model
            && snapshot.settings.learning_level == self.level
            && snapshot.settings.has_api_key
    }
}

pub fn patched_settings(current: &PublicSettings, patch: SettingsPatch) -> Result<PublicSettings> {
    let mut settings = current.clone();
    if let Some(contact) = patch.provider_contact {
        let contact = contact.trim();
        if !contact.is_empty() && !crate::lyrics::providers::valid_provider_contact(contact) {
            bail!("请输入有效的公开 HTTPS 项目链接或联系邮箱");
        }
        settings.provider_contact = contact.to_owned();
    }
    if let Some(model) = patch.deepseek_model {
        if !matches!(model.as_str(), "deepseek-v4-flash" | "deepseek-v4-pro") {
            bail!("不支持的分析模型");
        }
        settings.deepseek_model = model;
    }
    if let Some(level) = patch.learning_level {
        if !matches!(level.as_str(), "N4-N2" | "N4" | "N3" | "N2") {
            bail!("不支持的学习等级");
        }
        settings.learning_level = level;
    }
    macro_rules! apply { ($($name:ident),*) => { $(if let Some(value) = patch.$name { settings.$name = value; })* }; }
    apply!(
        reduce_motion,
        ambient_motion_enabled,
        lyrics_furigana_enabled,
        mini_always_on_top,
        monitoring_enabled,
        experimental_netease,
        experimental_qq
    );
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::LyricsDocument;

    fn active() -> (AppSnapshot, AnalysisStamp) {
        let mut snapshot = AppSnapshot {
            generation: 4,
            lyrics: Some(LyricsDocument {
                cache_key: "song-a".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        snapshot.settings.has_api_key = true;
        let stamp = AnalysisStamp {
            generation: 4,
            epoch: 2,
            document_key: "song-a".into(),
            model: snapshot.settings.deepseek_model.clone(),
            level: snapshot.settings.learning_level.clone(),
        };
        (snapshot, stamp)
    }

    #[test]
    fn rejects_delayed_results_after_track_switch_rematch_retry_or_key_clear() {
        let (mut snapshot, stamp) = active();
        assert!(stamp.is_current(&snapshot, 2));
        assert!(!stamp.is_current(&snapshot, 3));
        snapshot.generation += 1;
        assert!(!stamp.is_current(&snapshot, 2));
        snapshot.generation -= 1;
        snapshot.lyrics.as_mut().unwrap().cache_key = "other-selection".into();
        assert!(!stamp.is_current(&snapshot, 2));
        snapshot.lyrics.as_mut().unwrap().cache_key = "song-a".into();
        snapshot.settings.has_api_key = false;
        assert!(!stamp.is_current(&snapshot, 2));
    }

    #[test]
    fn model_and_teaching_profile_changes_invalidate_inflight_work() {
        let (mut snapshot, stamp) = active();
        snapshot.settings.deepseek_model = "deepseek-v4-pro".into();
        assert!(!stamp.is_current(&snapshot, 2));
        snapshot.settings.deepseek_model = stamp.model.clone();
        snapshot.settings.learning_level = "N4".into();
        assert!(!stamp.is_current(&snapshot, 2));
    }

    #[test]
    fn rejects_unrecognized_profile_without_changing_existing_settings() {
        let current = PublicSettings::default();
        let patch = serde_json::from_str(r#"{"deepseekModel":"invalid"}"#).unwrap();
        assert!(patched_settings(&current, patch).is_err());
        let patch = serde_json::from_str(r#"{"learningLevel":"arbitrary prompt"}"#).unwrap();
        assert!(patched_settings(&current, patch).is_err());
        let patch = serde_json::from_str(r#"{"ambientMotionEnabled":false}"#).unwrap();
        let updated = patched_settings(&current, patch).unwrap();
        assert!(!updated.ambient_motion_enabled);
        assert_eq!(updated.deepseek_model, current.deepseek_model);
    }
}
