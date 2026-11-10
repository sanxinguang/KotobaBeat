mod analysis_jobs;
mod credentials;
mod db;
mod deepseek;
mod lyrics;
mod media;
mod models;
mod workflow;

use crate::{
    db::Database,
    lyrics::{
        lrc::parse_lrc,
        matcher::{can_auto_select, identity_from_media, rank_candidates},
        providers,
    },
    models::{
        AppPhase, AppSnapshot, LyricCandidate, LyricsDocument, PublicSettings, SettingsPatch,
        SyncType,
    },
};
use anyhow::{Context, Result};
use reqwest::{header, Client};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{Duration, Instant},
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State, WindowEvent,
};

pub struct AppState {
    snapshot: RwLock<AppSnapshot>,
    database: Database,
    client: Client,
    artwork_client: Client,
    selected_source: RwLock<Option<String>>,
    generation: AtomicU64,
    exiting: AtomicBool,
    analysis_cancel: tokio::sync::watch::Sender<u64>,
    resolution_cancel: tokio::sync::watch::Sender<u64>,
    media_poll: tokio::sync::Mutex<()>,
    last_media_sample: Mutex<Option<Instant>>,
    runtime_notices: Vec<String>,
}

impl AppState {
    fn new(
        database: Database,
        settings: PublicSettings,
        runtime_notices: Vec<String>,
    ) -> Result<Self> {
        let client = Client::builder()
            .user_agent("KotobaBeat/0.1.0 (personal Windows lyrics companion)")
            .connect_timeout(Duration::from_secs(4))
            .timeout(Duration::from_secs(15))
            .build()?;
        let snapshot = AppSnapshot {
            settings,
            notices: runtime_notices.clone(),
            ..Default::default()
        };
        let artwork_client = Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(6))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 3 || !valid_artwork_url(attempt.url()) {
                    attempt.error("不支持的封面重定向")
                } else {
                    attempt.follow()
                }
            }))
            .build()?;
        Ok(Self {
            snapshot: RwLock::new(snapshot),
            database,
            client,
            artwork_client,
            selected_source: RwLock::new(None),
            generation: AtomicU64::new(0),
            exiting: AtomicBool::new(false),
            analysis_cancel: tokio::sync::watch::channel(0).0,
            resolution_cancel: tokio::sync::watch::channel(0).0,
            media_poll: tokio::sync::Mutex::new(()),
            last_media_sample: Mutex::new(None),
            runtime_notices,
        })
    }
}

#[tauri::command]
fn get_snapshot(state: State<'_, AppState>) -> AppSnapshot {
    state.snapshot.read().expect("snapshot poisoned").clone()
}

#[tauri::command]
async fn select_session(
    app: AppHandle,
    state: State<'_, AppState>,
    source_app: Option<String>,
) -> std::result::Result<(), String> {
    *state
        .selected_source
        .write()
        .map_err(|_| "session lock poisoned")? = source_app;
    poll_media_once(&app).await.map_err(error_string)
}

#[tauri::command]
async fn toggle_play_pause(state: State<'_, AppState>) -> std::result::Result<(), String> {
    let selected = state
        .snapshot
        .read()
        .map_err(|_| "session lock poisoned")?
        .media
        .as_ref()
        .map(|media| media.source_app.clone())
        .ok_or("当前没有可控制的歌曲")?;
    media::toggle_play_pause(Some(&selected))
        .await
        .map_err(error_string)
}

#[tauri::command]
async fn skip_next(state: State<'_, AppState>) -> std::result::Result<(), String> {
    let selected = state
        .snapshot
        .read()
        .map_err(|_| "session lock poisoned")?
        .media
        .as_ref()
        .map(|media| media.source_app.clone())
        .ok_or("当前没有可控制的歌曲")?;
    media::skip_next(Some(&selected))
        .await
        .map_err(error_string)
}

#[tauri::command]
async fn skip_previous(state: State<'_, AppState>) -> std::result::Result<(), String> {
    let selected = state
        .snapshot
        .read()
        .map_err(|_| "session lock poisoned")?
        .media
        .as_ref()
        .map(|media| media.source_app.clone())
        .ok_or("当前没有可控制的歌曲")?;
    media::skip_previous(Some(&selected))
        .await
        .map_err(error_string)
}

#[tauri::command]
async fn seek_to(state: State<'_, AppState>, position_ms: u64) -> std::result::Result<(), String> {
    let selected = state
        .snapshot
        .read()
        .map_err(|_| "session lock poisoned")?
        .media
        .as_ref()
        .map(|media| media.source_app.clone())
        .ok_or("当前没有可控制的歌曲")?;
    media::seek(Some(&selected), position_ms)
        .await
        .map_err(error_string)
}

#[tauri::command]
async fn choose_candidate(
    app: AppHandle,
    state: State<'_, AppState>,
    candidate_id: String,
    artwork_candidate_id: Option<String>,
) -> std::result::Result<(), String> {
    let (mut candidate, media, generation, artwork_url) = {
        let snapshot = state.snapshot.read().map_err(|_| "snapshot poisoned")?;
        let candidate = snapshot
            .candidates
            .iter()
            .find(|item| item.candidate_id == candidate_id)
            .cloned()
            .ok_or("候选歌词不存在")?;
        let artwork_url = artwork_candidate_id
            .as_deref()
            .and_then(|artwork_id| {
                snapshot
                    .candidates
                    .iter()
                    .find(|item| item.candidate_id == artwork_id)
            })
            .and_then(|item| item.artwork_url.clone());
        if artwork_candidate_id.is_some() && artwork_url.is_none() {
            return Err("候选封面不存在".into());
        }
        let media = snapshot.media.clone().ok_or("当前没有歌曲")?;
        (candidate, media, snapshot.generation, artwork_url)
    };
    if artwork_url.is_some() {
        candidate.artwork_url = artwork_url;
    }
    let generation = begin_user_selection(&state, generation).map_err(error_string)?;
    apply_candidate(
        &app,
        &state,
        generation,
        &media.track_signature(),
        candidate,
    )
    .await
    .map_err(error_string)
}

