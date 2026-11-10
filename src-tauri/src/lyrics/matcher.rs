use crate::models::{LyricCandidate, MediaState, ScoreBreakdown, SyncType, TrackIdentity};
use once_cell::sync::Lazy;
use regex::Regex;
use strsim::jaro_winkler;
use unicode_normalization::UnicodeNormalization;

static BRACKET_SUFFIX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\s*[\(\[（【].*?[\)\]）】]\s*$").unwrap());
static FEAT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\s+(feat\.?|ft\.?|featuring)\s+.*$").unwrap());
static PUNCTUATION: Lazy<Regex> = Lazy::new(|| Regex::new(r"[\s\p{P}\p{S}]+").unwrap());
static ARTIST_SEPARATOR: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\s*[/／,，;&、]\s*|\s+(?:feat\.?|ft\.?|featuring)\s+").unwrap());

const VERSION_TAGS: &[(&str, &[&str])] = &[
    ("live", &["live", "ライブ", "现场", "現場"]),
    ("remix", &["remix", "リミックス"]),
    (
        "acoustic",
        &[
            "acoustic",
            "unplugged",
            "アコースティック",
            "不插电",
            "不插電",
        ],
    ),
    ("cover", &["cover", "カバー", "歌ってみた", "翻唱"]),
    (
        "instrumental",
        &[
            "instrumental",
            "off vocal",
            "off-vocal",
            "karaoke",
            "伴奏",
            "カラオケ",
            "オフボーカル",
        ],
    ),
    (
        "tv_size",
        &[
            "tv size",
            "tv-size",
            "tvsize",
            "tv version",
            "tv ver",
            "テレビサイズ",
            "tvサイズ",
        ],
    ),
];

pub fn normalize(value: &str) -> String {
    let normalized: String = value.nfkc().collect::<String>().to_lowercase();
    PUNCTUATION.replace_all(&normalized, "").to_string()
}

pub fn search_variants(value: &str) -> Vec<String> {
    let normalized: String = value.nfkc().collect();
    let mut values = vec![value.trim().to_string()];
    if normalized.trim() != value.trim() {
        values.push(normalized.trim().to_string());
    }
    let no_feat = FEAT.replace(&normalized, "").trim().to_string();
    let no_bracket = BRACKET_SUFFIX.replace(&no_feat, "").trim().to_string();
    for item in [no_feat, no_bracket] {
        if !item.is_empty() && !values.iter().any(|existing| existing == &item) {
            values.push(item);
        }
    }
    values
}

pub fn version_tags(value: &str) -> Vec<String> {
    let lower = value.nfkc().collect::<String>().to_lowercase();
    VERSION_TAGS
        .iter()
        .filter_map(|(tag, needles)| {
            needles
                .iter()
                .any(|needle| contains_version_word(&lower, needle))
                .then_some((*tag).to_string())
        })
        .collect()
}

fn contains_version_word(value: &str, needle: &str) -> bool {
    // Latin labels are words, not arbitrary substrings: "believe", "alive"
    // and "discovery" must never turn into Live/Cover versions.
    if !needle.is_ascii() {
        return value.contains(needle);
    }
    value.match_indices(needle).any(|(index, _)| {
        let before = value[..index].chars().next_back();
        let after = value[index + needle.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphanumeric())
            && !after.is_some_and(|c| c.is_ascii_alphanumeric())
    })
}

fn recording_versions(title: &str, album: &str) -> Vec<String> {
    let mut versions = version_tags(title);
    versions.extend(version_tags(album));
    versions.sort();
    versions.dedup();
    versions
}

pub fn identity_from_media(media: &MediaState) -> TrackIdentity {
    TrackIdentity {
        title: media.title.clone(),
        artist: media.artist.clone(),
        album: media.album.clone(),
        source_app: media.source_app.clone(),
        duration_ms: media.duration_ms,
        normalized_title: normalize(&media.title),
        normalized_artist: normalize(&media.artist),
        normalized_album: normalize(&media.album),
        versions: recording_versions(&media.title, &media.album),
    }
}

fn text_score(left: &str, right: &str) -> f64 {
    let left = normalize(left);
    let right = normalize(right);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    if left == right {
        return 1.0;
    }
    jaro_winkler(&left, &right)
}

fn title_score(left: &str, right: &str) -> f64 {
    // Search-friendly annotations may differ between services. Version
    // compatibility is checked independently and is mandatory for auto-match.
    search_variants(left)
        .iter()
        .flat_map(|left| {
            search_variants(right)
                .into_iter()
                .map(move |right| text_score(left, &right))
        })
        .fold(0.0, f64::max)
}

