//! Offline protocol fixtures. These never contact production lyric services.
use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
};

#[derive(Clone)]
struct Route {
    prefix: &'static str,
    status: u16,
    body: String,
    delay: Duration,
    retry_after: Option<&'static str>,
}

fn route(prefix: &'static str, body: &str) -> Route {
    Route {
        prefix,
        status: 200,
        body: body.into(),
        delay: Duration::ZERO,
        retry_after: None,
    }
}

struct FixtureServer {
    endpoints: Endpoints,
    stop: Arc<AtomicBool>,
    task: Option<thread::JoinHandle<()>>,
}

impl FixtureServer {
    fn new(routes: Vec<Route>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let endpoints = Endpoints {
            lrclib: format!("{base}/lrclib"),
            netease_search: format!("{base}/netease/search"),
            netease_lyric: format!("{base}/netease/lyric"),
            qq_search: format!("{base}/qq/search"),
            qq_lyric: format!("{base}/qq/lyric"),
            musicbrainz: format!("{base}/musicbrainz"),
            public_user_agent: Some("KotobaBeat-tests/0.1.0 (https://fixture.invalid)".into()),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let routes = Arc::new(routes);
        let task = thread::spawn(move || {
            while !signal.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let routes = routes.clone();
                        thread::spawn(move || {
                            // Winsock accept() inherits the listener's nonblocking
                            // mode. Reading before the client sends its bytes would
                            // otherwise fail with WouldBlock and close the socket.
                            stream.set_nonblocking(false).unwrap();
                            stream
                                .set_read_timeout(Some(Duration::from_secs(1)))
                                .unwrap();
                            let mut request = Vec::new();
                            let mut buffer = [0u8; 1024];
                            while !request.windows(4).any(|window| window == b"\r\n\r\n")
                                && request.len() < 8192
                            {
                                match stream.read(&mut buffer) {
                                    Ok(0) | Err(_) => return,
                                    Ok(length) => request.extend_from_slice(&buffer[..length]),
                                }
                            }
                            let request = String::from_utf8_lossy(&request);
                            let path = request.split_whitespace().nth(1).unwrap_or_default();
                            let found = routes.iter().find(|route| path.starts_with(route.prefix));
                            let fallback = Route {
                                status: 404,
                                ..route("", "{}")
                            };
                            let found = found.unwrap_or(&fallback);
                            thread::sleep(found.delay);
                            let retry = found
                                .retry_after
                                .map(|value| format!("Retry-After: {value}\r\n"))
                                .unwrap_or_default();
                            let header = format!("HTTP/1.1 {} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{retry}Connection: close\r\n\r\n", found.status, found.body.len());
                            let _ = stream.write_all(header.as_bytes());
                            let _ = stream.write_all(found.body.as_bytes());
                        });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1))
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            endpoints,
            stop,
            task: Some(task),
        }
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(task) = self.task.take() {
            task.join().unwrap();
        }
    }
}

fn client() -> Client {
    Client::builder().no_proxy().build().unwrap()
}

fn track() -> TrackIdentity {
    TrackIdentity {
        title: "架空の歌".into(),
        artist: "架空歌手".into(),
        duration_ms: 200_000,
        ..Default::default()
    }
}

#[test]
fn candidate_metadata_has_no_provider_disclaimer() {
    let netease = serde_json::json!({"id": 1, "name": "架空の歌"});
    let qq = serde_json::json!({"songmid": "test", "songname": "架空の歌"});
    for (provider, song) in [(Provider::Netease, netease), (Provider::Qq, qq)] {
        let candidate = candidate_metadata(provider, &song).unwrap();
        assert!(candidate.warnings.is_empty());
    }
}

const NETEASE_SEARCH: &str = r#"{"code":200,"result":{"songs":[
    {"id":1,"name":"架空の歌","ar":[{"name":"架空歌手"}],"al":{"name":"架空アルバム","picUrl":"http://example.test/cover.jpg"},"dt":200000},
    {"id":2,"name":"架空の歌","artists":[{"name":"架空歌手"}],"album":{"name":"架空アルバム"},"duration":200000},
    {"id":3,"name":"架空の歌","ar":[{"name":"架空歌手"}],"dt":200000}
]}}"#;
const NETEASE_LYRIC: &str =
    r#"{"code":200,"lrc":{"lyric":"[00:01.00]架空の空\n[02:40.00]風の音"}}"#;
