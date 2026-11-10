use super::{
    lrc::parse_lrc,
    matcher::{normalize, rank_candidates, score_candidate, search_variants, version_tags},
};
use crate::models::{LyricCandidate, PublicSettings, SyncType, TrackIdentity};
use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use futures::future::join_all;
use futures::{stream::FuturesUnordered, StreamExt};
use reqwest::{header, Client, RequestBuilder};
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;
use strsim::jaro_winkler;
use tokio::time::Instant;
use uuid::Uuid;

const USER_AGENT: &str = "KotobaBeat/0.1.0 (personal desktop lyrics client)";
const FIRST_PASS_BUDGET: Duration = Duration::from_millis(4_500);
const FALLBACK_BUDGET: Duration = Duration::from_secs(6);
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_LYRIC_BYTES: usize = 128 * 1024;
const MAX_LYRIC_LINES: usize = 2_000;
static PROVIDER_COOLDOWNS: once_cell::sync::Lazy<
    std::sync::Mutex<std::collections::HashMap<String, Instant>>,
> = once_cell::sync::Lazy::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

#[derive(Debug)]
struct ArtworkHit {
    url: String,
    score: f64,
    provider: &'static str,
}

type ArtworkFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Option<ArtworkHit>> + Send>>;

pub async fn search_artwork(
    client: &Client,
    track: &TrackIdentity,
    settings: &PublicSettings,
) -> Option<String> {
    let mut tasks = Vec::<ArtworkFuture>::new();
    if settings.experimental_netease {
        tasks.push(Box::pin(search_netease_artwork(
            client.clone(),
            track.clone(),
        )));
    }
    if settings.experimental_qq {
        tasks.push(Box::pin(search_qq_artwork(client.clone(), track.clone())));
    }

    let source = track.source_app.to_ascii_lowercase();
    join_all(tasks)
        .await
        .into_iter()
        .flatten()
        .map(|mut hit| {
            if (source.contains("cloudmusic") && hit.provider == "netease")
                || (source.contains("qqmusic") && hit.provider == "qq")
            {
                hit.score += 0.08;
            }
            hit
        })
        .filter(|hit| hit.score >= 0.88)
        .max_by(|left, right| {
            left.score
                .partial_cmp(&right.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|hit| hit.url)
}

async fn search_netease_artwork(client: Client, track: TrackIdentity) -> Option<ArtworkHit> {
    let query = format!("{} {}", track.title, track.artist);
    let response = fetch_json(
        client
            .get("https://music.163.com/api/cloudsearch/pc")
            .header(header::REFERER, "https://music.163.com/")
            .query(&[
                ("s", query.as_str()),
                ("type", "1"),
                ("offset", "0"),
                ("total", "true"),
                ("limit", "8"),
            ]),
        Instant::now() + Duration::from_secs(5),
    )
    .await
    .ok()?;
    response
        .pointer("/result/songs")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(|song| {
            let title = song.get("name").and_then(Value::as_str)?;
            let artist = netease_artist_names(song);
            let raw_url = netease_album_value(song, "picUrl").and_then(Value::as_str)?;
            Some(ArtworkHit {
                url: netease_high_resolution_url(raw_url),
                score: artwork_metadata_score(&track, title, &artist),
                provider: "netease",
            })
        })
        .max_by(|left, right| {
            left.score
                .partial_cmp(&right.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

async fn search_qq_artwork(client: Client, track: TrackIdentity) -> Option<ArtworkHit> {
    let query = format!("{} {}", track.title, track.artist);
    let payload = fetch_json(
        client
            .get("https://c.y.qq.com/soso/fcgi-bin/client_search_cp")
            .header(header::REFERER, "https://y.qq.com/")
            .query(&[
                ("w", query.as_str()),
                ("format", "json"),
                ("p", "1"),
                ("n", "8"),
            ]),
        Instant::now() + Duration::from_secs(5),
    )
    .await
    .ok()?;
    payload
        .pointer("/data/song/list")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(|song| {
            let title = song.get("songname").and_then(Value::as_str)?;
            let artist = song
                .get("singer")
                .and_then(Value::as_array)
                .and_then(|artists| artists.first())
                .and_then(|artist| artist.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let album_mid = song
                .get("albummid")
                .and_then(Value::as_str)
                .filter(|mid| !mid.is_empty())?;
            Some(ArtworkHit {
                url: format!("https://y.gtimg.cn/music/photo_new/T002R800x800M000{album_mid}.jpg"),
                score: artwork_metadata_score(&track, title, artist),
                provider: "qq",
            })
        })
        .max_by(|left, right| {
            left.score
                .partial_cmp(&right.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

fn artwork_metadata_score(track: &TrackIdentity, title: &str, artist: &str) -> f64 {
    if normalize(&track.title).is_empty()
        || normalize(title).is_empty()
        || normalize(&track.artist).is_empty()
        || normalize(artist).is_empty()
        || version_tags(&track.title) != version_tags(title)
    {
        return 0.0;
    }
    let title_score = jaro_winkler(&normalize(&track.title), &normalize(title));
    let artist_score = jaro_winkler(&normalize(&track.artist), &normalize(artist));
    title_score * 0.65 + artist_score * 0.35
}

fn netease_high_resolution_url(raw_url: &str) -> String {
    let secure = raw_url.replacen("http://", "https://", 1);
    let base = secure.split('?').next().unwrap_or(&secure);
    format!("{base}?param=1000y1000")
}

fn netease_album_value<'a>(song: &'a Value, field: &str) -> Option<&'a Value> {
    song.get("al")
        .and_then(|album| album.get(field))
        .or_else(|| song.get("album").and_then(|album| album.get(field)))
}

fn netease_artist_names(song: &Value) -> String {
    song.get("ar")
        .or_else(|| song.get("artists"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|artist| artist.get("name").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(" / ")
        })
        .unwrap_or_default()
}

#[derive(Debug, Default)]
pub struct SearchOutcome {
    pub candidates: Vec<LyricCandidate>,
    /// Safe summaries only: never raw response bodies, URLs, or request data.
    pub warnings: Vec<String>,
}

impl SearchOutcome {
    fn append(&mut self, mut other: Self) {
        self.candidates.append(&mut other.candidates);
        self.warnings.append(&mut other.warnings);
        self.warnings.sort();
        self.warnings.dedup();
    }
}

#[derive(Clone)]
struct Endpoints {
    lrclib: String,
    netease_search: String,
    netease_lyric: String,
    qq_search: String,
    qq_lyric: String,
    musicbrainz: String,
    public_user_agent: Option<String>,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            lrclib: "https://lrclib.net/api/search".into(),
            netease_search: "https://music.163.com/api/cloudsearch/pc".into(),
            netease_lyric: "https://music.163.com/api/song/lyric".into(),
            qq_search: "https://c.y.qq.com/soso/fcgi-bin/client_search_cp".into(),
            qq_lyric: "https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg".into(),
            musicbrainz: "https://musicbrainz.org/ws/2/recording".into(),
            public_user_agent: contact_user_agent(
                std::env::var("KOTOBABEAT_CONTACT_URL").ok().as_deref(),
            ),
        }
    }
}

impl Endpoints {
    fn for_settings(settings: &PublicSettings) -> Self {
        let mut endpoints = Self::default();
        if let Some(agent) = contact_user_agent(Some(&settings.provider_contact)) {
            endpoints.public_user_agent = Some(agent);
        }
        endpoints
    }
}

pub fn valid_provider_contact(contact: &str) -> bool {
    contact_user_agent(Some(contact)).is_some()
}

fn contact_user_agent(contact: Option<&str>) -> Option<String> {
    let contact = contact?.trim();
    if contact.is_empty()
        || contact.len() > 256
        || !contact.is_ascii()
        || contact.chars().any(char::is_whitespace)
    {
        return None;
    }
    let valid_url = reqwest::Url::parse(contact).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    });
    let valid_email = contact.split_once('@').is_some_and(|(local, domain)| {
        !local.is_empty()
            && local
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || ".-_+".contains(c))
            && domain.contains('.')
            && domain.split('.').all(|part| {
                !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            })
    });
    if !valid_url && !valid_email {
        return None;
    }
    let agent = format!("KotobaBeat/0.1.0 ({contact})");
    agent.parse::<header::HeaderValue>().ok()?;
    Some(agent)
}

#[derive(Clone, Copy)]
enum Provider {
    LrcLib,
    Netease,
    Qq,
}

impl Provider {
    fn name(self) -> &'static str {
        match self {
            Self::LrcLib => "LRCLIB",
            Self::Netease => "网易云",
            Self::Qq => "QQ 音乐",
        }
    }
}

/// First round always finishes within one shared deadline. An unavailable
/// experimental provider must not withhold successful results from another.
pub async fn search_all(
    client: &Client,
    track: &TrackIdentity,
    settings: &PublicSettings,
) -> SearchOutcome {
    search_round(
        client,
        track,
        settings,
        &Endpoints::for_settings(settings),
        FIRST_PASS_BUDGET,
    )
    .await
}

async fn search_round(
    client: &Client,
    track: &TrackIdentity,
    settings: &PublicSettings,
    endpoints: &Endpoints,
    budget: Duration,
) -> SearchOutcome {
    let deadline = Instant::now() + budget;
    let mut providers = vec![Provider::LrcLib];
    if settings.experimental_netease {
        providers.push(Provider::Netease);
    }
    if settings.experimental_qq {
        providers.push(Provider::Qq);
    }
    let results = join_all(providers.into_iter().map(|provider| async move {
        match search_provider(provider, client, track, endpoints, deadline).await {
            Ok(found) => found,
            Err(error) => SearchOutcome {
                candidates: Vec::new(),
                warnings: vec![format!("{}：{error}", provider.name())],
            },
        }
    }))
    .await;
    let mut outcome = SearchOutcome::default();
    for result in results {
        outcome.append(result);
    }
    outcome
}

/// Called separately only after the UI has published first-round results or
/// an explicit fallback status. No hidden 20–30 second retry chain.
pub async fn search_fallback(
    client: &Client,
    track: &TrackIdentity,
    settings: &PublicSettings,
) -> SearchOutcome {
    let endpoints = Endpoints::for_settings(settings);
    let deadline = Instant::now() + FALLBACK_BUDGET;
    let mut result = SearchOutcome::default();
    let mut alternate = track.clone();
    alternate.title = search_variants(&track.title)
        .last()
        .cloned()
        .unwrap_or_else(|| track.title.clone());
    alternate.artist = search_variants(&track.artist)
        .last()
        .cloned()
        .unwrap_or_else(|| track.artist.clone());
    if alternate.title != track.title || alternate.artist != track.artist {
        result.append(
            search_round(
                client,
                &alternate,
                settings,
                &endpoints,
                Duration::from_millis(2_500),
            )
            .await,
        );
        if rank_candidates(track, result.candidates.clone())
            .first()
            .is_some_and(|candidate| candidate.score >= 0.68)
        {
            return result;
        }
    }
    match canonicalize_musicbrainz(client, track, &endpoints, deadline).await {
        Ok(Some((title, artist))) => {
            let mut canonical = track.clone();
            canonical.title = title;
            canonical.artist = artist;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining > Duration::from_millis(100) {
                result.append(
                    search_round(client, &canonical, settings, &endpoints, remaining).await,
                );
            }
        }
        Ok(None) => {}
        Err(error) => result.warnings.push(format!("MusicBrainz：{error}")),
    }
    result.warnings.sort();
    result.warnings.dedup();
    result
}

/// Bound both the transfer duration and the body size, including chunked bodies.
async fn fetch_json(request: RequestBuilder, deadline: Instant) -> Result<Value> {
    fetch_json_with_agent(request, deadline, USER_AGENT).await
}

async fn fetch_json_with_agent(
    request: RequestBuilder,
    deadline: Instant,
    user_agent: &str,
) -> Result<Value> {
    let origin = request
        .try_clone()
        .and_then(|request| request.build().ok())
        .map(|request| request.url().origin().ascii_serialization())
        .context("无法构造请求")?;
    if let Ok(mut cooldowns) = PROVIDER_COOLDOWNS.lock() {
        cooldowns.retain(|_, until| *until > Instant::now());
        if cooldowns.contains_key(&origin) {
            return Err(anyhow!("接口限流冷却中，请稍后重试"));
        }
    }
    tokio::time::timeout_at(deadline, async {
        let mut response = request
            .header(header::USER_AGENT, user_agent)
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    anyhow!("请求超时")
                } else {
                    anyhow!("网络连接失败")
                }
            })?;
        if !response.status().is_success() {
            if matches!(response.status().as_u16(), 429 | 503) {
                let delay = retry_after_delay(response.headers()).unwrap_or_else(|| {
                    Duration::from_secs(if response.status().as_u16() == 429 {
                        30
                    } else {
                        5
                    })
                });
                if let Some(until) = Instant::now().checked_add(delay) {
                    if let Ok(mut cooldowns) = PROVIDER_COOLDOWNS.lock() {
                        cooldowns
                            .entry(origin.clone())
                            .and_modify(|existing| *existing = (*existing).max(until))
                            .or_insert(until);
                    }
                }
            }
            return Err(anyhow!("HTTP {}，接口暂不可用", response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err(anyhow!("响应过大，已停止读取"));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow!("响应读取失败"))?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(anyhow!("响应过大，已停止读取"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let body = std::str::from_utf8(&bytes).map_err(|_| anyhow!("响应不是有效文本"))?;
        parse_json_or_jsonp(body).map_err(|_| anyhow!("返回格式已变化或不是 JSON"))
    })
    .await
    .map_err(|_| anyhow!("请求超时"))?
}

fn retry_after_delay(headers: &header::HeaderMap) -> Option<Duration> {
    let value = headers.get(header::RETRY_AFTER)?.to_str().ok()?;
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds.max(1)));
    }
    let until = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    let seconds = (until.timestamp() - chrono::Utc::now().timestamp()).max(1) as u64;
    Some(Duration::from_secs(seconds))
}

fn validate_api_code(payload: &Value, expected: i64) -> Result<()> {
    if let Some(code) = payload.get("code").and_then(Value::as_i64) {
        if code != expected {
            return Err(anyhow!("接口返回错误码 {code}，可能限流或已变更"));
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LrcLibItem {
    id: u64,
    track_name: String,
    artist_name: String,
    album_name: Option<String>,
    duration: Option<f64>,
    #[serde(default)]
    instrumental: bool,
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
}

async fn search_provider(
    provider: Provider,
    client: &Client,
    track: &TrackIdentity,
    endpoints: &Endpoints,
    deadline: Instant,
) -> Result<SearchOutcome> {
    let search_deadline = deadline.min(Instant::now() + Duration::from_millis(2_500));
    if matches!(provider, Provider::LrcLib) {
        let user_agent = endpoints
            .public_user_agent
            .as_deref()
            .context("未配置歌词服务联系信息（设置中的主页或邮箱），已跳过此歌词源")?;
        let mut query = vec![("track_name", track.title.as_str())];
        if !track.artist.trim().is_empty() {
            query.push(("artist_name", track.artist.as_str()));
        }
        let payload = fetch_json_with_agent(
            client.get(&endpoints.lrclib).query(&query),
            deadline,
            user_agent,
        )
        .await?;
        let items = payload.as_array().context("歌词列表格式已变化")?;
        let mut invalid = 0;
        let candidates = items
            .iter()
            .take(30)
            .filter_map(|item| {
                let item: LrcLibItem = match serde_json::from_value(item.clone()) {
                    Ok(item) => item,
                    Err(_) => {
                        invalid += 1;
                        return None;
                    }
                };
                let raw = item
                    .synced_lyrics
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .or(item.plain_lyrics.as_deref())
                    .unwrap_or_default();
                if raw.len() > MAX_LYRIC_BYTES {
                    invalid += 1;
                    return None;
                }
                let lines = parse_lrc(raw);
                if lines.len() > MAX_LYRIC_LINES {
                    invalid += 1;
                    return None;
                }
                if lines.is_empty() && !item.instrumental {
                    return None;
                }
                let sync_type = sync_type_for_lines(&lines);
                Some(LyricCandidate {
                    candidate_id: Uuid::new_v4().to_string(),
                    provider: "LRCLIB".into(),
                    provider_track_id: item.id.to_string(),
                    title: item.track_name,
                    artist: item.artist_name,
                    album: item.album_name.unwrap_or_default(),
                    duration_ms: duration_millis(item.duration.unwrap_or_default()),
                    instrumental: item.instrumental,
                    sync_type,
                    lines,
                    ..Default::default()
                })
            })
            .collect();
        return Ok(SearchOutcome {
            candidates,
            warnings: if invalid > 0 {
                vec![format!("LRCLIB：{invalid} 条结果格式异常，已跳过")]
            } else {
                Vec::new()
            },
        });
    }

    let query = format!("{} {}", track.title, track.artist);
    let request = match provider {
        Provider::Netease => client
            .get(&endpoints.netease_search)
            .header(header::REFERER, "https://music.163.com/")
            .query(&[
                ("s", query.as_str()),
                ("type", "1"),
                ("offset", "0"),
                ("total", "true"),
                ("limit", "8"),
            ]),
        Provider::Qq => client
            .get(&endpoints.qq_search)
            .header(header::REFERER, "https://y.qq.com/")
            .query(&[
                ("w", query.as_str()),
                ("format", "json"),
                ("p", "1"),
                ("n", "8"),
            ]),
        Provider::LrcLib => unreachable!(),
    };
    let payload = fetch_json(request, search_deadline).await?;
    validate_api_code(
        &payload,
        if matches!(provider, Provider::Netease) {
            200
        } else {
            0
        },
    )?;
    let list_path = if matches!(provider, Provider::Netease) {
        "/result/songs"
    } else {
        "/data/song/list"
    };
    // Some providers omit the empty list when songCount/totalnum is zero.
    let songs = match payload.pointer(list_path).and_then(Value::as_array) {
        Some(songs) => songs,
        None if payload.pointer("/result/songCount").and_then(Value::as_u64) == Some(0)
            || payload
                .pointer("/data/song/totalnum")
                .and_then(Value::as_u64)
                == Some(0) =>
        {
            return Ok(SearchOutcome::default())
        }
        None => return Err(anyhow!("搜索列表格式已变化")),
    };
    let mut metadata: Vec<_> = songs
        .iter()
        .take(8)
        .filter_map(|song| candidate_metadata(provider, song))
        .collect();
    if !songs.is_empty() && metadata.is_empty() {
        return Err(anyhow!("歌曲字段格式已变化"));
    }
    for candidate in &mut metadata {
        score_candidate(track, candidate);
    }
    metadata.sort_by(|left, right| right.score.total_cmp(&left.score));
    let mut seen = std::collections::HashSet::new();
    metadata.retain(|candidate| seen.insert(candidate.provider_track_id.clone()));
    let mut pending = FuturesUnordered::new();
    for candidate in metadata.into_iter().take(4) {
        pending.push(fetch_platform_lyrics(
            provider, client, candidate, endpoints, deadline,
        ));
    }
    let mut outcome = SearchOutcome::default();
    while !pending.is_empty() {
        match tokio::time::timeout_at(deadline, pending.next()).await {
            Ok(Some(Ok(Some(candidate)))) => outcome.candidates.push(candidate),
            Ok(Some(Ok(None))) => {}
            Ok(Some(Err(error))) => outcome
                .warnings
                .push(format!("{}：部分歌词{error}", provider.name())),
            Ok(None) => break,
            Err(_) => {
                outcome.warnings.push(format!(
                    "{}：部分歌词请求超时，已保留可用结果",
                    provider.name()
                ));
                break;
            }
        }
    }
    outcome.warnings.sort();
    outcome.warnings.dedup();
    Ok(outcome)
}

fn candidate_metadata(provider: Provider, song: &Value) -> Option<LyricCandidate> {
    let (id, title, artist, album, duration_ms, artwork_url) = match provider {
        Provider::Netease => (
            song.get("id")?.as_u64()?.to_string(),
            song.get("name")?.as_str()?.to_string(),
            netease_artist_names(song),
            netease_album_value(song, "name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            song.get("dt")
                .or_else(|| song.get("duration"))
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            netease_album_value(song, "picUrl")
                .and_then(Value::as_str)
                .map(netease_high_resolution_url),
        ),
        Provider::Qq => (
            song.get("songmid")?.as_str()?.to_string(),
            song.get("songname")?.as_str()?.to_string(),
            song.get("singer")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|artist| artist.get("name").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join(" / ")
                })
                .unwrap_or_default(),
            song.get("albumname")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            song.get("interval")
                .and_then(Value::as_u64)
                .unwrap_or_default()
                .saturating_mul(1_000),
            song.get("albummid")
                .and_then(Value::as_str)
                .filter(|mid| !mid.is_empty())
                .map(|mid| format!("https://y.gtimg.cn/music/photo_new/T002R800x800M000{mid}.jpg")),
        ),
        Provider::LrcLib => return None,
    };
    if title.trim().is_empty() {
        return None;
    }
    Some(LyricCandidate {
        candidate_id: Uuid::new_v4().to_string(),
        provider: if matches!(provider, Provider::Netease) {
            "NetEase"
        } else {
            "QQ Music"
        }
        .into(),
        provider_track_id: id,
        title,
        artist,
        album,
        duration_ms,
        artwork_url,
        ..Default::default()
    })
}

async fn fetch_platform_lyrics(
    provider: Provider,
    client: &Client,
    mut candidate: LyricCandidate,
    endpoints: &Endpoints,
    deadline: Instant,
) -> Result<Option<LyricCandidate>> {
    let request = match provider {
        Provider::Netease => client
            .get(&endpoints.netease_lyric)
            .header(header::REFERER, "https://music.163.com/")
            .query(&[
                ("id", candidate.provider_track_id.as_str()),
                ("lv", "-1"),
                ("kv", "-1"),
                ("tv", "-1"),
            ]),
        Provider::Qq => client
            .get(&endpoints.qq_lyric)
            .header(header::REFERER, "https://y.qq.com/")
            .query(&[
                ("songmid", candidate.provider_track_id.as_str()),
                ("format", "json"),
                ("nobase64", "1"),
            ]),
        Provider::LrcLib => unreachable!(),
    };
    let payload = fetch_json(request, deadline).await?;
    validate_api_code(
        &payload,
        if matches!(provider, Provider::Netease) {
            200
        } else {
            0
        },
    )?;
    if matches!(provider, Provider::Qq)
        && payload
            .get("retcode")
            .and_then(Value::as_i64)
            .is_some_and(|code| code != 0)
    {
        return Err(anyhow!("接口暂不可用"));
    }
    let mut raw = if matches!(provider, Provider::Netease) {
        candidate.instrumental = payload
            .get("nolyric")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if payload
            .pointer("/lrc/lyric")
            .and_then(Value::as_str)
            .is_none()
            && !candidate.instrumental
            && payload.get("uncollected").and_then(Value::as_bool) != Some(true)
        {
            return Err(anyhow!("格式已变化"));
        }
        payload
            .pointer("/lrc/lyric")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    } else {
        payload
            .get("lyric")
            .and_then(Value::as_str)
            .context("格式已变化")?
            .to_string()
    };
    if matches!(provider, Provider::Qq) && !raw.contains('[') {
        if let Ok(decoded) = STANDARD.decode(raw.trim()) {
            if let Ok(decoded) = String::from_utf8(decoded) {
                raw = decoded;
            }
        }
    }
    raw = decode_lyric_entities(&raw);
    if raw.len() > MAX_LYRIC_BYTES {
        return Err(anyhow!("内容过大，已跳过"));
    }
    candidate.lines = parse_lrc(&raw);
    if candidate.lines.len() > MAX_LYRIC_LINES {
        return Err(anyhow!("行数异常，已跳过"));
    }
    candidate.sync_type = sync_type_for_lines(&candidate.lines);
    if candidate.lines.is_empty() && !candidate.instrumental {
        return Ok(None);
    }
    Ok(Some(candidate))
}

fn sync_type_for_lines(lines: &[crate::models::LyricLine]) -> SyncType {
    if lines.iter().any(|line| line.start_ms.is_some()) {
        SyncType::Line
    } else {
        SyncType::Plain
    }
}

fn duration_millis(seconds: f64) -> u64 {
    if seconds.is_finite() && seconds > 0.0 && seconds < 24.0 * 60.0 * 60.0 {
        (seconds * 1_000.0).round() as u64
    } else {
        0
    }
}

fn decode_lyric_entities(input: &str) -> String {
    static NUMERIC: once_cell::sync::Lazy<regex::Regex> =
        once_cell::sync::Lazy::new(|| regex::Regex::new(r"&#(x[0-9a-fA-F]+|[0-9]+);").unwrap());
    let decoded = NUMERIC.replace_all(input, |captures: &regex::Captures<'_>| {
        let number = &captures[1];
        let value = if let Some(hex) = number.strip_prefix('x') {
            u32::from_str_radix(hex, 16)
        } else {
            number.parse()
        };
        value
            .ok()
            .and_then(char::from_u32)
            .map(|c| c.to_string())
            .unwrap_or_else(|| captures[0].to_string())
    });
    decoded
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn parse_json_or_jsonp(input: &str) -> Result<Value> {
    if let Ok(value) = serde_json::from_str(input) {
        return Ok(value);
    }
    let trimmed = input.trim();
    let start = trimmed.find('(').context("JSONP has no callback")?;
    let end = trimmed.rfind(')').context("JSONP has no callback end")?;
    let callback = &trimmed[..start];
    if callback.is_empty()
        || !callback
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_$.".contains(c))
        || start >= end
        || !trimmed[end + 1..]
            .trim()
            .trim_end_matches(';')
            .trim()
            .is_empty()
    {
        return Err(anyhow!("invalid JSONP wrapper"));
    }
    Ok(serde_json::from_str(&trimmed[start + 1..end])?)
}

static MUSICBRAINZ_LAST_REQUEST: tokio::sync::Mutex<Option<Instant>> =
    tokio::sync::Mutex::const_new(None);

async fn canonicalize_musicbrainz(
    client: &Client,
    track: &TrackIdentity,
    endpoints: &Endpoints,
    deadline: Instant,
) -> Result<Option<(String, String)>> {
    let user_agent = endpoints
        .public_user_agent
        .as_deref()
        .context("未配置歌词服务联系信息，已跳过可选的别名校准")?;
    if track.artist.trim().is_empty() {
        return Ok(None);
    }
    // A process-wide gate, not one independent sleep per request. Concurrent
    // track changes/manual searches still issue at most one request per second.
    tokio::time::timeout_at(deadline, async {
        let mut last = MUSICBRAINZ_LAST_REQUEST.lock().await;
        if let Some(previous) = *last {
            tokio::time::sleep_until(previous + Duration::from_secs(1)).await;
        }
        *last = Some(Instant::now());
    })
    .await
    .map_err(|_| anyhow!("请求频率限制，本轮已跳过"))?;
    let title = search_variants(&track.title)
        .last()
        .cloned()
        .unwrap_or_else(|| track.title.clone());
    let escape = |value: &str| value.replace('\\', "\\\\").replace('"', "\\\"");
    let query = format!(
        "recording:\"{}\" AND artist:\"{}\"",
        escape(&title),
        escape(&track.artist)
    );
    let payload = fetch_json_with_agent(
        client.get(&endpoints.musicbrainz).query(&[
            ("query", query.as_str()),
            ("fmt", "json"),
            ("limit", "3"),
        ]),
        deadline.min(Instant::now() + Duration::from_millis(1_800)),
        user_agent,
    )
    .await?;
    let Some(recordings) = payload.get("recordings").and_then(Value::as_array) else {
        return Err(anyhow!("录音列表格式已变化"));
    };
    for recording in recordings {
        let score = recording
            .get("score")
            .and_then(|score| score.as_u64().or_else(|| score.as_str()?.parse().ok()))
            .unwrap_or_default();
        if score < 90 {
            continue;
        }
        let Some(title) = recording.get("title").and_then(Value::as_str) else {
            continue;
        };
        let artist = recording
            .get("artist-credit")
            .and_then(Value::as_array)
            .map(|credits| {
                credits
                    .iter()
                    .filter_map(|credit| credit.get("name").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join(" / ")
            })
            .unwrap_or_default();
        let duration = recording
            .get("length")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let disambiguation = recording
            .get("disambiguation")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let actual_versions = version_tags(&format!("{title} {disambiguation}"));
        let expected_versions = version_tags(&track.title);
        if artist.is_empty()
            || jaro_winkler(&normalize(&track.artist), &normalize(&artist)) < 0.75
            || (duration > 0
                && track.duration_ms > 0
                && duration.abs_diff(track.duration_ms) > 8_000)
            || actual_versions != expected_versions
        {
            continue;
        }
        // Preserve requested version labels when a service keeps them only in
        // its disambiguation field rather than the recording title.
        return Ok(Some((
            if expected_versions.is_empty() {
                title.to_string()
            } else {
                track.title.clone()
            },
            artist,
        )));
    }
    Ok(None)
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod regression_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upgrades_netease_artwork_to_a_secure_1000_pixel_request() {
        assert_eq!(
            netease_high_resolution_url("http://example.test/cover.jpg?old=1"),
            "https://example.test/cover.jpg?param=1000y1000"
        );
    }

    #[test]
    fn exact_artwork_metadata_scores_higher_than_a_different_song() {
        let track = TrackIdentity {
            title: "わたがし".into(),
            artist: "back number".into(),
            ..Default::default()
        };
        assert!(
            artwork_metadata_score(&track, "わたがし", "back number")
                > artwork_metadata_score(&track, "高嶺の花子さん", "back number")
        );
    }

    #[tokio::test]
    #[ignore = "requires live experimental provider APIs"]
    async fn inspect_live_high_resolution_artwork() {
        let client = Client::builder().user_agent(USER_AGENT).build().unwrap();
        let track = TrackIdentity {
            title: "わたがし".into(),
            artist: "back number".into(),
            source_app: "cloudmusic.exe".into(),
            ..Default::default()
        };
        let netease = search_netease_artwork(client.clone(), track.clone()).await;
        let qq = search_qq_artwork(client.clone(), track.clone()).await;
        println!("netease artwork hit: {netease:#?}");
        println!("qq artwork hit: {qq:#?}");
        let url = search_artwork(&client, &track, &PublicSettings::default())
            .await
            .expect("no artwork found");
        let response = client.get(&url).send().await.unwrap();
        println!("selected artwork response: {}", response.status());
        let response = response.error_for_status().unwrap();
        let bytes = response.bytes().await.unwrap();
        println!("high resolution artwork: {url} ({} bytes)", bytes.len());
        assert!(url.contains("1000") || url.contains("800x800"));
        assert!(bytes.len() > 20_000);
    }
}