fn artist_score(left: &str, right: &str) -> f64 {
    let mut left_parts: Vec<String> = ARTIST_SEPARATOR
        .split(left)
        .map(normalize)
        .filter(|s| !s.is_empty())
        .collect();
    let mut right_parts: Vec<String> = ARTIST_SEPARATOR
        .split(right)
        .map(normalize)
        .filter(|s| !s.is_empty())
        .collect();
    left_parts.sort();
    left_parts.dedup();
    right_parts.sort();
    right_parts.dedup();
    if !left_parts.is_empty() && left_parts == right_parts {
        return 1.0;
    }
    // Do not give a guest artist the same confidence as the complete credit.
    text_score(left, right)
}

fn duration_score(expected: u64, actual: u64) -> f64 {
    if expected == 0 || actual == 0 {
        return 0.6;
    }
    let delta = expected.abs_diff(actual) as f64 / 1_000.0;
    match delta {
        value if value <= 2.0 => 1.0,
        value if value <= 8.0 => 1.0 - (value - 2.0) / 10.0,
        value if value <= 15.0 => 0.4 - (value - 8.0) / 20.0,
        _ => 0.0,
    }
}

fn source_affinity(source_app: &str, provider: &str) -> f64 {
    let source = source_app.to_lowercase();
    let provider = provider.to_lowercase();
    if ((source.contains("cloudmusic") || source.contains("netease"))
        && provider.contains("netease"))
        || (source.contains("qqmusic") && provider.contains("qq"))
    {
        1.0
    } else {
        0.45
    }
}

fn version_penalty(track: &TrackIdentity, candidate: &LyricCandidate) -> f64 {
    let expected = recording_versions(&track.title, &track.album);
    let actual = recording_versions(&candidate.title, &candidate.album);
    if expected == actual {
        0.0
    } else {
        0.2
    }
}

pub fn score_candidate(track: &TrackIdentity, candidate: &mut LyricCandidate) {
    let title = title_score(&track.title, &candidate.title);
    let artist = artist_score(&track.artist, &candidate.artist);
    let duration = duration_score(track.duration_ms, candidate.duration_ms);
    let album = if track.album.is_empty() || candidate.album.is_empty() {
        0.5
    } else {
        text_score(&track.album, &candidate.album)
    };
    let affinity = source_affinity(&track.source_app, &candidate.provider);
    let sync = match candidate.sync_type {
        SyncType::Word => 1.0,
        SyncType::Line => 0.85,
        SyncType::Plain => 0.35,
    };
    let mut penalties = version_penalty(track, candidate);
    if track.duration_ms > 0
        && candidate.duration_ms > 0
        && track.duration_ms.abs_diff(candidate.duration_ms) > 15_000
    {
        penalties += 0.25;
    }
    if track.duration_ms > 0 {
        let last_timestamp = candidate
            .lines
            .iter()
            .filter_map(|line| line.start_ms)
            .max()
            .unwrap_or(0);
        if last_timestamp > 0 && last_timestamp < track.duration_ms.saturating_mul(7) / 10 {
            penalties += 0.1;
        }
    }

    let total = title * 0.36
        + artist * 0.22
        + duration * 0.22
        + album * 0.08
        + affinity * 0.07
        + sync * 0.05
        - penalties;

    candidate.score = total.clamp(0.0, 1.0);
    candidate.score_breakdown = ScoreBreakdown {
        title,
        artist,
        duration,
        album,
        source_affinity: affinity,
        sync_quality: sync,
        penalties,
    };
}

pub fn rank_candidates(
    track: &TrackIdentity,
    mut candidates: Vec<LyricCandidate>,
) -> Vec<LyricCandidate> {
    for candidate in &mut candidates {
        score_candidate(track, candidate);
        if version_penalty(track, candidate) > 0.0 {
            candidate.warnings.push("版本不同".into());
        }
        if candidate.score < 0.68 {
            candidate.warnings.push("匹配度较低".into());
        }
        candidate.warnings.sort();
        candidate.warnings.dedup();
    }
    candidates.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    // Multiple search variants can return the same platform record. Retain
    // cross-platform records, including their alternative artwork, for users.
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|candidate| {
        seen.insert((
            candidate.provider.clone(),
            candidate.provider_track_id.clone(),
        ))
    });
    candidates
}