#[tauri::command]
async fn import_lrc(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> std::result::Result<(), String> {
    let file = Path::new(&path);
    if std::fs::metadata(file).map_err(error_string)?.len() > 128 * 1024 {
        return Err("歌词文件超过 128 KB，请导入单首歌曲的 LRC 文本".into());
    }
    let raw = std::fs::read_to_string(file).map_err(error_string)?;
    let (media, generation) = {
        let snapshot = state.snapshot.read().map_err(|_| "snapshot poisoned")?;
        (
            snapshot.media.clone().ok_or("当前没有歌曲")?,
            snapshot.generation,
        )
    };
    let lines = parse_lrc(&raw);
    if lines.len() > 2000 {
        return Err("歌词超过 2000 行，请只导入单首歌曲".into());
    }
    if lines.is_empty() {
        return Err("文件中没有可识别的歌词".into());
    }
    let sync_type = if lines.iter().any(|line| line.start_ms.is_some()) {
        SyncType::Line
    } else {
        SyncType::Plain
    };
    let candidate = LyricCandidate {
        candidate_id: uuid::Uuid::new_v4().to_string(),
        provider: "Manual".into(),
        provider_track_id: path,
        title: media.title.clone(),
        artist: media.artist.clone(),
        album: media.album.clone(),
        duration_ms: media.duration_ms,
        sync_type,
        lines,
        score: 1.0,
        ..Default::default()
    };
    let generation = begin_user_selection(&state, generation).map_err(error_string)?;
    apply_candidate(
        &app,
        &state,
        generation,
        &media.track_signature(),
        candidate,
    )
    .await
    .map_err(error_string)
}

#[tauri::command]
async fn paste_lyrics(
    app: AppHandle,
    state: State<'_, AppState>,
    raw: String,
) -> std::result::Result<(), String> {
    if raw.len() > 128 * 1024 {
        return Err("歌词文本超过 128 KB".into());
    }
    let temp = std::env::temp_dir().join("kotobabeat-pasted.lrc");
    let (media, generation) = {
        let snapshot = state.snapshot.read().map_err(|_| "snapshot poisoned")?;
        (
            snapshot.media.clone().ok_or("当前没有歌曲")?,
            snapshot.generation,
        )
    };
    let lines = parse_lrc(&raw);
    if lines.len() > 2000 {
        return Err("歌词超过 2000 行，请只导入单首歌曲".into());
    }
    if lines.is_empty() {
        return Err("粘贴内容中没有可识别的歌词".into());
    }
    let sync_type = if lines.iter().any(|line| line.start_ms.is_some()) {
        SyncType::Line
    } else {
        SyncType::Plain
    };
    let candidate = LyricCandidate {
        candidate_id: uuid::Uuid::new_v4().to_string(),
        provider: "Manual".into(),
        provider_track_id: temp.to_string_lossy().to_string(),
        title: media.title.clone(),
        artist: media.artist.clone(),
        album: media.album.clone(),
        duration_ms: media.duration_ms,
        sync_type,
        lines,
        score: 1.0,
        ..Default::default()
    };
    let generation = begin_user_selection(&state, generation).map_err(error_string)?;
    apply_candidate(
        &app,
        &state,
        generation,
        &media.track_signature(),
        candidate,
    )
    .await
    .map_err(error_string)
}

#[tauri::command]
async fn force_refresh(
    app: AppHandle,
    state: State<'_, AppState>,
) -> std::result::Result<(), String> {
    let media = state
        .snapshot
        .read()
        .map_err(|_| "snapshot poisoned")?
        .media
        .clone()
        .ok_or("当前没有歌曲")?;
    // Keep the confirmed mapping until a replacement succeeds. Offline refresh
    // must never destroy a user's only working lyrics/offset selection.
    start_resolution(&app, media, true);
    Ok(())
}

#[tauri::command]
fn set_lyric_offset(
    app: AppHandle,
    state: State<'_, AppState>,
    offset_ms: i64,
) -> std::result::Result<(), String> {
    let (track_key, document) = {
        let mut snapshot = state.snapshot.write().map_err(|_| "snapshot poisoned")?;
        let track_key = snapshot
            .media
            .as_ref()
            .ok_or("当前没有歌曲")?
            .track_signature();
        let document = snapshot.lyrics.as_mut().ok_or("当前没有歌词")?;
        document.offset_ms = offset_ms.clamp(-10_000, 10_000);
        (track_key, document.clone())
    };
    state
        .database
        .save_lyrics(&track_key, &document)
        .map_err(error_string)?;
    emit_snapshot(&app);
    Ok(())
}

#[tauri::command]
async fn save_deepseek_key(
    app: AppHandle,
    state: State<'_, AppState>,
    api_key: String,
) -> std::result::Result<(), String> {
    deepseek::test_api_key(&state.client, &api_key)
        .await
        .map_err(|error| analysis_jobs::error_message(&error))?;
    credentials::store_api_key(&api_key).map_err(error_string)?;
    {
        let mut snapshot = state.snapshot.write().map_err(|_| "snapshot poisoned")?;
        snapshot.settings.has_api_key = true;
        state
            .database
            .save_settings(&snapshot.settings)
            .map_err(error_string)?;
    }
    emit_snapshot(&app);
    start_analysis_if_possible(&app).map_err(error_string)
}

#[tauri::command]
fn clear_deepseek_key(
    app: AppHandle,
    state: State<'_, AppState>,
) -> std::result::Result<(), String> {
    credentials::delete_api_key().map_err(error_string)?;
    state
        .analysis_cancel
        .send_modify(|value| *value = value.wrapping_add(1));
    let mut snapshot = state.snapshot.write().map_err(|_| "snapshot poisoned")?;
    snapshot.settings.has_api_key = false;
    if snapshot.lyrics.is_some() {
        snapshot.phase = AppPhase::LyricsOnly;
    } else if snapshot.media.is_none() {
        snapshot.phase = AppPhase::NoSession;
    }
    snapshot.message = Some("API Key 已清除，已有歌词和缓存解析仍可使用".into());
    state
        .database
        .save_settings(&snapshot.settings)
        .map_err(error_string)?;
    drop(snapshot);
    emit_snapshot(&app);
    Ok(())
}

#[tauri::command]
async fn retry_analysis(app: AppHandle) -> std::result::Result<(), String> {
    start_analysis_if_possible(&app).map_err(error_string)
}

#[tauri::command]
fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    patch: SettingsPatch,
) -> std::result::Result<PublicSettings, String> {
    let (settings, analysis_changed, monitoring_changed) = {
        let mut snapshot = state.snapshot.write().map_err(|_| "snapshot poisoned")?;
        let settings =
            workflow::patched_settings(&snapshot.settings, patch).map_err(error_string)?;
        let analysis_changed = settings.deepseek_model != snapshot.settings.deepseek_model
            || settings.learning_level != snapshot.settings.learning_level;
        let monitoring_changed =
            settings.monitoring_enabled != snapshot.settings.monitoring_enabled;
        state
            .database
            .save_settings(&settings)
            .map_err(error_string)?;
        snapshot.settings = settings.clone();
        (settings, analysis_changed, monitoring_changed)
    };
    if let Some(window) = app.get_webview_window("mini") {
        let _ = window.set_always_on_top(settings.mini_always_on_top);
    }
    if monitoring_changed && !settings.monitoring_enabled {
        cancel_resolution_and_analysis(&state);
        let mut snapshot = state.snapshot.write().map_err(|_| "snapshot poisoned")?;
        snapshot.generation = state.generation.fetch_add(1, Ordering::SeqCst) + 1;
        snapshot.phase = if snapshot.lyrics.is_some() {
            AppPhase::LyricsOnly
        } else {
            AppPhase::NoSession
        };
        snapshot.message = Some("已暂停监听，可在设置中重新开启".into());
        if let Some(media) = snapshot.media.as_mut() {
            media.status = "paused".into();
        }
    }
    if analysis_changed {
        start_analysis_if_possible(&app).map_err(error_string)?;
    }
    if monitoring_changed && settings.monitoring_enabled {
        // Force a fresh media identity on the next poll, including the same song.
        state
            .snapshot
            .write()
            .map_err(|_| "snapshot poisoned")?
            .media = None;
    }
    emit_snapshot(&app);
    Ok(settings)
}

