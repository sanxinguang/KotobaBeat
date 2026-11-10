use crate::{
    deepseek,
    models::{LineAnalysis, LyricsDocument, PublicSettings},
};
use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

pub struct Database {
    connection: Mutex<Connection>,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)
            .with_context(|| format!("failed to open database at {}", path.display()))?;
        Self::from_connection(connection)
    }

    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        connection.busy_timeout(Duration::from_secs(2))?;
        connection.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS lyrics_cache (
                cache_key TEXT PRIMARY KEY,
                document_json TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS track_mapping (
                track_key TEXT PRIMARY KEY,
                cache_key TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY(cache_key) REFERENCES lyrics_cache(cache_key)
            );

            CREATE TABLE IF NOT EXISTS analysis_cache (
                cache_key TEXT NOT NULL,
                line_id TEXT NOT NULL,
                analysis_json TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY(cache_key, line_id)
            );

            CREATE TABLE IF NOT EXISTS analysis_context_cache (
                context_key TEXT PRIMARY KEY,
                analysis_json TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS legacy_lyrics_documents (
                cache_key TEXT NOT NULL,
                song_scope TEXT NOT NULL,
                document_json TEXT NOT NULL,
                PRIMARY KEY(cache_key, song_scope)
            );

            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value_json TEXT NOT NULL
            );
            "#,
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| anyhow!("本地缓存暂时不可用，请重启应用"))
    }

    pub fn load_settings(&self) -> Result<PublicSettings> {
        let connection = self.connection()?;
        let value = connection
            .query_row(
                "SELECT value_json FROM settings WHERE key = 'public'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        match value {
            // A damaged preferences row must not prevent the application from
            // starting. Keep the row untouched for recovery; credentials live
            // separately and are never inferred from this flag.
            Some(json) => Ok(serde_json::from_str(&json).unwrap_or_default()),
            None => Ok(PublicSettings::default()),
        }
    }

    pub fn save_settings(&self, settings: &PublicSettings) -> Result<()> {
        let json = serde_json::to_string(settings)?;
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO settings(key, value_json) VALUES('public', ?1)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
            [json],
        )?;
        Ok(())
    }

    pub fn lyrics_for_track(&self, track_key: &str) -> Result<Option<LyricsDocument>> {
        let connection = self.connection()?;
        let json = connection
            .query_row(
                "SELECT lyrics_cache.document_json
                 FROM track_mapping
                 JOIN lyrics_cache USING(cache_key)
                 WHERE track_mapping.track_key = ?1",
                [track_key],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        // Treat a malformed cache entry as a miss so online/manual fallback is
        // still usable. Do not delete the user's data automatically.
        Ok(json.and_then(|value| serde_json::from_str(&value).ok()))
    }

    pub fn save_lyrics(&self, track_key: &str, document: &LyricsDocument) -> Result<()> {
        let json = serde_json::to_string(document)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let old_json = transaction
            .query_row(
                "SELECT document_json FROM lyrics_cache WHERE cache_key = ?1",
                [&document.cache_key],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(old) = old_json
            .as_ref()
            .and_then(|json| serde_json::from_str::<LyricsDocument>(json).ok())
        {
            if old.lines.iter().any(|line| is_legacy_line_id(&line.id)) {
                // Retain only pre-upgrade source-ID metadata, once per document
                // and song. Other models/profiles can migrate lazily later.
                transaction.execute(
                    "INSERT INTO legacy_lyrics_documents(cache_key, song_scope, document_json) VALUES(?1, ?2, ?3)
                     ON CONFLICT(cache_key, song_scope) DO NOTHING",
                    params![old.cache_key, deepseek::song_analysis_scope(&old), old_json],
                )?;
            }
        }
        transaction.execute(
            "INSERT INTO lyrics_cache(cache_key, document_json) VALUES(?1, ?2)
             ON CONFLICT(cache_key) DO UPDATE SET document_json = excluded.document_json",
            params![document.cache_key, json],
        )?;
        transaction.execute(
            "INSERT INTO track_mapping(track_key, cache_key) VALUES(?1, ?2)
             ON CONFLICT(track_key) DO UPDATE SET
                cache_key = excluded.cache_key,
                updated_at = CURRENT_TIMESTAMP",
            params![track_key, document.cache_key],
        )?;
        transaction.commit()?;
        Ok(())
    }

    #[cfg(test)]
    pub fn load_analyses(&self, cache_key: &str) -> Result<Vec<LineAnalysis>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT analysis_json FROM analysis_cache WHERE cache_key = ?1 ORDER BY rowid",
        )?;
        let rows = statement.query_map([cache_key], |row| row.get::<_, String>(0))?;
        let mut analyses = Vec::new();
        for row in rows {
            if let Some(analysis) = decode_analysis(&row?) {
                analyses.push(analysis);
            }
        }
        Ok(analyses)
    }

    #[cfg(test)]
    pub fn save_analysis(&self, cache_key: &str, analysis: &LineAnalysis) -> Result<()> {
        let json = serde_json::to_string(analysis)?;
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO analysis_cache(cache_key, line_id, analysis_json) VALUES(?1, ?2, ?3)
             ON CONFLICT(cache_key, line_id) DO UPDATE SET analysis_json = excluded.analysis_json",
            params![cache_key, analysis.line_id, json],
        )?;
        Ok(())
    }

    /// Resolve analyses against the current document, including safe promotion
    /// of the old provider-specific cache. Only the current line IDs escape to
    /// the UI, so stale rows cannot make an incomplete song appear complete.
    pub fn load_document_analyses(
        &self,
        document: &LyricsDocument,
        model: &str,
        learning_level: &str,
    ) -> Result<Vec<LineAnalysis>> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        // Prefer running this before save_lyrics replaces a freshly rematched
        // document. Retained legacy metadata also permits lazy migration of
        // other model profiles later. Never infer source text from an ID index.
        let song_scope = deepseek::song_analysis_scope(document);
        let matches_document = |old: &LyricsDocument| {
            deepseek::song_analysis_scope(old) == song_scope
                && old.lines.len() == document.lines.len()
                && old
                    .lines
                    .iter()
                    .zip(&document.lines)
                    .all(|(a, b)| a.text == b.text)
        };
        let current_original = transaction
            .query_row(
                "SELECT document_json FROM lyrics_cache WHERE cache_key = ?1",
                [&document.cache_key],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .and_then(|json| serde_json::from_str::<LyricsDocument>(&json).ok())
            .filter(matches_document);
        let retained_original = transaction.query_row(
            "SELECT document_json FROM legacy_lyrics_documents WHERE cache_key = ?1 AND song_scope = ?2",
            params![document.cache_key, song_scope],
            |row| row.get::<_, String>(0),
        ).optional()?.and_then(|json| serde_json::from_str::<LyricsDocument>(&json).ok())
            .filter(matches_document);
        let legacy_key = deepseek::analysis_cache_key(document, model, learning_level);
        let mut analyses = Vec::new();
        for (index, line) in document.lines.iter().enumerate() {
            let context_key =
                deepseek::line_analysis_cache_key(document, index, model, learning_level);
            let cached = transaction
                .query_row(
                    "SELECT analysis_json FROM analysis_context_cache WHERE context_key = ?1",
                    [&context_key],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .and_then(|json| decode_analysis(&json));
            let context_hit = cached.is_some();
            let legacy = if cached.is_none() {
                let mut found = None;
                for original in [current_original.as_ref(), retained_original.as_ref()]
                    .into_iter()
                    .flatten()
                {
                    let old = &original.lines[index];
                    found = transaction.query_row(
                        "SELECT analysis_json FROM analysis_cache WHERE cache_key = ?1 AND line_id = ?2",
                        params![legacy_key, old.id],
                        |row| row.get::<_, String>(0),
                    ).optional()?.and_then(|json| decode_analysis(&json))
                        .filter(|analysis| analysis.line_id == old.id);
                    if found.is_some() {
                        break;
                    }
                }
                found
            } else {
                None
            };
            let Some(mut analysis) = cached.or(legacy) else {
                continue;
            };
            analysis.line_id = line.id.clone();
            // Old annotation defects should degrade just the ruby, never the
            // usable translation or the whole cache.
            if analysis
                .furigana
                .iter()
                .map(|s| s.text.as_str())
                .collect::<String>()
                != line.text
            {
                analysis.furigana.clear();
            }
            if !context_hit {
                transaction.execute(
                    "INSERT INTO analysis_context_cache(context_key, analysis_json) VALUES(?1, ?2)
                     ON CONFLICT(context_key) DO UPDATE SET analysis_json = excluded.analysis_json,
                         updated_at = CURRENT_TIMESTAMP",
                    params![context_key, serde_json::to_string(&analysis)?],
                )?;
            }
            analyses.push(analysis);
        }
        transaction.commit()?;
        Ok(analyses)
    }

    pub fn save_document_analysis(
        &self,
        document: &LyricsDocument,
        model: &str,
        learning_level: &str,
        analysis: &LineAnalysis,
    ) -> Result<()> {
        let Some(index) = document
            .lines
            .iter()
            .position(|line| line.id == analysis.line_id)
        else {
            return Err(anyhow!("分析结果不属于当前歌词"));
        };
        if analysis.translation_zh.trim().is_empty() {
            return Err(anyhow!("分析结果没有有效译文"));
        }
        let context_key = deepseek::line_analysis_cache_key(document, index, model, learning_level);
        let json = serde_json::to_string(analysis)?;
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO analysis_context_cache(context_key, analysis_json) VALUES(?1, ?2)
             ON CONFLICT(context_key) DO UPDATE SET analysis_json = excluded.analysis_json,
                 updated_at = CURRENT_TIMESTAMP",
            params![context_key, json],
        )?;
        Ok(())
    }

    #[cfg(test)]
    fn clear_track_mapping(&self, track_key: &str) -> Result<()> {
        let connection = self.connection()?;
        connection.execute(
            "DELETE FROM track_mapping WHERE track_key = ?1",
            [track_key],
        )?;
        Ok(())
    }
}