fn same_recording(left: &LyricCandidate, right: &LyricCandidate) -> bool {
    !left.lines.is_empty()
        && title_score(&left.title, &right.title) == 1.0
        && artist_score(&left.artist, &right.artist) == 1.0
        && recording_versions(&left.title, &left.album)
            == recording_versions(&right.title, &right.album)
        && left.duration_ms > 0
        && right.duration_ms > 0
        && left.duration_ms.abs_diff(right.duration_ms) <= 2_000
        && left.lines.len() == right.lines.len()
        && left
            .lines
            .iter()
            .zip(&right.lines)
            .all(|(left, right)| normalize(&left.text) == normalize(&right.text))
}

pub fn can_auto_select(track: &TrackIdentity, ranked: &[LyricCandidate]) -> bool {
    let Some(best) = ranked.first() else {
        return false;
    };
    let gap = ranked
        .iter()
        .skip(1)
        .filter(|candidate| !same_recording(best, candidate))
        .map(|candidate| candidate.score)
        .reduce(f64::max)
        .map(|score| best.score - score)
        .unwrap_or(1.0);
    let duration_ok = track.duration_ms > 0
        && best.duration_ms > 0
        && track.duration_ms.abs_diff(best.duration_ms) <= 8_000;
    best.score >= 0.84
        && best.score_breakdown.title >= 0.92
        && best.score_breakdown.artist >= 0.75
        && duration_ok
        && version_penalty(track, best) == 0.0
        && gap >= 0.07
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::LyricLine;

    fn candidate(title: &str, artist: &str, duration_ms: u64) -> LyricCandidate {
        LyricCandidate {
            candidate_id: "one".into(),
            provider: "lrclib".into(),
            provider_track_id: "1".into(),
            title: title.into(),
            artist: artist.into(),
            album: "残響散歌".into(),
            duration_ms,
            sync_type: SyncType::Line,
            lines: vec![LyricLine {
                start_ms: Some(150_000),
                text: "声を枯らして".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn normalizes_full_width_and_punctuation() {
        assert_eq!(normalize("Ａｉｍｅｒ　— 残響"), normalize("aimer-残響"));
    }

    #[test]
    fn strips_search_suffix_without_losing_original_variant() {
        let variants = search_variants("花の塔 (TV Size) feat. someone");
        assert!(variants.contains(&"花の塔".to_string()));
    }

    #[test]
    fn penalizes_wrong_versions() {
        let track = TrackIdentity {
            title: "花の塔".into(),
            artist: "さユり".into(),
            album: "花の塔".into(),
            source_app: "QQMusic".into(),
            duration_ms: 275_000,
            normalized_title: normalize("花の塔"),
            normalized_artist: normalize("さユり"),
            normalized_album: normalize("花の塔"),
            versions: vec![],
        };
        let mut official = candidate("花の塔", "さユり", 275_000);
        let mut live = candidate("花の塔 (Live)", "さユり", 275_000);
        score_candidate(&track, &mut official);
        score_candidate(&track, &mut live);
        assert!(official.score > live.score);
    }

    fn track(title: &str, artist: &str) -> TrackIdentity {
        TrackIdentity {
            title: title.into(),
            artist: artist.into(),
            album: "架空アルバム".into(),
            duration_ms: 200_000,
            source_app: "cloudmusic.exe".into(),
            versions: version_tags(title),
            ..Default::default()
        }
    }

    fn matching_candidate(track: &TrackIdentity) -> LyricCandidate {
        let mut found = candidate(&track.title, &track.artist, track.duration_ms);
        found.provider = "NetEase".into();
        found.album = track.album.clone();
        found
    }

    #[test]
    fn version_tags_are_words_not_substrings() {
        assert!(version_tags("Believe / ALIVE / Discovery").is_empty());
        assert_eq!(version_tags("曲名（20周年Live at 東京）"), vec!["live"]);
        assert_eq!(version_tags("曲名 - TV-Size"), vec!["tv_size"]);
        assert_eq!(version_tags("曲名 off-vocal"), vec!["instrumental"]);
        assert_eq!(version_tags("曲名 翻唱"), vec!["cover"]);
    }

    #[test]
    fn annotations_and_artist_order_can_differ() {
        let identity = track("花 (Official Audio)", "甲 / 乙");
        let mut found = matching_candidate(&identity);
        found.title = "花".into();
        found.artist = "乙 / 甲".into();
        let ranked = rank_candidates(&identity, vec![found]);
        assert!(can_auto_select(&identity, &ranked));
    }

    #[test]
    fn missing_duration_or_artist_requires_confirmation() {
        let mut identity = track("架空の歌", "架空歌手");
        let found = matching_candidate(&identity);
        identity.duration_ms = 0;
        let ranked = rank_candidates(&identity, vec![found.clone()]);
        assert!(!can_auto_select(&identity, &ranked));
        assert!(!ranked[0]
            .warnings
            .iter()
            .any(|warning| warning.contains("时长")));
        identity.duration_ms = 200_000;
        let mut unknown = found.clone();
        unknown.duration_ms = 0;
        assert!(!can_auto_select(
            &identity,
            &rank_candidates(&identity, vec![unknown])
        ));
        identity.artist.clear();
        assert!(!can_auto_select(
            &identity,
            &rank_candidates(&identity, vec![found])
        ));
    }

    #[test]
    fn duplicate_recordings_do_not_hide_the_confidence_gap_but_covers_remain_selectable() {
        let identity = track("架空の歌", "架空歌手");
        let first = matching_candidate(&identity);
        let mut second = first.clone();
        second.provider = "LRCLIB".into();
        second.artwork_url = Some("https://example.test/alternative.jpg".into());
        let ranked = rank_candidates(&identity, vec![first, second]);
        assert_eq!(ranked.len(), 2);
        assert!(can_auto_select(&identity, &ranked));
    }

    #[test]
    fn same_title_different_lyrics_is_still_ambiguous() {
        let identity = track("架空の歌", "架空歌手");
        let first = matching_candidate(&identity);
        let mut second = first.clone();
        second.provider_track_id = "different-recording".into();
        second.lines[0].text = "別の録音".into();
        let ranked = rank_candidates(&identity, vec![first, second]);
        assert!(!can_auto_select(&identity, &ranked));
    }

    #[test]
    fn album_version_labels_block_wrong_recordings() {
        let identity = track("架空の歌", "架空歌手");
        let mut found = matching_candidate(&identity);
        found.album = "Live at Tokyo".into();
        assert!(!can_auto_select(
            &identity,
            &rank_candidates(&identity, vec![found])
        ));
    }

    #[test]
    fn duration_and_score_boundaries_are_conservative() {
        let identity = track("架空の歌", "架空歌手");
        let mut good = matching_candidate(&identity);
        good.duration_ms += 8_001;
        assert!(!can_auto_select(
            &identity,
            &rank_candidates(&identity, vec![good])
        ));
        let mut good = matching_candidate(&identity);
        good.score = 0.84;
        good.score_breakdown.title = 0.92;
        good.score_breakdown.artist = 0.75;
        assert!(can_auto_select(&identity, &[good.clone()]));
        good.score = 0.8399;
        assert!(!can_auto_select(&identity, &[good]));
    }

    #[test]
    fn synthetic_metadata_regression_matrix_120_cases() {
        // Fictional metadata exercises invariants; this is NOT a measured
        // real-world 120-song matching-accuracy benchmark.
        let bases = [
            "架空の歌",
            "東京 夜",
            "光と影",
            "夢への旅",
            "冬の音",
            "夜空中文",
            "Believe",
            "ALIVE",
            "Discovery",
            "ＳＯＮＧ",
        ];
        for (index, base) in bases.iter().enumerate() {
            let identity = track(base, &format!("架空歌手{index}"));
            for case in 0..12 {
                let mut found = matching_candidate(&identity);
                let expected = match case {
                    0 => true,
                    1 => {
                        found.title = format!("{base} (Official Audio)");
                        true
                    }
                    2 => {
                        found.duration_ms += 1_000;
                        true
                    }
                    3 => {
                        found.title = format!("{base} (Live)");
                        false
                    }
                    4 => {
                        found.title = format!("{base} Remix");
                        false
                    }
                    5 => {
                        found.title = format!("{base} (TV Size)");
                        found.duration_ms = 90_000;
                        false
                    }
                    6 => {
                        found.title = format!("{base} (Cover)");
                        false
                    }
                    7 => {
                        found.title = format!("{base} (Acoustic)");
                        false
                    }
                    8 => {
                        found.title = format!("{base} (Instrumental)");
                        false
                    }
                    9 => {
                        found.duration_ms = 0;
                        false
                    }
                    10 => {
                        found.duration_ms += 16_000;
                        false
                    }
                    _ => {
                        found.artist = "Unrelated performer".into();
                        false
                    }
                };
                let ranked = rank_candidates(&identity, vec![found]);
                assert_eq!(
                    can_auto_select(&identity, &ranked),
                    expected,
                    "{base}: case {case}"
                );
            }
        }
    }
}
