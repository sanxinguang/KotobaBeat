use crate::models::LineAnalysis;
use anyhow::Result;
use futures::{stream::FuturesUnordered, StreamExt};
use std::{
    collections::{HashSet, VecDeque},
    future::Future,
};

struct Job {
    ids: Vec<String>,
    retried: bool,
}

pub fn retryable(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<reqwest::Error>()
        .and_then(reqwest::Error::status)
        .is_none_or(|status| {
            status.as_u16() == 429 || status.as_u16() == 408 || status.is_server_error()
        })
}

pub fn error_message(error: &anyhow::Error) -> String {
    if let Some(http) = error.downcast_ref::<reqwest::Error>() {
        if http.is_timeout() {
            return "翻译请求超时，请稍后重试".into();
        }
        return match http.status().map(|status| status.as_u16()) {
            Some(401 | 403) => "翻译服务认证失败，请在设置中检查 API Key".into(),
            Some(402) => "翻译服务余额不足，请检查账户余额".into(),
            Some(429) => "翻译服务繁忙或请求受限，请稍后重试".into(),
            Some(code) => format!("翻译服务暂时不可用（HTTP {code}）"),
            None => "无法连接翻译服务，请检查网络后重试".into(),
        };
    }
    error.to_string()
}

/// Deliver every completed batch immediately. Only missing rows get one
/// smaller retry; dropping this future cancels queued and in-flight requests.
pub async fn run_batches<F, Fut, Publish>(
    batches: Vec<Vec<String>>,
    request: F,
    mut publish: Publish,
) -> Result<()>
where
    F: Fn(Vec<String>) -> Fut,
    Fut: Future<Output = Result<Vec<LineAnalysis>>>,
    Publish: FnMut(Vec<LineAnalysis>, Option<String>) -> Result<()>,
{
    let mut scheduled = HashSet::new();
    let mut queue: VecDeque<_> = batches
        .into_iter()
        .map(|ids| {
            ids.into_iter()
                .filter(|id| !id.is_empty() && scheduled.insert(id.clone()))
                .collect::<Vec<_>>()
        })
        .filter(|ids| !ids.is_empty())
        .map(|ids| Job {
            ids,
            retried: false,
        })
        .collect();
    let mut active = FuturesUnordered::new();
    let mut consecutive_failures = 0;
    loop {
        while active.len() < 2 {
            let Some(job) = queue.pop_front() else {
                break;
            };
            let future = request(job.ids.clone());
            active.push(async move {
                if job.retried {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                (job, future.await)
            });
        }
        let Some((job, result)) = active.next().await else {
            break;
        };
        let fatal = result.as_ref().err().is_some_and(|error| !retryable(error));
        let (lines, error) = match result {
            Ok(lines) => (lines, None),
            Err(error) => (Vec::new(), Some(error_message(&error))),
        };
        let mut seen = HashSet::new();
        let lines: Vec<_> = lines
            .into_iter()
            .filter(|line| {
                !line.translation_zh.trim().is_empty()
                    && job.ids.contains(&line.line_id)
                    && seen.insert(line.line_id.clone())
            })
            .collect();
        let missing: Vec<_> = job
            .ids
            .into_iter()
            .filter(|id| !seen.contains(id))
            .collect();
        consecutive_failures = if lines.is_empty() {
            consecutive_failures + 1
        } else {
            0
        };
        let stop_queue = fatal || consecutive_failures >= 3;
        if !lines.is_empty() {
            publish(lines, None)?;
        }
        if missing.is_empty() {
            continue;
        }
        if stop_queue || job.retried {
            publish(
                Vec::new(),
                Some(error.unwrap_or_else(|| "部分歌词未返回有效翻译，可手动重试".into())),
            )?;
            if stop_queue {
                break;
            }
        } else {
            for ids in missing.chunks(2).rev() {
                queue.push_front(Job {
                    ids: ids.to_vec(),
                    retried: true,
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn line(id: &str) -> LineAnalysis {
        LineAnalysis {
            line_id: id.into(),
            translation_zh: "译文".into(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn repeated_service_failures_stop_before_draining_the_song() {
        let requests = Mutex::new(0);
        let mut errors = Vec::new();
        run_batches(
            (0..30).map(|i| vec![i.to_string()]).collect(),
            |_| {
                *requests.lock().unwrap() += 1;
                async { anyhow::bail!("service unavailable") }
            },
            |_, error| {
                errors.extend(error);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert!(*requests.lock().unwrap() <= 4);
        assert!(!errors.is_empty());
    }

    #[tokio::test]
    async fn cancellation_drops_inflight_requests_and_never_starts_queued_work() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Active(Arc<AtomicUsize>);
        impl Drop for Active {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let active = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(AtomicUsize::new(0));
        let ready = Arc::new(tokio::sync::Notify::new());
        let task = tokio::spawn({
            let active = active.clone();
            let started = started.clone();
            let ready = ready.clone();
            async move {
                run_batches(
                    (0..8).map(|i| vec![i.to_string()]).collect(),
                    move |_| {
                        let active = active.clone();
                        let started = started.clone();
                        let ready = ready.clone();
                        async move {
                            let _guard = Active(active.clone());
                            active.fetch_add(1, Ordering::SeqCst);
                            if started.fetch_add(1, Ordering::SeqCst) == 1 {
                                ready.notify_one();
                            }
                            std::future::pending::<Result<Vec<LineAnalysis>>>().await
                        }
                    },
                    |_, _| panic!("cancelled results must not publish"),
                )
                .await
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), ready.notified())
            .await
            .unwrap();
        assert_eq!(active.load(Ordering::SeqCst), 2);
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert_eq!(started.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn authentication_failure_stops_queue_without_retrying() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let mut buffer = [0; 2048];
            let _ = socket.read(&mut buffer);
            socket
                .write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });
        let response = reqwest::Client::new()
            .get(format!("http://{address}"))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap_err();
        server.join().unwrap();
        let error = Mutex::new(Some(anyhow::Error::new(response)));
        let calls = Mutex::new(0);
        let mut errors = Vec::new();
        run_batches(
            (0..6).map(|i| vec![i.to_string()]).collect(),
            |_| {
                let failure = error.lock().unwrap().take();
                *calls.lock().unwrap() += 1;
                async move {
                    match failure {
                        Some(error) => Err(error),
                        None => std::future::pending().await,
                    }
                }
            },
            |_, error| {
                errors.extend(error);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert!(*calls.lock().unwrap() <= 2);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("认证失败"));
    }

    #[tokio::test]
    async fn publishes_fast_batch_before_slow_batch_finishes() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let output = Arc::new(Mutex::new(Vec::new()));
        let delivered = Arc::new(tokio::sync::Notify::new());
        let task = tokio::spawn({
            let gate = gate.clone();
            let output = output.clone();
            let delivered = delivered.clone();
            async move {
                run_batches(
                    vec![vec!["slow".into()], vec!["fast".into()]],
                    move |ids| {
                        let gate = gate.clone();
                        async move {
                            if ids[0] == "slow" {
                                gate.notified().await;
                            }
                            Ok(vec![line(&ids[0])])
                        }
                    },
                    move |lines, _| {
                        output
                            .lock()
                            .unwrap()
                            .extend(lines.into_iter().map(|l| l.line_id));
                        delivered.notify_one();
                        Ok(())
                    },
                )
                .await
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), delivered.notified())
            .await
            .unwrap();
        assert_eq!(*output.lock().unwrap(), vec!["fast"]);
        gate.notify_one();
        task.await.unwrap().unwrap();
        assert_eq!(*output.lock().unwrap(), vec!["fast", "slow"]);
    }

    #[tokio::test]
    async fn retries_only_missing_rows_once_and_keeps_successes() {
        let requests = Mutex::new(Vec::new());
        let mut output = Vec::new();
        let mut errors = Vec::new();
        run_batches(
            vec![vec!["good".into(), "bad".into()]],
            |ids| {
                requests.lock().unwrap().push(ids.clone());
                async move {
                    if ids.contains(&"good".to_string()) {
                        Ok(vec![line("good")])
                    } else {
                        anyhow::bail!("invalid JSON")
                    }
                }
            },
            |lines, error| {
                output.extend(lines);
                errors.extend(error);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(
            *requests.lock().unwrap(),
            vec![vec!["good", "bad"], vec!["bad"]]
        );
        assert_eq!(output.len(), 1);
        assert_eq!(errors.len(), 1);
    }

    #[tokio::test]
    async fn duplicate_input_ids_are_requested_only_once() {
        let requested = Mutex::new(Vec::new());
        let mut output = Vec::new();
        run_batches(
            vec![
                vec!["a".into(), "a".into(), "".into()],
                vec!["a".into(), "b".into()],
            ],
            |ids| {
                requested.lock().unwrap().extend(ids.clone());
                async move { Ok(ids.iter().map(|id| line(id)).collect()) }
            },
            |lines, _| {
                output.extend(lines);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(*requested.lock().unwrap(), vec!["a", "b"]);
        assert_eq!(output.len(), 2);
    }

    #[tokio::test]
    async fn blank_foreign_and_duplicate_results_do_not_count_as_successes() {
        let requested = Mutex::new(Vec::new());
        let mut output = Vec::new();
        run_batches(
            vec![vec!["a".into(), "b".into()]],
            |ids| {
                requested.lock().unwrap().push(ids.clone());
                async move {
                    if ids.len() == 2 {
                        Ok(vec![
                            line("a"),
                            line("a"),
                            line("foreign"),
                            LineAnalysis {
                                line_id: "b".into(),
                                translation_zh: " ".into(),
                                ..Default::default()
                            },
                        ])
                    } else {
                        Ok(vec![line("b")])
                    }
                }
            },
            |lines, _| {
                output.extend(lines);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(*requested.lock().unwrap(), vec![vec!["a", "b"], vec!["b"]]);
        assert_eq!(
            output
                .iter()
                .map(|row| row.line_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }
}
