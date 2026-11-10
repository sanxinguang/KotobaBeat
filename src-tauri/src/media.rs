use crate::models::{MediaState, PlaybackCapabilities, SessionSummary};
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::{future::Future, time::Duration};
use tokio::sync::Notify;
use windows::{
    Foundation::TypedEventHandler,
    Media::{
        Control::{
            CurrentSessionChangedEventArgs, GlobalSystemMediaTransportControlsSession,
            GlobalSystemMediaTransportControlsSessionManager,
            GlobalSystemMediaTransportControlsSessionMediaProperties,
            GlobalSystemMediaTransportControlsSessionPlaybackStatus,
            GlobalSystemMediaTransportControlsSessionTimelineProperties,
            MediaPropertiesChangedEventArgs, PlaybackInfoChangedEventArgs,
            SessionsChangedEventArgs, TimelinePropertiesChangedEventArgs,
        },
        MediaPlaybackType,
    },
    Storage::Streams::{DataReader, InputStreamOptions},
};

const MAX_THUMBNAIL_BYTES: usize = 12 * 1024 * 1024;
const WINDOWS_TO_UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;
const MAX_TIMELINE_STALENESS_MS: i64 = 24 * 60 * 60 * 1000;

async fn bounded<T>(future: impl Future<Output = windows::core::Result<T>>) -> Result<T> {
    Ok(tokio::time::timeout(Duration::from_millis(900), future)
        .await
        .map_err(|_| anyhow!("Windows 媒体接口响应超时"))??)
}

pub fn no_session_message(selected: bool) -> &'static str {
    if selected {
        "锁定的播放器暂不可用。请启动该播放器并开启系统媒体控制，或切回“自动跟随”。"
    } else {
        "未检测到可用的音乐会话。若网易云正在播放，请到网易云“设置 → 系统”勾选“开启 SMTC”，再暂停/播放或切歌；其他播放器也需开启系统媒体控制。"
    }
}

fn choose_session_index(
    selected: Option<usize>,
    current: Option<usize>,
    playing: Option<usize>,
    first: Option<usize>,
    pinned: bool,
) -> Option<usize> {
    if pinned {
        selected
    } else {
        current.or(playing).or(first)
    }
}

struct WatchedSession {
    session: GlobalSystemMediaTransportControlsSession,
    timeline_token: Option<i64>,
    playback_token: Option<i64>,
    media_token: Option<i64>,
}

pub struct MediaEventWatcher {
    manager: GlobalSystemMediaTransportControlsSessionManager,
    current_session_token: Option<i64>,
    sessions_token: Option<i64>,
    sessions: Vec<WatchedSession>,
}

impl MediaEventWatcher {
    pub async fn new(notify: Arc<Notify>, topology_changed: Arc<AtomicBool>) -> Result<Self> {
        let operation = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?;
        let manager = bounded(async { operation.await }).await?;

        let current_session_token = {
            let notify = Arc::clone(&notify);
            let topology_changed = Arc::clone(&topology_changed);
            manager
                .CurrentSessionChanged(&TypedEventHandler::<
                    GlobalSystemMediaTransportControlsSessionManager,
                    CurrentSessionChangedEventArgs,
                >::new(move |_, _| {
                    topology_changed.store(true, Ordering::Release);
                    notify.notify_one();
                    Ok(())
                }))
                .ok()
        };

        let sessions_token = {
            let notify = Arc::clone(&notify);
            let topology_changed = Arc::clone(&topology_changed);
            manager
                .SessionsChanged(&TypedEventHandler::<
                    GlobalSystemMediaTransportControlsSessionManager,
                    SessionsChangedEventArgs,
                >::new(move |_, _| {
                    topology_changed.store(true, Ordering::Release);
                    notify.notify_one();
                    Ok(())
                }))
                .ok()
        };

        let session_list = manager.GetSessions()?;
        let mut sessions = Vec::with_capacity(session_list.Size()? as usize);
        for index in 0..session_list.Size()? {
            let Ok(session) = session_list.GetAt(index) else {
                continue;
            };
            let timeline_token = {
                let notify = Arc::clone(&notify);
                session
                    .TimelinePropertiesChanged(&TypedEventHandler::<
                        GlobalSystemMediaTransportControlsSession,
                        TimelinePropertiesChangedEventArgs,
                    >::new(move |_, _| {
                        notify.notify_one();
                        Ok(())
                    }))
                    .ok()
            };
            let playback_token = {
                let notify = Arc::clone(&notify);
                session
                    .PlaybackInfoChanged(&TypedEventHandler::<
                        GlobalSystemMediaTransportControlsSession,
                        PlaybackInfoChangedEventArgs,
                    >::new(move |_, _| {
                        notify.notify_one();
                        Ok(())
                    }))
                    .ok()
            };
            let media_token = {
                let notify = Arc::clone(&notify);
                session
                    .MediaPropertiesChanged(&TypedEventHandler::<
                        GlobalSystemMediaTransportControlsSession,
                        MediaPropertiesChangedEventArgs,
                    >::new(move |_, _| {
                        notify.notify_one();
                        Ok(())
                    }))
                    .ok()
            };
            sessions.push(WatchedSession {
                session,
                timeline_token,
                playback_token,
                media_token,
            });
        }

        Ok(Self {
            manager,
            current_session_token,
            sessions_token,
            sessions,
        })
    }
}