#[tauri::command]
fn show_window(app: AppHandle, label: String) -> std::result::Result<(), String> {
    let window = app
        .get_webview_window(&label)
        .ok_or_else(|| format!("窗口 {label} 不存在"))?;
    window.show().map_err(error_string)?;
    window.set_focus().map_err(error_string)
}

#[tauri::command]
fn hide_window(app: AppHandle, label: String) -> std::result::Result<(), String> {
    app.get_webview_window(&label)
        .ok_or_else(|| format!("窗口 {label} 不存在"))?
        .hide()
        .map_err(error_string)
}

#[tauri::command]
fn quit_app(app: AppHandle, state: State<'_, AppState>) {
    state.exiting.store(true, Ordering::Relaxed);
    app.exit(0);
}

async fn poll_media_once(app: &AppHandle) -> Result<()> {
    let state = app.state::<AppState>();
    let _poll_guard = state.media_poll.lock().await;
    let selected = state
        .selected_source
        .read()
        .expect("session lock poisoned")
        .clone();
    let (sessions, mut media) = tokio::time::timeout(
        Duration::from_secs(4),
        media::read_media(selected.as_deref()),
    )
    .await
    .context("Windows 媒体读取超时")??;
    if *state.selected_source.read().expect("session lock poisoned") != selected {
        return Ok(());
    }
    if !state
        .snapshot
        .read()
        .expect("snapshot poisoned")
        .settings
        .monitoring_enabled
    {
        return Ok(());
    }
    let (previous_media, preserve_resolved_artwork) = {
        let snapshot = state.snapshot.read().expect("snapshot poisoned");
        (
            snapshot.media.clone(),
            snapshot
                .lyrics
                .as_ref()
                .is_some_and(|lyrics| lyrics.artwork_url.is_some()),
        )
    };
    let elapsed_ms = state
        .last_media_sample
        .lock()
        .expect("clock lock poisoned")
        .replace(Instant::now())
        .map(|previous| previous.elapsed().as_millis().min(u64::MAX as u128) as u64);
    if let (Some(current), Some(previous)) = (media.as_mut(), previous_media.as_ref()) {
        merge_media_fields(current, previous, preserve_resolved_artwork, elapsed_ms);
    }
    let previous_signature = previous_media.as_ref().map(|value| value.track_signature());
    let new_signature = media.as_ref().map(|value| value.track_signature());
    {
        let mut snapshot = state.snapshot.write().expect("snapshot poisoned");
        snapshot.selected_source = selected.clone();
        snapshot
            .notices
            .retain(|notice| !notice.starts_with("媒体连接："));
        snapshot.sessions = sessions;
        snapshot.media = media.clone();
        if media.is_none() {
            if previous_media.is_some() {
                snapshot.generation = state.generation.fetch_add(1, Ordering::SeqCst) + 1;
                cancel_resolution_and_analysis(&state);
            }
            snapshot.phase = AppPhase::NoSession;
            snapshot.message = Some(media::no_session_message(selected.is_some()).into());
            snapshot.notices.clear();
            snapshot.lyrics = None;
            snapshot.candidates.clear();
            snapshot.analyses.clear();
        }
    }
    emit_snapshot(app);
    if new_signature.is_some() && new_signature != previous_signature {
        start_resolution(app, media.expect("signature requires media"), false);
    }
    Ok(())
}

#[cfg(test)]
fn merge_transient_media_fields(
    current: &mut crate::models::MediaState,
    previous: &crate::models::MediaState,
    preserve_resolved_artwork: bool,
) {
    merge_media_fields(current, previous, preserve_resolved_artwork, None);
}

fn merge_media_fields(
    current: &mut crate::models::MediaState,
    previous: &crate::models::MediaState,
    preserve_resolved_artwork: bool,
    elapsed_ms: Option<u64>,
) {
    if !current.same_track_as(previous) {
        return;
    }
    if current.artist.trim().is_empty() {
        current.artist = previous.artist.clone();
    }
    if current.album.trim().is_empty() {
        current.album = previous.album.clone();
    }
    let previous_cover_is_better =
        previous
            .thumbnail_data_url
            .as_ref()
            .is_some_and(|previous_cover| {
                preserve_resolved_artwork
                    || current
                        .thumbnail_data_url
                        .as_ref()
                        .is_none_or(|current_cover| previous_cover.len() > current_cover.len())
            });
    if previous_cover_is_better {
        current.thumbnail_data_url = previous.thumbnail_data_url.clone();
    }
    if current.duration_ms == 0 {
        current.duration_ms = previous.duration_ms;
    }
    if !current.timeline_available {
        let now_ms = chrono::Utc::now().timestamp_millis();
        // System clock adjustments must not move estimated playback by hours.
        let sample_gap_ms = elapsed_ms
            .unwrap_or_else(|| now_ms.saturating_sub(previous.sampled_at_ms).max(0) as u64)
            .min(10_000) as f64;
        let transition_factor = match (
            previous.status.as_str() == "playing",
            current.status.as_str() == "playing",
        ) {
            (true, true) => 1.0,
            (true, false) | (false, true) => 0.5,
            (false, false) => 0.0,
        };
        let first_sample_compensation_ms = if current.status == "playing"
            && previous.position_ms == 0
            && !previous.position_estimated
        {
            350.0
        } else {
            0.0
        };
        let elapsed_ms = sample_gap_ms * transition_factor * previous.playback_rate
            + first_sample_compensation_ms;
        let estimated = previous
            .position_ms
            .saturating_add(elapsed_ms.max(0.0).round() as u64);
        current.position_ms = if current.duration_ms > 0 {
            estimated.min(current.duration_ms)
        } else {
            estimated
        };
        current.sampled_at_ms = now_ms;
        current.position_estimated = true;
    }
}