const LRCLIB: &str = r#"[{"id":1,"trackName":"架空の歌","artistName":"架空歌手","albumName":"架空アルバム","duration":200,"instrumental":false,"syncedLyrics":"[00:01.00]架空の空\n[02:40.00]風の音"}]"#;

#[tokio::test]
async fn lrclib_plain_fallback_and_malformed_item_do_not_discard_valid_results() {
    let server = FixtureServer::new(vec![route(
        "/lrclib",
        r#"[
        {"id":1,"trackName":"架空の歌","artistName":"架空歌手","duration":200,"syncedLyrics":"","plainLyrics":"架空の空"},
        {"id":2,"trackName":"無音","artistName":"架空歌手","instrumental":true},
        {"id":3,"trackName":"歌詞なし","artistName":"架空歌手"},
        {"newSchema":true}
    ]"#,
    )]);
    let result = search_provider(
        Provider::LrcLib,
        &client(),
        &track(),
        &server.endpoints,
        Instant::now() + Duration::from_secs(1),
    )
    .await
    .unwrap();
    assert_eq!(result.candidates.len(), 2);
    assert!(matches!(result.candidates[0].sync_type, SyncType::Plain));
    assert!(result.candidates[1].instrumental);
    assert_eq!(result.warnings.len(), 1);
}

#[tokio::test]
async fn netease_concurrent_details_retain_success_when_another_times_out() {
    let server = FixtureServer::new(vec![
        route("/netease/search", NETEASE_SEARCH),
        route("/netease/lyric?id=1", NETEASE_LYRIC),
        Route {
            delay: Duration::from_millis(500),
            ..route("/netease/lyric?id=2", NETEASE_LYRIC)
        },
        Route {
            status: 503,
            ..route("/netease/lyric?id=3", "private response should not leak")
        },
    ]);
    let result = search_provider(
        Provider::Netease,
        &client(),
        &track(),
        &server.endpoints,
        Instant::now() + Duration::from_millis(150),
    )
    .await
    .unwrap();
    assert_eq!(result.candidates.len(), 1, "{result:?}");
    assert_eq!(result.candidates[0].provider_track_id, "1");
    assert_eq!(result.candidates[0].lines.len(), 2);
    assert!(result
        .warnings
        .iter()
        .any(|message| message.contains("超时")));
    assert!(result
        .warnings
        .iter()
        .all(|message| !message.contains("private") && !message.contains("http://")));
}