impl Drop for MediaEventWatcher {
    fn drop(&mut self) {
        if let Some(token) = self.current_session_token {
            let _ = self.manager.RemoveCurrentSessionChanged(token);
        }
        if let Some(token) = self.sessions_token {
            let _ = self.manager.RemoveSessionsChanged(token);
        }
        for watched in &self.sessions {
            if let Some(token) = watched.timeline_token {
                let _ = watched.session.RemoveTimelinePropertiesChanged(token);
            }
            if let Some(token) = watched.playback_token {
                let _ = watched.session.RemovePlaybackInfoChanged(token);
            }
            if let Some(token) = watched.media_token {
                let _ = watched.session.RemoveMediaPropertiesChanged(token);
            }
        }
    }
}

pub async fn read_media(
    selected_source: Option<&str>,
) -> Result<(Vec<SessionSummary>, Option<MediaState>)> {
    let operation = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?;
    let manager = bounded(async { operation.await }).await?;
    let current_source = manager
        .GetCurrentSession()
        .ok()
        .and_then(|session| session.SourceAppUserModelId().ok())
        .map(|value| value.to_string_lossy());
    // WinRT's collection view is apartment-bound; materialize agile sessions
    // before awaiting their properties on Tokio's multi-thread runtime.
    let sessions = collect_sessions(&manager)?;

    let mut summaries = Vec::new();
    let mut available = Vec::new();
    let mut metadata_failures = 0;
    let mut selected_session = None;
    let mut current_session = None;
    let mut playing_session = None;

    for session in sessions {
        let Ok(source) = session.SourceAppUserModelId() else {
            continue;
        };
        let source = source.to_string_lossy();
        let Ok(operation) = session.TryGetMediaPropertiesAsync() else {
            metadata_failures += 1;
            continue;
        };
        let Ok(properties) = bounded(async { operation.await }).await else {
            metadata_failures += 1;
            continue;
        };
        let title = properties
            .Title()
            .map(|value| value.to_string_lossy())
            .unwrap_or_default();
        let artist = properties
            .Artist()
            .map(|value| value.to_string_lossy())
            .unwrap_or_default();
        let album = properties
            .AlbumTitle()
            .map(|value| value.to_string_lossy())
            .unwrap_or_default();
        let playback_type = read_playback_type(&properties);

        if !should_include_session(&source, &title, &artist, &album, playback_type) {
            continue;
        }

        let is_current = current_source.as_deref() == Some(source.as_str());
        if is_current {
            current_session = Some(available.len());
        }
        if selected_source == Some(source.as_str()) {
            selected_session = Some(available.len());
        }
        if playing_session.is_none()
            && session
                .GetPlaybackInfo()
                .and_then(|info| info.PlaybackStatus())
                .is_ok_and(|status| {
                    status == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing
                })
        {
            playing_session = Some(available.len());
        }
        summaries.push(SessionSummary {
            id: source.clone(),
            source_app: source,
            title,
            artist,
            is_current,
        });
        available.push((session, properties));
    }

    if summaries.is_empty() && metadata_failures > 0 {
        return Err(anyhow!("播放器未能返回歌曲信息，请稍后重试或重启播放器"));
    }
    let chosen = choose_session_index(
        selected_session,
        current_session,
        playing_session,
        (!available.is_empty()).then_some(0),
        selected_source.is_some(),
    );
    let media = match chosen {
        Some(index) => {
            let (session, properties) = &available[index];
            Some(read_session(session, properties).await?)
        }
        None => None,
    };
    Ok((summaries, media))
}