fn start_artwork_fetch(
    app: &AppHandle,
    generation: u64,
    artwork_url: Option<String>,
    replace_existing: bool,
) {
    let Some(artwork_url) = artwork_url else {
        return;
    };
    let should_fetch = app
        .state::<AppState>()
        .snapshot
        .read()
        .expect("snapshot poisoned")
        .media
        .as_ref()
        .is_some();
    if !should_fetch {
        return;
    }

    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let client = handle.state::<AppState>().artwork_client.clone();
        let Ok(data_url) = download_artwork(&client, &artwork_url).await else {
            return;
        };
        let state = handle.state::<AppState>();
        if state.generation.load(Ordering::SeqCst) != generation {
            return;
        }
        let mut snapshot = state.snapshot.write().expect("snapshot poisoned");
        if snapshot.generation != generation {
            return;
        }
        if !replace_existing
            && snapshot
                .lyrics
                .as_ref()
                .is_some_and(|lyrics| lyrics.artwork_url.is_some())
        {
            return;
        }
        let Some(media) = snapshot.media.as_mut() else {
            return;
        };
        let is_better_cover = replace_existing
            || media
                .thumbnail_data_url
                .as_ref()
                .is_none_or(|current| data_url.len() > current.len());
        if !is_better_cover {
            return;
        }
        media.thumbnail_data_url = Some(data_url);
        drop(snapshot);
        emit_snapshot(&handle);
    });
}

fn start_platform_artwork_lookup(
    app: &AppHandle,
    generation: u64,
    track: crate::models::TrackIdentity,
) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let (client, settings) = {
            let state = handle.state::<AppState>();
            let settings = state
                .snapshot
                .read()
                .expect("snapshot poisoned")
                .settings
                .clone();
            (state.client.clone(), settings)
        };
        let artwork_url = providers::search_artwork(&client, &track, &settings).await;
        if handle.state::<AppState>().generation.load(Ordering::SeqCst) != generation {
            return;
        }
        start_artwork_fetch(&handle, generation, artwork_url, false);
    });
}

async fn download_artwork(client: &Client, artwork_url: &str) -> Result<String> {
    const MAX_BYTES: u64 = 12 * 1024 * 1024;
    let url = reqwest::Url::parse(artwork_url)?;
    if !valid_artwork_url(&url) {
        anyhow::bail!("不支持的封面地址");
    }
    let mut response = client
        .get(url)
        .timeout(Duration::from_secs(6))
        .send()
        .await?
        .error_for_status()?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_BYTES)
    {
        anyhow::bail!("artwork is too large");
    }
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    if !valid_artwork_url(response.url()) {
        anyhow::bail!("封面重定向到不支持的地址");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len().saturating_add(chunk.len()) > MAX_BYTES as usize {
            anyhow::bail!("artwork is too large");
        }
        bytes.extend_from_slice(&chunk);
    }
    media::image_data_url(&bytes, &content_type).context("unsupported artwork image")
}

fn valid_artwork_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some_and(|host| {
            host == "y.gtimg.cn" || host == "music.126.net" || host.ends_with(".music.126.net")
        })
}

fn cancel_resolution_and_analysis(state: &AppState) {
    state
        .resolution_cancel
        .send_modify(|value| *value = value.wrapping_add(1));
    state
        .analysis_cancel
        .send_modify(|value| *value = value.wrapping_add(1));
}

fn begin_user_selection(state: &AppState, expected_generation: u64) -> Result<u64> {
    let mut snapshot = state.snapshot.write().expect("snapshot poisoned");
    if snapshot.generation != expected_generation {
        anyhow::bail!("歌曲已切换，请重新选择歌词");
    }
    cancel_resolution_and_analysis(state);
    snapshot.generation = state.generation.fetch_add(1, Ordering::SeqCst) + 1;
    Ok(snapshot.generation)
}

fn start_resolution(app: &AppHandle, media: crate::models::MediaState, force: bool) {
    let state = app.state::<AppState>();
    let (generation, mut cancellation) = {
        let mut snapshot = state.snapshot.write().expect("snapshot poisoned");
        cancel_resolution_and_analysis(&state);
        let generation = state.generation.fetch_add(1, Ordering::SeqCst) + 1;
        snapshot.generation = generation;
        snapshot.phase = AppPhase::Identifying;
        snapshot.message = Some("正在识别歌曲版本…".into());
        snapshot.notices.clear();
        if !force {
            snapshot.lyrics = None;
            snapshot.analyses.clear();
        }
        snapshot.candidates.clear();
        (generation, state.resolution_cancel.subscribe())
    };
    emit_snapshot(app);
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = tokio::select! {
            biased;
            _ = cancellation.changed() => Ok(()),
            result = resolve_track(handle.clone(), generation, media, force) => result,
        };
        if let Err(error) = result {
            set_error_if_current(&handle, generation, error.to_string());
        }
    });
}