fn decode_analysis(json: &str) -> Option<LineAnalysis> {
    serde_json::from_str::<LineAnalysis>(json)
        .ok()
        .filter(|analysis| {
            !analysis.line_id.is_empty() && !analysis.translation_zh.trim().is_empty()
        })
}

fn is_legacy_line_id(id: &str) -> bool {
    let Some(remainder) = id.strip_prefix("line-") else {
        return false;
    };
    let Some((index, uuid)) = remainder.split_once('-') else {
        return false;
    };
    index.parse::<usize>().is_ok()
        && uuid.len() == 32
        && uuid.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lyrics::lrc::parse_lrc, models::TrackIdentity};
    use sha2::{Digest, Sha256};

    const MODEL: &str = "deepseek-v4-flash";
    const LEVEL: &str = "N4-N2";

    fn database() -> (tempfile::TempDir, Database) {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::open(&directory.path().join("test.sqlite3")).unwrap();
        (directory, database)
    }

    fn document(raw: &str) -> LyricsDocument {
        let lines = parse_lrc(raw);
        let mut hash = Sha256::new();
        hash.update(b"TestProvider");
        hash.update(b"song-1");
        for line in &lines {
            hash.update(line.text.as_bytes());
        }
        LyricsDocument {
            cache_key: format!("{:x}", hash.finalize()),
            track: TrackIdentity {
                title: "テストの歌".into(),
                artist: "テスト歌手".into(),
                ..Default::default()
            },
            provider: "TestProvider".into(),
            provider_track_id: "song-1".into(),
            lines,
            ..Default::default()
        }
    }

    fn analysis(document: &LyricsDocument, index: usize) -> LineAnalysis {
        LineAnalysis {
            line_id: document.lines[index].id.clone(),
            translation_zh: format!("第 {index} 行的译文"),
            ..Default::default()
        }
    }

    fn save_all(database: &Database, document: &LyricsDocument) {
        database.save_lyrics("track", document).unwrap();
        for index in 0..document.lines.len() {
            database
                .save_document_analysis(document, MODEL, LEVEL, &analysis(document, index))
                .unwrap();
        }
    }

    #[test]
    fn temporary_database_still_supports_lyrics_and_settings() {
        let database = Database::in_memory().unwrap();
        let document = document("君の声\n聞こえる");
        save_all(&database, &document);
        database.save_settings(&PublicSettings::default()).unwrap();
        assert_eq!(
            database
                .load_document_analyses(&document, MODEL, LEVEL)
                .unwrap()
                .len(),
            2
        );
        assert!(database.lyrics_for_track("track").unwrap().is_some());
    }

    #[test]
    #[ignore = "manual local performance measurement, no network"]
    fn measure_warm_cache_latency() {
        let (_directory, database) = database();
        let raw = (0..100)
            .map(|i| format!("試験用の歌詞{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let document = document(&raw);
        save_all(&database, &document);
        let mut measurements = Vec::new();
        for _ in 0..30 {
            let start = std::time::Instant::now();
            let saved = database.lyrics_for_track("track").unwrap().unwrap();
            assert_eq!(
                database
                    .load_document_analyses(&saved, MODEL, LEVEL)
                    .unwrap()
                    .len(),
                100
            );
            measurements.push(start.elapsed().as_micros());
        }
        measurements.sort_unstable();
        println!(
            "100-line lyric+analysis cache: p50={}us, p95={}us, max={}us",
            measurements[15], measurements[28], measurements[29]
        );
    }

    #[test]
    fn rematch_and_retiming_reuse_complete_analysis_cache() {
        let (_directory, database) = database();
        let first = document("[00:01.00]君の声\n[00:03.00]聞こえる");
        save_all(&database, &first);
        database.clear_track_mapping("track").unwrap();
        assert!(database.lyrics_for_track("track").unwrap().is_none());
        let rematched = document("[00:02.00]君の声\n[00:04.00]聞こえる");
        database.save_lyrics("track", &rematched).unwrap();
        let cached = database
            .load_document_analyses(&rematched, MODEL, LEVEL)
            .unwrap();
        assert_eq!(cached.len(), 2);
        assert_eq!(cached[0].line_id, rematched.lines[0].id);
    }

    #[test]
    fn upgrades_legacy_random_ids_before_same_lyrics_are_overwritten() {
        let (_directory, database) = database();
        let new = document("君の声\n聞こえる\n君の声");
        let mut old = new.clone();
        for (index, line) in old.lines.iter_mut().enumerate() {
            line.id = format!("line-{index}-{index:032x}");
        }
        database.save_lyrics("track", &old).unwrap();
        for index in 0..old.lines.len() {
            database
                .save_analysis(
                    &deepseek::analysis_cache_key(&old, MODEL, LEVEL),
                    &analysis(&old, index),
                )
                .unwrap();
        }
        let mut pro_analysis = analysis(&old, 0);
        pro_analysis.translation_zh = "独立模型的译文".into();
        database
            .save_analysis(
                &deepseek::analysis_cache_key(&old, "deepseek-v4-pro", LEVEL),
                &pro_analysis,
            )
            .unwrap();
        database.clear_track_mapping("track").unwrap();
        // Preflight loading is the migration step before replacing old IDs.
        database.load_document_analyses(&new, MODEL, LEVEL).unwrap();
        database.save_lyrics("track", &new).unwrap();
        let cached = database.load_document_analyses(&new, MODEL, LEVEL).unwrap();
        assert_eq!(cached.len(), 3);
        for (index, row) in cached.iter().enumerate() {
            assert_eq!(row.line_id, new.lines[index].id);
            assert_eq!(row.translation_zh, format!("第 {index} 行的译文"));
        }
        let pro = database
            .load_document_analyses(&new, "deepseek-v4-pro", LEVEL)
            .unwrap();
        assert_eq!(pro.len(), 1);
        assert_eq!(pro[0].translation_zh, "独立模型的译文");
    }

    #[test]
    fn legacy_migration_never_guesses_by_index_when_text_changed() {
        let (_directory, database) = database();
        let mut old = document("君の声\n聞こえる");
        old.lines[0].id = format!("line-0-{:032x}", 1);
        database.save_lyrics("track", &old).unwrap();
        database
            .save_analysis(
                &deepseek::analysis_cache_key(&old, MODEL, LEVEL),
                &analysis(&old, 0),
            )
            .unwrap();
        let mut changed = document("別の歌詞\n聞こえる");
        // Defend even against a damaged old content key or a concatenation
        // collision in the legacy document hash.
        changed.cache_key = old.cache_key.clone();
        assert!(database
            .load_document_analyses(&changed, MODEL, LEVEL)
            .unwrap()
            .is_empty());
        database.save_lyrics("track", &changed).unwrap();
        assert!(database
            .load_document_analyses(&changed, MODEL, LEVEL)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn changed_line_invalidates_only_itself_and_its_neighbors() {
        let (_directory, database) = database();
        let first = document("始まり\n朝の空\n君の声\n夜の風\n終わり");
        save_all(&database, &first);
        let changed = document("始まり\n朝の空\n新しい声\n夜の風\n終わり");
        let cached = database
            .load_document_analyses(&changed, MODEL, LEVEL)
            .unwrap();
        assert_eq!(cached.len(), 2);
        assert_eq!(cached[0].line_id, changed.lines[0].id);
        assert_eq!(cached[1].line_id, changed.lines[4].id);
    }

    #[test]
    fn cross_provider_reuse_is_scoped_to_same_song_and_profile() {
        let (_directory, database) = database();
        let first = document("君の声\n聞こえる");
        save_all(&database, &first);
        let mut other = first.clone();
        other.provider = "AnotherProvider".into();
        other.provider_track_id = "different-platform-id".into();
        other.cache_key = "another-document-key".into();
        assert_eq!(
            database
                .load_document_analyses(&other, MODEL, LEVEL)
                .unwrap()
                .len(),
            2
        );
        assert!(database
            .load_document_analyses(&other, MODEL, "N2")
            .unwrap()
            .is_empty());
        assert!(database
            .load_document_analyses(&other, "deepseek-v4-pro", LEVEL)
            .unwrap()
            .is_empty());
        other.track.artist = "別の歌手".into();
        assert!(database
            .load_document_analyses(&other, MODEL, LEVEL)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn malformed_rows_are_misses_not_global_failures() {
        let (_directory, database) = database();
        let doc = document("君の声\n聞こえる");
        save_all(&database, &doc);
        let key = deepseek::analysis_cache_key(&doc, MODEL, LEVEL);
        let context = deepseek::line_analysis_cache_key(&doc, 0, MODEL, LEVEL);
        {
            let connection = database.connection().unwrap();
            connection.execute("UPDATE analysis_cache SET analysis_json = 'not json' WHERE cache_key = ?1 AND line_id = ?2", params![key, doc.lines[0].id]).unwrap();
            connection
                .execute(
                    "UPDATE analysis_context_cache SET analysis_json = '{}' WHERE context_key = ?1",
                    [context],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO settings(key, value_json) VALUES('public', '{broken')",
                    [],
                )
                .unwrap();
            connection
                .execute("UPDATE lyrics_cache SET document_json = 'broken'", [])
                .unwrap();
        }
        let cached = database.load_document_analyses(&doc, MODEL, LEVEL).unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].line_id, doc.lines[1].id);
        assert!(database.load_settings().unwrap().monitoring_enabled);
        assert!(database.lyrics_for_track("track").unwrap().is_none());
        assert!(database.load_analyses(&key).unwrap().is_empty());
    }

    #[test]
    fn invalid_result_ids_and_empty_translations_are_not_cached() {
        let (_directory, database) = database();
        let doc = document("君の声");
        let mut value = analysis(&doc, 0);
        value.line_id = "another-track".into();
        assert!(database
            .save_document_analysis(&doc, MODEL, LEVEL, &value)
            .is_err());
        value.line_id = doc.lines[0].id.clone();
        value.translation_zh = "  ".into();
        assert!(database
            .save_document_analysis(&doc, MODEL, LEVEL, &value)
            .is_err());
        assert!(database
            .load_document_analyses(&doc, MODEL, LEVEL)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn legacy_cache_cannot_cross_song_identity_even_with_same_text_and_key() {
        let (_directory, database) = database();
        let mut old = document("君の声");
        old.lines[0].id = format!("line-0-{:032x}", 123);
        database.save_lyrics("track-a", &old).unwrap();
        database
            .save_analysis(
                &deepseek::analysis_cache_key(&old, MODEL, LEVEL),
                &analysis(&old, 0),
            )
            .unwrap();
        let mut new = document("君の声");
        new.track.artist = "別の歌手".into();
        assert!(database
            .load_document_analyses(&new, MODEL, LEVEL)
            .unwrap()
            .is_empty());
        database.save_lyrics("track-b", &new).unwrap();
        assert!(database
            .load_document_analyses(&new, MODEL, LEVEL)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn warmed_context_cache_does_not_write_again_on_lookup() {
        let (_directory, database) = database();
        let doc = document("君の声\n聞こえる");
        save_all(&database, &doc);
        let changes = || {
            database
                .connection()
                .unwrap()
                .query_row("SELECT total_changes()", [], |row| row.get::<_, i64>(0))
                .unwrap()
        };
        let before = changes();
        assert_eq!(
            database
                .load_document_analyses(&doc, MODEL, LEVEL)
                .unwrap()
                .len(),
            2
        );
        assert_eq!(changes(), before);
    }

    #[test]
    fn malformed_legacy_analysis_does_not_hide_other_valid_rows() {
        let (_directory, database) = database();
        let doc = document("君の声\n聞こえる");
        let key = deepseek::analysis_cache_key(&doc, MODEL, LEVEL);
        database.save_analysis(&key, &analysis(&doc, 0)).unwrap();
        database.connection().unwrap().execute(
            "INSERT INTO analysis_cache(cache_key, line_id, analysis_json) VALUES(?1, ?2, 'broken')",
            params![key, doc.lines[1].id],
        ).unwrap();
        let cached = database.load_analyses(&key).unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].line_id, doc.lines[0].id);
    }
}