async fn read_session(
    session: &GlobalSystemMediaTransportControlsSession,
    properties: &GlobalSystemMediaTransportControlsSessionMediaProperties,
) -> Result<MediaState> {
    let source_app = session.SourceAppUserModelId()?.to_string_lossy();
    let timeline = session.GetTimelineProperties().ok();
    let playback = session.GetPlaybackInfo()?;
    let controls = playback.Controls()?;
    let status = playback.PlaybackStatus()?;
    let playback_rate = playback
        .PlaybackRate()
        .ok()
        .and_then(|value| value.Value().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(1.0);

    let now_ms = chrono::Utc::now().timestamp_millis();
    let (duration_ms, raw_position_ms) = timeline.as_ref().map(timeline_values).unwrap_or_default();
    let last_updated_ms = timeline
        .as_ref()
        .and_then(|timeline| timeline.LastUpdatedTime().ok())
        .and_then(|value| windows_datetime_to_unix_ms(value.UniversalTime, now_ms));
    let position_ms = correct_position_to_now(
        raw_position_ms,
        duration_ms,
        status,
        playback_rate,
        last_updated_ms,
        now_ms,
    );
    let can_seek = controls.IsPlaybackPositionEnabled().unwrap_or(false);
    let timeline_available = timeline.is_some() && (duration_ms > 0 || raw_position_ms > 0);
    let thumbnail_data_url =
        tokio::time::timeout(Duration::from_millis(500), read_thumbnail(properties))
            .await
            .ok()
            .and_then(Result::ok)
            .flatten();

    Ok(MediaState {
        session_id: source_app.clone(),
        source_app,
        title: properties.Title()?.to_string_lossy(),
        artist: properties.Artist()?.to_string_lossy(),
        album: properties.AlbumTitle()?.to_string_lossy(),
        duration_ms,
        position_ms,
        sampled_at_ms: now_ms,
        playback_rate,
        status: status_name(status).into(),
        thumbnail_data_url,
        timeline_available,
        position_estimated: false,
        capabilities: PlaybackCapabilities {
            can_play_pause: controls.IsPlayEnabled().unwrap_or(false)
                || controls.IsPauseEnabled().unwrap_or(false),
            can_skip_next: controls.IsNextEnabled().unwrap_or(false),
            can_skip_previous: controls.IsPreviousEnabled().unwrap_or(false),
            can_seek,
        },
    })
}

fn read_playback_type(
    properties: &GlobalSystemMediaTransportControlsSessionMediaProperties,
) -> Option<MediaPlaybackType> {
    properties
        .PlaybackType()
        .ok()
        .and_then(|value| value.Value().ok())
}

fn should_include_session(
    source_app: &str,
    title: &str,
    _artist: &str,
    _album: &str,
    playback_type: Option<MediaPlaybackType>,
) -> bool {
    if title.trim().is_empty() {
        return false;
    }
    // Browser media sessions cannot be trusted to distinguish video from music:
    // Chrome/Edge may publish a normal web video as MediaPlaybackType::Music.
    // KotobaBeat intentionally targets native music players, so browser sessions
    // are excluded completely to guarantee that web videos never appear.
    if is_browser_source(source_app) {
        return false;
    }
    match playback_type {
        Some(MediaPlaybackType::Video | MediaPlaybackType::Image) => false,
        Some(MediaPlaybackType::Music) => true,
        _ => true,
    }
}

fn is_browser_source(source_app: &str) -> bool {
    let source = source_app.to_ascii_lowercase();
    [
        "chrome", "msedge", "firefox", "brave", "opera", "vivaldi", "arc.exe",
    ]
    .iter()
    .any(|browser| source.contains(browser))
}

fn timeline_values(
    timeline: &GlobalSystemMediaTransportControlsSessionTimelineProperties,
) -> (u64, u64) {
    timeline_values_from_ticks(
        timeline
            .StartTime()
            .map(|value| value.Duration)
            .unwrap_or(0),
        timeline.EndTime().map(|value| value.Duration).unwrap_or(0),
        timeline
            .MinSeekTime()
            .map(|value| value.Duration)
            .unwrap_or(0),
        timeline
            .MaxSeekTime()
            .map(|value| value.Duration)
            .unwrap_or(0),
        timeline.Position().map(|value| value.Duration).unwrap_or(0),
    )
}

fn timeline_values_from_ticks(
    start: i64,
    end: i64,
    min_seek: i64,
    max_seek: i64,
    position: i64,
) -> (u64, u64) {
    let start = start.max(0);
    let end = end.max(0);
    let min_seek = min_seek.max(0);
    let max_seek = max_seek.max(0);
    let position = position.max(0);
    let origin = match (start, min_seek) {
        (0, value) | (value, 0) => value,
        (left, right) => left.min(right),
    };
    let end_duration = end.saturating_sub(start);
    let seek_duration = max_seek.saturating_sub(min_seek);
    let duration_ticks = end_duration.max(seek_duration);
    let position_ticks = position.saturating_sub(origin);
    let duration_ms = ticks_to_ms(duration_ticks);
    let position_ms = ticks_to_ms(position_ticks);
    (
        duration_ms,
        if duration_ms > 0 {
            position_ms.min(duration_ms)
        } else {
            position_ms
        },
    )
}

fn windows_datetime_to_unix_ms(universal_time: i64, now_ms: i64) -> Option<i64> {
    let unix_ticks = universal_time.checked_sub(WINDOWS_TO_UNIX_EPOCH_TICKS)?;
    let unix_ms = unix_ticks / 10_000;
    if unix_ms <= 0
        || unix_ms > now_ms + 60_000
        || now_ms.saturating_sub(unix_ms) > MAX_TIMELINE_STALENESS_MS
    {
        return None;
    }
    Some(unix_ms)
}

fn correct_position_to_now(
    position_ms: u64,
    duration_ms: u64,
    status: GlobalSystemMediaTransportControlsSessionPlaybackStatus,
    playback_rate: f64,
    last_updated_ms: Option<i64>,
    now_ms: i64,
) -> u64 {
    let mut corrected = position_ms;
    if status == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing {
        if let Some(updated_at) = last_updated_ms {
            let elapsed = now_ms.saturating_sub(updated_at).max(0) as f64;
            corrected = corrected.saturating_add((elapsed * playback_rate).round() as u64);
        }
    }
    if duration_ms > 0 {
        corrected.min(duration_ms)
    } else {
        corrected
    }
}

async fn read_thumbnail(
    properties: &GlobalSystemMediaTransportControlsSessionMediaProperties,
) -> Result<Option<String>> {
    let operation = properties.Thumbnail()?.OpenReadAsync()?;
    let (reader, declared_size, reported_type) = {
        let stream = bounded(async { operation.await }).await?;
        let size = stream.Size().unwrap_or_default() as usize;
        let content_type = stream.ContentType()?.to_string_lossy();
        let input = stream.GetInputStreamAt(0)?;
        (DataReader::CreateDataReader(&input)?, size, content_type)
    };
    if declared_size > MAX_THUMBNAIL_BYTES {
        return Ok(None);
    }

    reader.SetInputStreamOptions(InputStreamOptions::Partial)?;
    let mut bytes = Vec::with_capacity(declared_size.min(MAX_THUMBNAIL_BYTES));

    loop {
        let remaining = MAX_THUMBNAIL_BYTES.saturating_sub(bytes.len());
        if remaining == 0 {
            return Ok(None);
        }
        let requested = remaining.min(64 * 1024) as u32;
        let operation = reader.LoadAsync(requested)?;
        let loaded = bounded(async { operation.await }).await? as usize;
        if loaded == 0 {
            break;
        }
        let mut chunk = vec![0u8; loaded];
        reader.ReadBytes(&mut chunk)?;
        bytes.extend_from_slice(&chunk);
        if declared_size > 0 && bytes.len() >= declared_size {
            break;
        }
    }

    Ok(image_data_url(&bytes, &reported_type))
}

pub(crate) fn image_data_url(bytes: &[u8], reported_type: &str) -> Option<String> {
    if bytes.is_empty() || bytes.len() > MAX_THUMBNAIL_BYTES {
        return None;
    }
    let content_type =
        sniff_image_content_type(bytes).or_else(|| normalize_image_content_type(reported_type))?;
    Some(format!(
        "data:{content_type};base64,{}",
        STANDARD.encode(bytes)
    ))
}

fn normalize_image_content_type(reported_type: &str) -> Option<&'static str> {
    reported_type
        .split([',', ';'])
        .map(str::trim)
        .find_map(|value| match value.to_ascii_lowercase().as_str() {
            "image/jpeg" | "image/jpe" | "image/jpg" => Some("image/jpeg"),
            "image/png" => Some("image/png"),
            "image/gif" => Some("image/gif"),
            "image/bmp" => Some("image/bmp"),
            "image/webp" => Some("image/webp"),
            "image/avif" => Some("image/avif"),
            _ => None,
        })
}