#[tokio::test]
async fn qq_accepts_jsonp_base64_and_html_entities() {
    let lyric = STANDARD.encode("&#91;00:01.00&#93;架空の空&#10;[02:40.00]風 &amp; 音");
    let server = FixtureServer::new(vec![
        route(
            "/qq/search",
            r#"MusicJsonCallback({"code":0,"data":{"song":{"list":[{"songmid":"abc","songname":"架空の歌","singer":[{"name":"架空歌手"}],"interval":200,"albummid":"cover"}]}}});"#,
        ),
        route(
            "/qq/lyric",
            &format!(r#"callback({{"code":0,"lyric":"{lyric}"}});"#),
        ),
    ]);
    let result = search_provider(
        Provider::Qq,
        &client(),
        &track(),
        &server.endpoints,
        Instant::now() + Duration::from_secs(1),
    )
    .await
    .unwrap();
    assert_eq!(result.candidates.len(), 1, "{result:?}");
    assert_eq!(result.candidates[0].lines[0].text, "架空の空");
    assert_eq!(result.candidates[0].lines[1].text, "風 & 音");
    assert!(matches!(result.candidates[0].sync_type, SyncType::Line));
}

#[tokio::test]
async fn provider_failures_do_not_block_successful_sources() {
    let server = FixtureServer::new(vec![
        route("/lrclib", LRCLIB),
        route(
            "/netease/search",
            r#"{"code":405,"message":"private server details"}"#,
        ),
        route("/qq/search", "<html>an invalid response</html>"),
    ]);
    let result = search_round(
        &client(),
        &track(),
        &PublicSettings::default(),
        &server.endpoints,
        Duration::from_secs(1),
    )
    .await;
    assert_eq!(result.candidates.len(), 1, "{result:?}");
    assert_eq!(result.warnings.len(), 2);
    assert!(result
        .warnings
        .iter()
        .all(|warning| !warning.contains("private")));
}

#[tokio::test]
async fn global_deadline_keeps_an_available_source_when_another_hangs() {
    let server = FixtureServer::new(vec![
        Route {
            delay: Duration::from_millis(500),
            ..route("/lrclib", LRCLIB)
        },
        route("/netease/search", NETEASE_SEARCH),
        route("/netease/lyric", NETEASE_LYRIC),
        route("/qq/search", r#"{"code":0,"data":{"song":{"totalnum":0}}}"#),
    ]);
    let started = Instant::now();
    let result = search_round(
        &client(),
        &track(),
        &PublicSettings::default(),
        &server.endpoints,
        Duration::from_millis(150),
    )
    .await;
    assert_eq!(result.candidates.len(), 3, "{result:?}");
    assert!(result
        .warnings
        .iter()
        .any(|warning| warning.starts_with("LRCLIB") && warning.contains("超时")));
    assert!(started.elapsed() < Duration::from_millis(450));
}

#[tokio::test]
async fn disabled_providers_are_not_requested() {
    let server = FixtureServer::new(vec![route("/lrclib", LRCLIB)]);
    let settings = PublicSettings {
        experimental_netease: false,
        experimental_qq: false,
        ..Default::default()
    };
    let result = search_round(
        &client(),
        &track(),
        &settings,
        &server.endpoints,
        Duration::from_secs(1),
    )
    .await;
    assert_eq!(result.candidates.len(), 1);
    assert!(result.warnings.is_empty());
}

#[tokio::test]
async fn schema_change_is_distinct_from_no_search_results() {
    let server = FixtureServer::new(vec![
        route(
            "/netease/search",
            r#"{"code":200,"result":{"songCount":0}}"#,
        ),
        route("/qq/search", r#"{"code":0,"newSchema":true}"#),
        route("/lrclib", "[]"),
    ]);
    let result = search_round(
        &client(),
        &track(),
        &PublicSettings::default(),
        &server.endpoints,
        Duration::from_secs(1),
    )
    .await;
    assert!(result.candidates.is_empty());
    assert_eq!(result.warnings.len(), 1, "{result:?}");
    assert!(result.warnings[0].contains("格式已变化"));
}

#[tokio::test]
async fn oversized_response_is_rejected_before_deserialization() {
    let server = FixtureServer::new(vec![route("/lrclib", &"x".repeat(MAX_RESPONSE_BYTES + 1))]);
    let error = search_provider(
        Provider::LrcLib,
        &client(),
        &track(),
        &server.endpoints,
        Instant::now() + Duration::from_secs(1),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("响应过大"));
}

#[tokio::test]
async fn missing_or_changed_platform_song_fields_are_reported() {
    let server = FixtureServer::new(vec![route(
        "/netease/search",
        r#"{"code":200,"result":{"songs":[{"unexpected_song_id":12}]}}"#,
    )]);
    let error = search_provider(
        Provider::Netease,
        &client(),
        &track(),
        &server.endpoints,
        Instant::now() + Duration::from_secs(1),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("字段格式已变化"));
}

#[tokio::test]
async fn musicbrainz_does_not_replace_a_recording_with_the_wrong_version() {
    let server = FixtureServer::new(vec![route(
        "/musicbrainz",
        r#"{"recordings":[
        {"score":100,"title":"架空の歌 (Live)","length":200000,"disambiguation":"live","artist-credit":[{"name":"架空歌手"}]},
        {"score":"99","title":"架空の歌","length":200000,"artist-credit":[{"name":"架空歌手"}]}
    ]}"#,
    )]);
    let canonical = canonicalize_musicbrainz(
        &client(),
        &track(),
        &server.endpoints,
        Instant::now() + Duration::from_secs(3),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(canonical, ("架空の歌".into(), "架空歌手".into()));
}

#[test]
fn jsonp_parser_rejects_embedded_objects_in_html_or_scripts() {
    assert!(parse_json_or_jsonp("<html>{\"code\":0}</html>").is_err());
    assert!(parse_json_or_jsonp("cb({\"code\":0});alert(1)").is_err());
    assert_eq!(
        parse_json_or_jsonp(" cb({\"code\":0}); ").unwrap()["code"],
        0
    );
}

#[test]
fn duration_values_and_entities_are_sanitized() {
    assert_eq!(duration_millis(f64::INFINITY), 0);
    assert_eq!(duration_millis(-1.0), 0);
    assert_eq!(duration_millis(201.123), 201_123);
    assert_eq!(
        decode_lyric_entities("&#x65E5; &amp; &#99999999;"),
        "日 & &#99999999;"
    );
}

#[test]
fn artwork_requires_artist_evidence_and_compatible_versions() {
    let mut identity = track();
    identity.artist.clear();
    assert_eq!(artwork_metadata_score(&identity, &identity.title, ""), 0.0);
    let identity = track();
    assert_eq!(
        artwork_metadata_score(&identity, "架空の歌 (Live)", &identity.artist),
        0.0
    );
}

#[test]
fn contact_configuration_requires_an_explicit_valid_url_or_email() {
    assert!(contact_user_agent(None).is_none());
    assert!(contact_user_agent(Some("")).is_none());
    assert!(contact_user_agent(Some("not a contact")).is_none());
    assert!(contact_user_agent(Some("https://user:secret@example.test/")).is_none());
    assert!(contact_user_agent(Some("https://example.test/?token=secret")).is_none());
    assert!(contact_user_agent(Some("someone@example.test\r\nHeader: injected")).is_none());
    assert!(contact_user_agent(Some("https://example.test/project")).is_some());
    assert!(contact_user_agent(Some("someone@example.test")).is_some());
}

#[tokio::test]
async fn public_services_skip_without_contact_instead_of_sending_an_anonymous_request() {
    let mut server = FixtureServer::new(Vec::new());
    server.endpoints.public_user_agent = None;
    let error = search_provider(
        Provider::LrcLib,
        &client(),
        &track(),
        &server.endpoints,
        Instant::now() + Duration::from_secs(1),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("未配置歌词服务联系信息"));
    let error = canonicalize_musicbrainz(
        &client(),
        &track(),
        &server.endpoints,
        Instant::now() + Duration::from_secs(1),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("未配置歌词服务联系信息"));
}

#[tokio::test]
async fn retry_after_prevents_immediate_followup_requests() {
    let server = FixtureServer::new(vec![Route {
        status: 429,
        retry_after: Some("60"),
        ..route("/lrclib", "{}")
    }]);
    let client = client();
    let first = search_provider(
        Provider::LrcLib,
        &client,
        &track(),
        &server.endpoints,
        Instant::now() + Duration::from_secs(1),
    )
    .await
    .unwrap_err();
    assert!(first.to_string().contains("HTTP 429"));
    let second = search_provider(
        Provider::LrcLib,
        &client,
        &track(),
        &server.endpoints,
        Instant::now() + Duration::from_secs(1),
    )
    .await
    .unwrap_err();
    assert!(second.to_string().contains("冷却"));
}

#[test]
fn retry_after_accepts_seconds_and_http_dates() {
    let mut headers = header::HeaderMap::new();
    headers.insert(header::RETRY_AFTER, "120".parse().unwrap());
    assert_eq!(retry_after_delay(&headers), Some(Duration::from_secs(120)));
    // Generate a matching weekday/date, as RFC 2822 parsers validate both.
    let future = (chrono::Utc::now() + chrono::Duration::seconds(90)).to_rfc2822();
    headers.insert(header::RETRY_AFTER, future.parse().unwrap());
    assert!(retry_after_delay(&headers).unwrap() >= Duration::from_secs(88));
}

#[tokio::test]
#[ignore = "manual test: contacts public third-party lyric APIs, no translation API"]
async fn inspect_live_lyrics_providers() {
    let client = Client::builder().build().unwrap();
    let track = TrackIdentity {
        title: "わたがし".into(),
        artist: "back number".into(),
        source_app: "cloudmusic.exe".into(),
        ..Default::default()
    };
    let started = Instant::now();
    let result = search_all(&client, &track, &PublicSettings::default()).await;
    println!(
        "elapsed_ms={}, candidates={}, warnings={:?}",
        started.elapsed().as_millis(),
        result.candidates.len(),
        result.warnings
    );
    assert!(started.elapsed() < Duration::from_secs(6));
}