async fn resolve_track(
    app: AppHandle,
    generation: u64,
    media: crate::models::MediaState,
    force: bool,
) -> Result<()> {
    let state = app.state::<AppState>();
    let track_key = media.track_signature();
    let track = identity_from_media(&media);
    let cached_document = state.database.lyrics_for_track(&track_key)?;
    if let Some(document) = cached_document.clone().filter(|_| !force) {
        let artwork_url = document.artwork_url.clone();
        install_document(&app, generation, document)?;
        if artwork_url.is_none() {
            start_platform_artwork_lookup(&app, generation, track.clone());
        }
        start_artwork_fetch(&app, generation, artwork_url, true);
        start_analysis_if_possible(&app)?;
        return Ok(());
    }

    {
        let mut snapshot = state.snapshot.write().expect("snapshot poisoned");
        if snapshot.generation != generation {
            return Ok(());
        }
        snapshot.phase = AppPhase::ResolvingLyrics;
        snapshot.message = Some("正在从多个歌词源检索…".into());
    }
    emit_snapshot(&app);
    let settings = state
        .snapshot
        .read()
        .expect("snapshot poisoned")
        .settings
        .clone();
    let found = providers::search_all(&state.client, &track, &settings).await;
    if state.generation.load(Ordering::SeqCst) != generation {
        return Ok(());
    }
    let mut warnings = found.warnings;
    let mut candidates = found.candidates;
    let mut ranked = rank_candidates(&track, candidates.clone());
    if ranked
        .first()
        .is_none_or(|candidate| candidate.score < 0.68)
    {
        {
            let mut snapshot = state.snapshot.write().expect("snapshot poisoned");
            if snapshot.generation != generation {
                return Ok(());
            }
            snapshot.notices = warnings.clone();
            snapshot.message = Some("首轮未找到可靠版本，正在校准歌名继续检索…".into());
        }
        emit_snapshot(&app);
        let fallback = providers::search_fallback(&state.client, &track, &settings).await;
        candidates.extend(fallback.candidates);
        warnings.extend(fallback.warnings);
        ranked = rank_candidates(&track, candidates);
    }
    {
        let mut snapshot = state.snapshot.write().expect("snapshot poisoned");
        if snapshot.generation != generation {
            return Ok(());
        }
        warnings.sort();
        warnings.dedup();
        snapshot.notices = warnings;
    }
    install_candidate_media_fallbacks(&app, generation, &ranked);
    if ranked.is_empty() {
        if let Some(document) = cached_document {
            install_document(&app, generation, document)?;
            {
                let mut snapshot = state.snapshot.write().expect("snapshot poisoned");
                if snapshot.generation != generation {
                    return Ok(());
                }
                snapshot
                    .notices
                    .push("重新检索未获得可用结果，已保留上次确认的歌词和解析。".into());
            }
            start_analysis_if_possible(&app)?;
            emit_snapshot(&app);
            return Ok(());
        }
        let mut snapshot = state.snapshot.write().expect("snapshot poisoned");
        if snapshot.generation != generation {
            return Ok(());
        }
        snapshot.phase = AppPhase::LyricsOnly;
        snapshot.message = Some("未找到可靠歌词，可导入 LRC 或粘贴歌词".into());
        drop(snapshot);
        emit_snapshot(&app);
        return Ok(());
    }
    if can_auto_select(&track, &ranked) {
        apply_candidate(&app, &state, generation, &track_key, ranked[0].clone()).await?;
    } else {
        let viable = ranked.into_iter().take(8).collect::<Vec<_>>();
        let mut snapshot = state.snapshot.write().expect("snapshot poisoned");
        if snapshot.generation != generation {
            return Ok(());
        }
        snapshot.candidates = viable;
        snapshot.phase = AppPhase::CandidateRequired;
        snapshot.message = None;
        drop(snapshot);
        emit_snapshot(&app);
    }
    Ok(())
}

fn install_candidate_media_fallbacks(app: &AppHandle, generation: u64, ranked: &[LyricCandidate]) {
    if app.state::<AppState>().generation.load(Ordering::SeqCst) != generation {
        return;
    }
    if let Some(duration_ms) = infer_candidate_duration(ranked) {
        let state = app.state::<AppState>();
        let mut snapshot = state.snapshot.write().expect("snapshot poisoned");
        if let Some(media) = snapshot.media.as_mut() {
            if media.duration_ms == 0 {
                media.duration_ms = duration_ms;
            }
        }
        drop(snapshot);
        emit_snapshot(app);
    }
    let artwork_url = ranked
        .iter()
        .find(|candidate| {
            candidate.score_breakdown.title >= 0.92
                && candidate.score_breakdown.artist >= 0.75
                && candidate.score_breakdown.penalties < 0.2
                && candidate.artwork_url.is_some()
        })
        .and_then(|candidate| candidate.artwork_url.clone());
    start_artwork_fetch(app, generation, artwork_url, false);
}

fn infer_candidate_duration(ranked: &[LyricCandidate]) -> Option<u64> {
    let mut durations = ranked
        .iter()
        .filter(|candidate| {
            candidate.duration_ms > 0
                && candidate.score_breakdown.title >= 0.92
                && candidate.score_breakdown.artist >= 0.75
                && candidate.score_breakdown.penalties < 0.2
        })
        .map(|candidate| candidate.duration_ms)
        .collect::<Vec<_>>();
    if durations.is_empty() {
        return None;
    }
    durations.sort_unstable();
    if durations.len() > 1 && durations.last().expect("non-empty").abs_diff(durations[0]) > 8_000 {
        return None;
    }
    Some(durations[durations.len() / 2])
}

async fn apply_candidate(
    app: &AppHandle,
    state: &AppState,
    generation: u64,
    track_key: &str,
    candidate: LyricCandidate,
) -> Result<()> {
    let mut media = {
        let snapshot = state.snapshot.read().expect("snapshot poisoned");
        if snapshot.generation != generation {
            return Ok(());
        }
        snapshot
            .media
            .clone()
            .context("current media disappeared")?
    };
    if media.duration_ms == 0 && candidate.duration_ms > 0 {
        media.duration_ms = candidate.duration_ms;
        let mut snapshot = state.snapshot.write().expect("snapshot poisoned");
        if snapshot.generation != generation {
            return Ok(());
        }
        if let Some(current) = snapshot.media.as_mut() {
            if current.same_track_as(&media) && current.duration_ms == 0 {
                current.duration_ms = candidate.duration_ms;
            }
        }
        drop(snapshot);
        emit_snapshot(app);
    }
    let track = identity_from_media(&media);
    let mut hasher = Sha256::new();
    hasher.update(candidate.provider.as_bytes());
    hasher.update(candidate.provider_track_id.as_bytes());
    for line in &candidate.lines {
        hasher.update(line.text.as_bytes());
    }
    let previous_document = state.database.lyrics_for_track(track_key)?;
    let offset_ms = previous_document
        .as_ref()
        .filter(|previous| {
            previous.lines.len() == candidate.lines.len()
                && previous
                    .lines
                    .iter()
                    .zip(&candidate.lines)
                    .all(|(old, new)| old.text == new.text && old.start_ms == new.start_ms)
        })
        .map(|document| document.offset_ms)
        .unwrap_or(0);
    let document = LyricsDocument {
        cache_key: format!("{:x}", hasher.finalize()),
        track,
        provider: candidate.provider,
        provider_track_id: candidate.provider_track_id,
        sync_type: candidate.sync_type,
        instrumental: candidate.instrumental,
        offset_ms,
        lines: candidate.lines,
        artwork_url: candidate.artwork_url,
    };
    {
        let snapshot = state.snapshot.read().expect("snapshot poisoned");
        if snapshot.generation != generation {
            return Ok(());
        }
        // Promote legacy random-ID rows before replacing their source document.
        state.database.load_document_analyses(
            &document,
            &snapshot.settings.deepseek_model,
            &snapshot.settings.learning_level,
        )?;
        state.database.save_lyrics(track_key, &document)?;
    }
    let artwork_url = document.artwork_url.clone();
    install_document(app, generation, document)?;
    if artwork_url.is_none() {
        start_platform_artwork_lookup(app, generation, identity_from_media(&media));
    }
    start_artwork_fetch(app, generation, artwork_url, true);
    start_analysis_if_possible(app)
}