fn sniff_image_content_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"BM") {
        Some("image/bmp")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

fn ticks_to_ms(ticks: i64) -> u64 {
    (ticks.max(0) as u64) / 10_000
}

fn status_name(status: GlobalSystemMediaTransportControlsSessionPlaybackStatus) -> &'static str {
    use GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status;
    match status {
        Status::Playing => "playing",
        Status::Paused => "paused",
        Status::Stopped => "stopped",
        Status::Changing => "changing",
        Status::Closed => "closed",
        Status::Opened => "opened",
        _ => "unknown",
    }
}

async fn selected_session(
    selected_source: Option<&str>,
) -> Result<GlobalSystemMediaTransportControlsSession> {
    let operation = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?;
    let manager = bounded(async { operation.await }).await?;
    if let Some(source) = selected_source {
        let sessions = collect_sessions(&manager)?;
        for session in sessions {
            if session.SourceAppUserModelId()?.to_string_lossy() == source
                && session_is_supported(&session).await
            {
                return Ok(session);
            }
        }
        return Err(anyhow!("该播放器已关闭或未开启系统媒体控制"));
    }
    let current = manager
        .GetCurrentSession()
        .map_err(|_| anyhow!("当前没有可控制的音乐会话"))?;
    if session_is_supported(&current).await {
        Ok(current)
    } else {
        Err(anyhow!("当前媒体会话不是音乐"))
    }
}

