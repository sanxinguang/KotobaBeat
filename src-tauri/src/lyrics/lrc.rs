use crate::models::{LyricLine, WordTiming};
use once_cell::sync::Lazy;
use regex::Regex;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

static TIMESTAMP: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[(\d{1,3}):(\d{2})(?:[.:](\d{1,3}))?\]").unwrap());
static METADATA: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\[(ar|al|ti|by|offset|re|ve):").unwrap());

pub fn parse_lrc(input: &str) -> Vec<LyricLine> {
    let mut parsed: Vec<(u64, String)> = Vec::new();
    let mut plain: Vec<String> = Vec::new();

    for raw in input.lines() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || METADATA.is_match(&line.to_lowercase()) {
            continue;
        }

        let timestamps: Vec<u64> = TIMESTAMP
            .captures_iter(line)
            .filter_map(|capture| {
                let minutes = capture.get(1)?.as_str().parse::<u64>().ok()?;
                let seconds = capture.get(2)?.as_str().parse::<u64>().ok()?;
                let fraction = capture.get(3).map(|part| part.as_str()).unwrap_or("0");
                let millis = match fraction.len() {
                    1 => fraction.parse::<u64>().ok()? * 100,
                    2 => fraction.parse::<u64>().ok()? * 10,
                    _ => fraction.get(..3)?.parse::<u64>().ok()?,
                };
                Some(minutes * 60_000 + seconds * 1_000 + millis)
            })
            .collect();

        let text = TIMESTAMP.replace_all(line, "").trim().to_string();
        if timestamps.is_empty() {
            if !text.is_empty() {
                plain.push(text);
            }
        } else if !text.is_empty() {
            for timestamp in timestamps {
                parsed.push((timestamp, text.clone()));
            }
        }
    }

    let mut lines: Vec<LyricLine> = if parsed.is_empty() {
        plain
            .into_iter()
            .map(|text| LyricLine {
                id: String::new(),
                start_ms: None,
                end_ms: None,
                text,
                words: Vec::<WordTiming>::new(),
            })
            .collect()
    } else {
        parsed.sort_by_key(|(timestamp, _)| *timestamp);
        parsed
            .iter()
            .enumerate()
            .map(|(index, (timestamp, text))| LyricLine {
                id: String::new(),
                start_ms: Some(*timestamp),
                end_ms: parsed.get(index + 1).map(|next| next.0),
                text: text.clone(),
                words: Vec::new(),
            })
            .collect()
    };
    assign_stable_line_ids(&mut lines);
    lines
}

/// Content plus occurrence, not timestamps or a fresh UUID. Repeated choruses
/// remain separate lines while a rematch, retiming or unrelated insertion does
/// not invalidate all existing analyses.
pub fn assign_stable_line_ids(lines: &mut [LyricLine]) {
    let mut occurrences = HashMap::<String, usize>::new();
    for line in lines {
        let occurrence = occurrences.entry(line.text.clone()).or_default();
        let digest = format!("{:x}", Sha256::digest(line.text.as_bytes()));
        line.id = format!("line-v2-{}-{occurrence}", &digest[..32]);
        *occurrence += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_timestamps_and_fraction_precision() {
        let lines = parse_lrc("[00:01.2][00:03.45]君の声\n[00:05.678]聞こえる");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].start_ms, Some(1_200));
        assert_eq!(lines[1].start_ms, Some(3_450));
        assert_eq!(lines[2].start_ms, Some(5_678));
        assert_eq!(lines[0].text, "君の声");
    }

    #[test]
    fn keeps_plain_lyrics_without_fake_timestamps() {
        let lines = parse_lrc("春が来る\n君に会える");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].start_ms, None);
    }

    #[test]
    fn ids_are_stable_across_rematches_and_timestamp_corrections() {
        let first = parse_lrc("[00:01.00]君の声\n[00:03.00]聞こえる");
        let repeat = parse_lrc("[00:01.00]君の声\n[00:03.00]聞こえる");
        let retimed = parse_lrc("[00:02.00]君の声\n[00:04.00]聞こえる");
        let ids = |lines: &[LyricLine]| lines.iter().map(|l| l.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&first), ids(&repeat));
        assert_eq!(ids(&first), ids(&retimed));
    }

    #[test]
    fn duplicate_lines_are_distinct_and_insertions_do_not_renumber_other_text() {
        let original = parse_lrc("君の声\n聞こえる\n君の声");
        let inserted = parse_lrc("新しい行\n君の声\n聞こえる\n君の声");
        assert_ne!(original[0].id, original[2].id);
        for (old, new) in original.iter().zip(&inserted[1..]) {
            assert_eq!(old.id, new.id);
        }
    }
}