fn install_document(app: &AppHandle, generation: u64, document: LyricsDocument) -> Result<()> {
    let state = app.state::<AppState>();
    let mut snapshot = state.snapshot.write().expect("snapshot poisoned");
    if snapshot.generation != generation {
        return Ok(());
    }
    state
        .analysis_cancel
        .send_modify(|value| *value = value.wrapping_add(1));
    let mut analyses = HashMap::new();
    let model = snapshot.settings.deepseek_model.clone();
    let level = snapshot.settings.learning_level.clone();
    for analysis in state
        .database
        .load_document_analyses(&document, &model, &level)?
    {
        analyses.insert(analysis.line_id.clone(), analysis);
    }
    snapshot.lyrics = Some(document);
    snapshot.candidates.clear();
    snapshot.analyses = analyses;
    snapshot.phase = if snapshot.settings.has_api_key {
        AppPhase::Analyzing
    } else {
        AppPhase::LyricsOnly
    };
    snapshot.message = if snapshot.settings.has_api_key {
        Some("正在生成中文翻译与语法知识…".into())
    } else {
        Some("歌词已同步；添加 API Key 后生成翻译与语法".into())
    };
    drop(snapshot);
    emit_snapshot(app);
    Ok(())
}

fn start_analysis_if_possible(app: &AppHandle) -> Result<()> {
    let state = app.state::<AppState>();
    let (document, media, settings, stamp, mut cancellation) = {
        let mut snapshot = state.snapshot.write().expect("snapshot poisoned");
        state
            .analysis_cancel
            .send_modify(|value| *value = value.wrapping_add(1));
        let Some(document) = snapshot.lyrics.clone() else {
            return Ok(());
        };
        let Some(media) = snapshot.media.clone() else {
            return Ok(());
        };
        let settings = snapshot.settings.clone();
        snapshot.analyses = state
            .database
            .load_document_analyses(
                &document,
                &settings.deepseek_model,
                &settings.learning_level,
            )?
            .into_iter()
            .map(|analysis| (analysis.line_id.clone(), analysis))
            .collect();
        if document.instrumental
            || document
                .lines
                .iter()
                .all(|line| snapshot.analyses.contains_key(&line.id))
        {
            snapshot.phase = AppPhase::Ready;
            snapshot.message = document.instrumental.then(|| "这是一首纯音乐".into());
            drop(snapshot);
            emit_snapshot(app);
            return Ok(());
        }
        if !settings.has_api_key || !settings.monitoring_enabled {
            snapshot.phase = AppPhase::LyricsOnly;
            snapshot.message = Some(
                if settings.monitoring_enabled {
                    "歌词已同步；可在设置中添加 API Key 以生成解析"
                } else {
                    "已暂停监听"
                }
                .into(),
            );
            drop(snapshot);
            emit_snapshot(app);
            return Ok(());
        }
        snapshot.phase = AppPhase::Analyzing;
        snapshot.message = Some("正在生成中文翻译与语法知识…".into());
        let cancellation = state.analysis_cancel.subscribe();
        let stamp = workflow::AnalysisStamp {
            generation: snapshot.generation,
            epoch: *cancellation.borrow(),
            document_key: document.cache_key.clone(),
            model: settings.deepseek_model.clone(),
            level: settings.learning_level.clone(),
        };
        (document, media, settings, stamp, cancellation)
    };
    emit_snapshot(app);
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = tokio::select! {
            biased;
            _ = cancellation.changed() => Ok(()),
            result = run_analysis(&handle, &document, media.position_ms, &settings, &stamp) => result,
        };
        if let Err(error) = result {
            let state = handle.state::<AppState>();
            let mut snapshot = state.snapshot.write().expect("snapshot poisoned");
            if stamp.is_current(&snapshot, *state.analysis_cancel.borrow()) {
                snapshot.phase = AppPhase::LyricsOnly;
                snapshot.message = Some(analysis_jobs::error_message(&error));
                drop(snapshot);
                emit_snapshot(&handle);
            }
        }
    });
    Ok(())
}

async fn run_analysis(
    app: &AppHandle,
    document: &LyricsDocument,
    position_ms: u64,
    settings: &PublicSettings,
    stamp: &workflow::AnalysisStamp,
) -> Result<()> {
    let state = app.state::<AppState>();
    let api_key = credentials::get_api_key()?.context("无法读取 API Key，请在设置中重新保存")?;
    let existing = {
        let snapshot = state.snapshot.read().expect("snapshot poisoned");
        if !stamp.is_current(&snapshot, *state.analysis_cancel.borrow()) {
            return Ok(());
        }
        snapshot
            .analyses
            .keys()
            .cloned()
            .collect::<std::collections::HashSet<_>>()
    };
    let mut batches = deepseek::prioritized_batches(document, position_ms);
    for batch in &mut batches {
        batch.retain(|line_id| !existing.contains(line_id));
    }
    batches.retain(|batch| !batch.is_empty());
    if batches.is_empty() {
        let mut snapshot = state.snapshot.write().expect("snapshot poisoned");
        if !stamp.is_current(&snapshot, *state.analysis_cancel.borrow()) {
            return Ok(());
        }
        snapshot.phase = AppPhase::Ready;
        snapshot.message = None;
        drop(snapshot);
        emit_snapshot(app);
        return Ok(());
    }

    let mut last_error = None;
    analysis_jobs::run_batches(
        batches,
        |batch| {
            let client = &state.client;
            let api_key = &api_key;
            async move {
                deepseek::analyze_batch(
                    client,
                    api_key,
                    document,
                    &batch,
                    &settings.deepseek_model,
                    &settings.learning_level,
                )
                .await
            }
        },
        |lines, error| {
            if let Some(error) = error {
                last_error = Some(error);
            }
            if !lines.is_empty() {
                apply_analysis_result(app, stamp, document, lines)?;
            }
            Ok(())
        },
    )
    .await?;
    {
        let mut snapshot = state.snapshot.write().expect("snapshot poisoned");
        if !stamp.is_current(&snapshot, *state.analysis_cancel.borrow()) {
            return Ok(());
        }
        let missing = document
            .lines
            .iter()
            .filter(|line| !snapshot.analyses.contains_key(&line.id))
            .count();
        snapshot.phase = if missing == 0 {
            AppPhase::Ready
        } else {
            AppPhase::LyricsOnly
        };
        snapshot.message = (missing > 0).then(|| {
            format!(
                "还有 {missing} 行未完成翻译。{}",
                last_error.unwrap_or_else(|| "可点击重试补齐".into())
            )
        });
        drop(snapshot);
        emit_snapshot(app);
    }
    Ok(())
}