fn collect_sessions(
    manager: &GlobalSystemMediaTransportControlsSessionManager,
) -> Result<Vec<GlobalSystemMediaTransportControlsSession>> {
    let view = manager.GetSessions()?;
    Ok((0..view.Size()?)
        .filter_map(|index| view.GetAt(index).ok())
        .collect())
}

async fn session_is_supported(session: &GlobalSystemMediaTransportControlsSession) -> bool {
    let Ok(source) = session.SourceAppUserModelId() else {
        return false;
    };
    let Ok(operation) = session.TryGetMediaPropertiesAsync() else {
        return false;
    };
    let Ok(properties) = bounded(async { operation.await }).await else {
        return false;
    };
    let title = properties
        .Title()
        .map(|value| value.to_string_lossy())
        .unwrap_or_default();
    let artist = properties
        .Artist()
        .map(|value| value.to_string_lossy())
        .unwrap_or_default();
    let album = properties
        .AlbumTitle()
        .map(|value| value.to_string_lossy())
        .unwrap_or_default();
    should_include_session(
        &source.to_string_lossy(),
        &title,
        &artist,
        &album,
        read_playback_type(&properties),
    )
}

fn accepted(value: bool) -> Result<()> {
    if value {
        Ok(())
    } else {
        Err(anyhow!("播放器未接受此操作，可能不支持该控制功能"))
    }
}