fn apply_analysis_result(
    app: &AppHandle,
    stamp: &workflow::AnalysisStamp,
    document: &LyricsDocument,
    analyses: Vec<crate::models::LineAnalysis>,
) -> Result<()> {
    let state = app.state::<AppState>();
    let mut snapshot = state.snapshot.write().expect("snapshot poisoned");
    if !stamp.is_current(&snapshot, *state.analysis_cancel.borrow()) {
        return Ok(());
    }
    for analysis in analyses {
        if !document
            .lines
            .iter()
            .any(|line| line.id == analysis.line_id)
        {
            continue;
        }
        state
            .database
            .save_document_analysis(document, &stamp.model, &stamp.level, &analysis)?;
        snapshot.analyses.insert(analysis.line_id.clone(), analysis);
    }
    drop(snapshot);
    emit_snapshot(app);
    Ok(())
}

fn set_error_if_current(app: &AppHandle, generation: u64, message: String) {
    let state = app.state::<AppState>();
    let mut snapshot = state.snapshot.write().expect("snapshot poisoned");
    if snapshot.generation != generation {
        return;
    }
    snapshot.phase = AppPhase::Error;
    snapshot.message = Some(message);
    drop(snapshot);
    emit_snapshot(app);
}

fn emit_snapshot(app: &AppHandle) {
    let state = app.state::<AppState>();
    let snapshot = {
        let mut snapshot = state.snapshot.write().expect("snapshot poisoned");
        snapshot.revision = snapshot.revision.saturating_add(1);
        for notice in &state.runtime_notices {
            if !snapshot.notices.contains(notice) {
                snapshot.notices.push(notice.clone());
            }
        }
        snapshot.clone()
    };
    let _ = app.emit("state://snapshot", snapshot);
}

async fn monitor_loop(app: AppHandle) {
    let media_changed = Arc::new(tokio::sync::Notify::new());
    let topology_changed = Arc::new(AtomicBool::new(false));
    let mut watcher =
        media::MediaEventWatcher::new(Arc::clone(&media_changed), Arc::clone(&topology_changed))
            .await
            .ok();
    let mut watcher_attempt = Instant::now();

    loop {
        let should_exit = app.state::<AppState>().exiting.load(Ordering::Relaxed);
        if should_exit {
            break;
        }
        if topology_changed.swap(false, Ordering::AcqRel)
            || (watcher.is_none() && watcher_attempt.elapsed() >= Duration::from_secs(15))
        {
            watcher = media::MediaEventWatcher::new(
                Arc::clone(&media_changed),
                Arc::clone(&topology_changed),
            )
            .await
            .ok();
            watcher_attempt = Instant::now();
        }
        let monitoring = app
            .state::<AppState>()
            .snapshot
            .read()
            .expect("snapshot poisoned")
            .settings
            .monitoring_enabled;
        if monitoring {
            if let Err(_error) = poll_media_once(&app).await {
                let state = app.state::<AppState>();
                let mut snapshot = state.snapshot.write().expect("snapshot poisoned");
                let notice = "媒体连接：暂时无法读取 Windows 媒体信息，将自动重试；如持续出现，请重新播放或重启播放器。";
                if !snapshot.notices.iter().any(|item| item == notice) {
                    snapshot.notices.push(notice.into());
                }
                if let Some(media) = snapshot.media.as_mut() {
                    media.status = "paused".into();
                    media.capabilities = Default::default();
                } else {
                    snapshot.phase = AppPhase::NoSession;
                    snapshot.message = Some(notice.into());
                }
                drop(snapshot);
                emit_snapshot(&app);
            }
        }
        tokio::select! {
            _ = media_changed.notified() => {}
            _ = tokio::time::sleep(Duration::from_millis(800)) => {}
        }
    }

    drop(watcher);
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show_main = MenuItem::with_id(app, "show_main", "打开主界面", true, None::<&str>)?;
    let show_mini = MenuItem::with_id(app, "show_mini", "显示迷你窗", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_main, &show_mini, &quit])?;
    let mut tray = TrayIconBuilder::new()
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show_main" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "show_mini" => {
                if let Some(window) = app.get_webview_window("mini") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "quit" => {
                app.state::<AppState>()
                    .exiting
                    .store(true, Ordering::Relaxed);
                app.exit(0);
            }
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn error_string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let data_dir = app
                .path()
                .app_local_data_dir()
                .context("cannot resolve app data directory")?;
            let mut notices = Vec::new();
            let persistent = Database::open(&data_dir.join("kotobabeat.sqlite3")).and_then(|database| {
                let settings = database.load_settings()?;
                Ok((database, settings))
            });
            let (database, mut settings) = match persistent {
                Ok(value) => value,
                Err(_) => {
                    notices.push("本地数据库无法打开，已进入临时模式；原文件未删除，本次设置和解析不会持久保存。请检查磁盘空间和文件权限后重启。".into());
                    (Database::in_memory()?, PublicSettings::default())
                }
            };
            settings.has_api_key = match credentials::get_api_key() {
                Ok(key) => key.is_some(),
                Err(_) => { notices.push("Windows 凭据库暂时无法读取，已保留同步歌词和缓存；可稍后在设置中重新保存 API Key。".into()); false }
            };
            // Do not overwrite malformed preferences just by opening the app.
            app.manage(AppState::new(database, settings, notices)?);
            setup_tray(app)?;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(monitor_loop(handle));
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    if !window
                        .app_handle()
                        .state::<AppState>()
                        .exiting
                        .load(Ordering::Relaxed)
                    {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            select_session,
            toggle_play_pause,
            skip_next,
            skip_previous,
            seek_to,
            choose_candidate,
            import_lrc,
            paste_lyrics,
            force_refresh,
            set_lyric_offset,
            save_deepseek_key,
            clear_deepseek_key,
            retry_analysis,
            update_settings,
            show_window,
            hide_window,
            quit_app
        ])
        .run(tauri::generate_context!())
        .expect("error while running KotobaBeat");
}

#[cfg(test)]
mod media_merge_tests {
    use super::{infer_candidate_duration, merge_transient_media_fields};
    use crate::models::{LyricCandidate, MediaState, ScoreBreakdown};

    #[test]
    fn manual_adoption_invalidates_delayed_searches_and_stale_choices() {
        let state = super::AppState::new(
            crate::db::Database::in_memory().unwrap(),
            Default::default(),
            Vec::new(),
        )
        .unwrap();
        let search = state.resolution_cancel.subscribe();
        let analysis = state.analysis_cancel.subscribe();
        assert_eq!(super::begin_user_selection(&state, 0).unwrap(), 1);
        assert!(search.has_changed().unwrap());
        assert!(analysis.has_changed().unwrap());
        assert_eq!(state.snapshot.read().unwrap().generation, 1);
        assert!(super::begin_user_selection(&state, 0).is_err());
    }

    #[test]
    fn different_album_version_does_not_inherit_old_progress_cover_or_duration() {
        let previous = MediaState {
            source_app: "player".into(),
            title: "song".into(),
            artist: "artist".into(),
            album: "studio".into(),
            duration_ms: 180_000,
            position_ms: 170_000,
            thumbnail_data_url: Some("old-cover".into()),
            ..Default::default()
        };
        let mut current = MediaState {
            source_app: "player".into(),
            title: "song".into(),
            artist: "artist".into(),
            album: "Live at Tokyo".into(),
            ..Default::default()
        };
        super::merge_media_fields(&mut current, &previous, true, Some(800));
        assert_eq!(current.position_ms, 0);
        assert_eq!(current.duration_ms, 0);
        assert!(current.thumbnail_data_url.is_none());
    }

    #[test]
    fn artwork_domains_do_not_allow_local_or_untrusted_redirects() {
        for value in [
            "http://y.gtimg.cn/cover",
            "https://127.0.0.1/cover",
            "https://music.126.net.evil.test/a",
            "file:///C:/secret",
        ] {
            assert!(!super::valid_artwork_url(
                &reqwest::Url::parse(value).unwrap()
            ));
        }
        assert!(super::valid_artwork_url(
            &reqwest::Url::parse("https://p1.music.126.net/a").unwrap()
        ));
    }

    #[test]
    fn estimated_clock_uses_monotonic_elapsed_not_wall_clock_jump() {
        let previous = MediaState {
            source_app: "player".into(),
            title: "song".into(),
            position_ms: 42_000,
            sampled_at_ms: 1,
            playback_rate: 1.0,
            status: "playing".into(),
            position_estimated: true,
            ..Default::default()
        };
        let mut current = previous.clone();
        super::merge_media_fields(&mut current, &previous, false, Some(800));
        assert_eq!(current.position_ms, 42_800);
    }

    #[test]
    fn keeps_cover_duration_and_advances_when_timeline_temporarily_disappears() {
        let now = chrono::Utc::now().timestamp_millis();
        let previous = MediaState {
            source_app: "cloudmusic.exe".into(),
            title: "Song".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            duration_ms: 180_000,
            position_ms: 42_000,
            sampled_at_ms: now - 1_000,
            playback_rate: 1.0,
            status: "playing".into(),
            thumbnail_data_url: Some("data:image/png;base64,AA==".into()),
            timeline_available: false,
            ..Default::default()
        };
        let mut current = MediaState {
            source_app: "cloudmusic.exe".into(),
            title: "Song".into(),
            artist: "Artist".into(),
            sampled_at_ms: now,
            playback_rate: 1.0,
            status: "playing".into(),
            ..Default::default()
        };

        merge_transient_media_fields(&mut current, &previous, false);

        assert_eq!(current.album, "Album");
        assert_eq!(current.duration_ms, 180_000);
        assert_eq!(current.thumbnail_data_url, previous.thumbnail_data_url);
        assert!(current.position_ms >= 42_900);
        assert!(current.position_estimated);
    }

    #[test]
    fn infers_duration_only_from_consistent_high_confidence_candidates() {
        let candidate = |duration_ms| LyricCandidate {
            duration_ms,
            score_breakdown: ScoreBreakdown {
                title: 1.0,
                artist: 1.0,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            infer_candidate_duration(&[candidate(329_000), candidate(329_746), candidate(332_000)]),
            Some(329_746)
        );
        assert_eq!(
            infer_candidate_duration(&[candidate(180_000), candidate(240_000)]),
            None
        );
    }

    #[test]
    fn compensates_the_first_estimated_playing_sample() {
        let now = chrono::Utc::now().timestamp_millis();
        let previous = MediaState {
            source_app: "cloudmusic.exe".into(),
            title: "Song".into(),
            artist: "Artist".into(),
            sampled_at_ms: now - 800,
            playback_rate: 1.0,
            status: "playing".into(),
            timeline_available: false,
            position_estimated: false,
            ..Default::default()
        };
        let mut current = MediaState {
            source_app: "cloudmusic.exe".into(),
            title: "Song".into(),
            artist: "Artist".into(),
            sampled_at_ms: now,
            playback_rate: 1.0,
            status: "playing".into(),
            timeline_available: false,
            ..Default::default()
        };

        merge_transient_media_fields(&mut current, &previous, false);

        assert!(current.position_ms >= 1_100);
        assert!(current.position_estimated);
    }

    #[test]
    fn keeps_the_larger_cover_when_gsmtc_refreshes_with_a_thumbnail() {
        let previous = MediaState {
            source_app: "cloudmusic.exe".into(),
            title: "Song".into(),
            artist: "Artist".into(),
            thumbnail_data_url: Some(format!("data:image/jpeg;base64,{}", "A".repeat(20_000))),
            ..Default::default()
        };
        let mut current = MediaState {
            source_app: "cloudmusic.exe".into(),
            title: "Song".into(),
            artist: "Artist".into(),
            thumbnail_data_url: Some("data:image/jpeg;base64,small".into()),
            ..Default::default()
        };

        merge_transient_media_fields(&mut current, &previous, false);

        assert_eq!(current.thumbnail_data_url, previous.thumbnail_data_url);
    }

    #[test]
    fn keeps_the_user_selected_cover_even_when_it_is_smaller() {
        let previous = MediaState {
            source_app: "cloudmusic.exe".into(),
            title: "Song".into(),
            artist: "Artist".into(),
            thumbnail_data_url: Some("data:image/jpeg;base64,chosen".into()),
            ..Default::default()
        };
        let mut current = MediaState {
            source_app: "cloudmusic.exe".into(),
            title: "Song".into(),
            artist: "Artist".into(),
            thumbnail_data_url: Some(format!("data:image/jpeg;base64,{}", "A".repeat(20_000))),
            ..Default::default()
        };

        merge_transient_media_fields(&mut current, &previous, true);

        assert_eq!(current.thumbnail_data_url, previous.thumbnail_data_url);
    }
}