pub async fn toggle_play_pause(selected_source: Option<&str>) -> Result<()> {
    let operation = selected_session(selected_source)
        .await?
        .TryTogglePlayPauseAsync()?;
    accepted(bounded(async { operation.await }).await?)
}

pub async fn skip_next(selected_source: Option<&str>) -> Result<()> {
    let operation = selected_session(selected_source)
        .await?
        .TrySkipNextAsync()?;
    accepted(bounded(async { operation.await }).await?)
}

pub async fn skip_previous(selected_source: Option<&str>) -> Result<()> {
    let operation = selected_session(selected_source)
        .await?
        .TrySkipPreviousAsync()?;
    accepted(bounded(async { operation.await }).await?)
}

pub async fn seek(selected_source: Option<&str>, position_ms: u64) -> Result<()> {
    let session = selected_session(selected_source).await?;
    if !session
        .GetPlaybackInfo()?
        .Controls()?
        .IsPlaybackPositionEnabled()?
    {
        return Err(anyhow!("当前播放器不支持跳转进度"));
    }
    let timeline = session.GetTimelineProperties()?;
    let start = timeline
        .StartTime()
        .map(|value| value.Duration)
        .unwrap_or(0)
        .max(0);
    let min_seek = timeline
        .MinSeekTime()
        .map(|value| value.Duration)
        .unwrap_or(0)
        .max(0);
    let origin = match (start, min_seek) {
        (0, value) | (value, 0) => value,
        (left, right) => left.min(right),
    };
    let (duration_ms, _) = timeline_values(&timeline);
    let position_ms = if duration_ms > 0 {
        position_ms.min(duration_ms)
    } else {
        position_ms
    };
    let target = origin.saturating_add(
        i64::try_from(position_ms)
            .unwrap_or(i64::MAX)
            .saturating_mul(10_000),
    );
    let operation = session.TryChangePlaybackPositionAsync(target)?;
    accepted(bounded(async { operation.await }).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_player_disappearance_never_controls_another_session() {
        assert_eq!(
            choose_session_index(None, Some(1), Some(2), Some(0), true),
            None
        );
        assert_eq!(
            choose_session_index(Some(3), Some(1), Some(2), Some(0), true),
            Some(3)
        );
        assert_eq!(
            choose_session_index(None, Some(1), Some(2), Some(0), false),
            Some(1)
        );
        assert_eq!(
            choose_session_index(None, None, Some(2), Some(0), false),
            Some(2)
        );
        assert_eq!(
            choose_session_index(None, None, None, Some(0), false),
            Some(0)
        );
    }

    #[test]
    fn no_session_guidance_is_conditional_not_a_false_diagnosis() {
        assert!(no_session_message(false).contains("若网易云正在播放"));
        assert!(no_session_message(false).contains("SMTC"));
        assert!(no_session_message(true).contains("自动跟随"));
    }

    #[test]
    fn rejected_media_controls_do_not_report_success() {
        assert!(accepted(true).is_ok());
        assert!(accepted(false).is_err());
    }

    #[test]
    fn timeline_handles_pause_seek_rate_and_future_timestamps() {
        use GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status;
        assert_eq!(
            correct_position_to_now(10_000, 60_000, Status::Paused, 2.0, Some(1_000), 5_000),
            10_000
        );
        assert_eq!(
            correct_position_to_now(10_000, 60_000, Status::Playing, 2.0, Some(1_000), 5_000),
            18_000
        );
        assert_eq!(
            correct_position_to_now(1_000, 60_000, Status::Playing, 1.0, Some(5_000), 5_100),
            1_100
        );
        assert_eq!(
            correct_position_to_now(10_000, 60_000, Status::Playing, 1.0, Some(7_000), 5_000),
            10_000
        );
        assert_eq!(timeline_values_from_ticks(-10, -2, 0, 0, -1), (0, 0));
    }

    #[tokio::test]
    async fn unresponsive_windows_operation_times_out() {
        let start = std::time::Instant::now();
        let result: Result<()> = bounded(std::future::pending()).await;
        assert!(result.is_err());
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn excludes_video_sessions_even_with_music_like_metadata() {
        assert!(!should_include_session(
            "chrome.exe",
            "A music video",
            "Artist",
            "Album",
            Some(MediaPlaybackType::Video),
        ));
    }

    #[test]
    fn excludes_browser_sessions_even_when_the_browser_claims_music() {
        assert!(!should_include_session(
            "MSEdge.exe",
            "Video title",
            "Channel",
            "",
            Some(MediaPlaybackType::Unknown),
        ));
        assert!(!should_include_session(
            "MSEdge.exe",
            "Song",
            "Artist",
            "",
            Some(MediaPlaybackType::Music),
        ));
    }

    #[test]
    fn uses_seek_range_when_end_time_is_missing() {
        let (duration, position) =
            timeline_values_from_ticks(0, 0, 0, 240_000 * 10_000, 12_500 * 10_000);
        assert_eq!(duration, 240_000);
        assert_eq!(position, 12_500);
    }

    #[test]
    fn advances_position_from_the_gsmtc_update_timestamp() {
        let corrected = correct_position_to_now(
            10_000,
            60_000,
            GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing,
            1.0,
            Some(1_000),
            3_500,
        );
        assert_eq!(corrected, 12_500);
    }

    #[test]
    fn detects_common_thumbnail_formats() {
        assert_eq!(
            sniff_image_content_type(b"\x89PNG\r\n\x1a\nrest"),
            Some("image/png")
        );
        assert_eq!(
            sniff_image_content_type(b"\xff\xd8\xffrest"),
            Some("image/jpeg")
        );
        let data_url = image_data_url(b"\xff\xd8\xffrest", "image/jpeg,image/jpe,image/jpg")
            .expect("valid image");
        assert!(data_url.starts_with("data:image/jpeg;base64,"));
        assert!(!data_url
            .trim_start_matches("data:image/jpeg;base64,")
            .contains(','));
    }

    #[tokio::test]
    #[ignore = "requires an interactive Windows media session"]
    async fn inspect_live_windows_media_sessions() {
        let (sessions, media) = read_media(None).await.expect("GSMTC read failed");
        println!("eligible sessions: {sessions:#?}");
        println!(
            "automatically selected source: {:?}",
            media.as_ref().map(|item| item.source_app.as_str())
        );
        for source in sessions.iter().map(|session| session.source_app.as_str()) {
            let (_, selected) = read_media(Some(source))
                .await
                .expect("selected GSMTC read failed");
            let selected = selected.expect("eligible session was not selectable");
            println!(
                "media for {source}: title={:?}, artist={:?}, album={:?}, duration_ms={}, position_ms={}, status={}, timeline_available={}, cover_bytes={}",
                selected.title,
                selected.artist,
                selected.album,
                selected.duration_ms,
                selected.position_ms,
                selected.status,
                selected.timeline_available,
                selected
                    .thumbnail_data_url
                    .as_ref()
                    .map(String::len)
                    .unwrap_or_default()
            );
            if let Some(data_url) = selected.thumbnail_data_url.as_deref() {
                println!(
                    "cover data URL prefix: {}",
                    data_url.chars().take(100).collect::<String>()
                );
                assert!(data_url.contains(";base64,"));
            }
        }
    }
}
