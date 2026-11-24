use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, RwLock,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, oneshot},
};
use tokio_util::sync::CancellationToken;
use vibesetter_lib::{
    engine::{AudioBackend, AudioError, AudioFrame, Engine, PlaybackDone},
    model::*,
    settings::Settings,
    store::Store,
};

#[derive(Default)]
struct TestAudio {
    drain: Arc<AtomicBool>,
    samples: Arc<AtomicUsize>,
}
impl AudioBackend for TestAudio {
    fn devices(&self) -> Result<Vec<AudioDevice>> {
        Ok(vec![AudioDevice {
            id: "out".into(),
            name: "Test output".into(),
            kind: "output".into(),
            is_default: true,
        }])
    }
    fn start_inputs(
        &self,
        _: &SessionConfig,
        _: mpsc::Sender<AudioFrame>,
        _: AudioError,
        _: CancellationToken,
    ) -> Result<()> {
        Err("Test has no capture devices".into())
    }
    fn start_output(
        &self,
        _: &[String],
        mut samples: mpsc::Receiver<Vec<f32>>,
        _: AudioError,
        cancel: CancellationToken,
    ) -> Result<PlaybackDone> {
        let (tx, rx) = oneshot::channel();
        let drain = self.drain.clone();
        let count = self.samples.clone();
        std::thread::spawn(move || {
            while !cancel.is_cancelled() {
                match samples.try_recv() {
                    Ok(s) => {
                        count.fetch_add(s.len(), Ordering::Relaxed);
                    }
                    Err(mpsc::error::TryRecvError::Disconnected)
                        if drain.load(Ordering::Relaxed) =>
                    {
                        break
                    }
                    _ => {}
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let _ = tx.send(Ok(()));
        });
        Ok(rx)
    }
}

async fn request(socket: &mut tokio::net::TcpStream) -> String {
    let mut data = Vec::new();
    loop {
        let mut buf = [0; 4096];
        let n = socket.read(&mut buf).await.unwrap();
        assert!(n > 0);
        data.extend_from_slice(&buf[..n]);
        if let Some(end) = data.windows(4).position(|s| s == b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&data[..end]);
            let size = header
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            if data.len() >= end + 4 + size {
                return String::from_utf8(data).unwrap();
            }
        }
    }
}
async fn respond(socket: &mut tokio::net::TcpStream, body: &str, kind: &str) {
    let _=socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await;
}
fn completion() -> String {
    serde_json::json!({"choices":[{"message":{"content":"{\"respond\":true,\"text\":\"这是自定义助手的回应\"}"}}]}).to_string()
}
fn fixture(
    settings: Settings,
    audio: Arc<TestAudio>,
) -> (
    tempfile::TempDir,
    Arc<Engine>,
    Assistant,
    mpsc::UnboundedReceiver<SessionEvent>,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("test.db")).unwrap());
    let a = store
        .save_assistant(Assistant {
            id: "".into(),
            name: "通用助手".into(),
            prompt: "原来的提示词".into(),
            avatar: None,
            color: "violet".into(),
            created_at: 0,
            updated_at: 0,
        })
        .unwrap();
    let (events, rx) = mpsc::unbounded_channel();
    let engine = Engine::new(
        store,
        Arc::new(RwLock::new(settings)),
        audio,
        Arc::new(move |event| {
            let _ = events.send(event);
        }),
    );
    (dir, engine, a, rx)
}
async fn wait_for(
    events: &mut mpsc::UnboundedReceiver<SessionEvent>,
    predicate: impl Fn(&SessionEvent) -> bool,
) -> SessionEvent {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap();
            if predicate(&event) {
                return event;
            }
        }
    })
    .await
    .expect("expected session event")
}

#[tokio::test]
async fn a_custom_prompt_is_reloaded_and_text_roundtrip_is_persisted() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let req = request(&mut socket).await;
        assert!(req.contains("更新后的任意角色"));
        assert!(req.contains("我的新问题"));
        respond(&mut socket, &completion(), "application/json").await;
    });
    let (_dir, engine, a, mut events) = fixture(
        Settings {
            text_base_url: format!("http://{address}"),
            text_api_key: "test".into(),
            text_model: "model".into(),
            ..Default::default()
        },
        Arc::new(TestAudio::default()),
    );
    let s = engine
        .start(
            a.id.clone(),
            SessionConfig {
                text_input: true,
                text_output: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let mut updated = a;
    updated.prompt = "更新后的任意角色".into();
    engine.store.save_assistant(updated).unwrap();
    engine.submit("我的新问题".into()).await.unwrap();
    wait_for(
        &mut events,
        |e| matches!(e,SessionEvent::Message{message,..} if message.role=="assistant"),
    )
    .await;
    assert_eq!(engine.store.messages(&s.id).unwrap().len(), 2);
    engine.stop().await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn interrupt_discards_an_inflight_reply_and_session_remains_usable() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (accepted, ready) = oneshot::channel();
    let (release, wait) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        request(&mut socket).await;
        let _ = accepted.send(());
        let _ = wait.await;
        respond(&mut socket, &completion(), "application/json").await;
    });
    let (_dir, engine, a, mut events) = fixture(
        Settings {
            text_base_url: format!("http://{address}"),
            text_api_key: "test".into(),
            text_model: "model".into(),
            ..Default::default()
        },
        Arc::new(TestAudio::default()),
    );
    let s = engine
        .start(
            a.id,
            SessionConfig {
                text_input: true,
                text_output: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    engine.submit("请回应".into()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), ready)
        .await
        .unwrap()
        .unwrap();
    engine.interrupt().await.unwrap();
    wait_for(
        &mut events,
        |e| matches!(e,SessionEvent::Status{status,..} if status=="listening"),
    )
    .await;
    let _ = release.send(());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(engine.store.messages(&s.id).unwrap().len(), 1);
    assert!(engine.active_session().await.is_some());
    engine.stop().await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn audio_only_reply_stays_speaking_until_playback_drains() {
    let text = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tts = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let text_addr = text.local_addr().unwrap();
    let tts_addr = tts.local_addr().unwrap();
    let text_server = tokio::spawn(async move {
        let (mut s, _) = text.accept().await.unwrap();
        request(&mut s).await;
        respond(&mut s, &completion(), "application/json").await;
    });
    let tts_server = tokio::spawn(async move {
        let (mut s, _) = tts.accept().await.unwrap();
        request(&mut s).await;
        respond(
            &mut s,
            "data: {\"code\":0,\"data\":\"AAAAQA==\"}\n\ndata: {\"code\":20000000}\n\n",
            "text/event-stream",
        )
        .await;
    });
    let audio = Arc::new(TestAudio::default());
    let (_dir, engine, a, mut events) = fixture(
        Settings {
            text_base_url: format!("http://{text_addr}"),
            text_api_key: "test".into(),
            text_model: "model".into(),
            tts_url: format!("http://{tts_addr}"),
            speech_api_key: "speech-test".into(),
            ..Default::default()
        },
        audio.clone(),
    );
    engine
        .start(
            a.id,
            SessionConfig {
                text_input: true,
                output_ids: vec!["out".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    engine.submit("回应一次".into()).await.unwrap();
    wait_for(
        &mut events,
        |e| matches!(e,SessionEvent::Message{visible:false,message} if message.role=="assistant"),
    )
    .await;
    wait_for(
        &mut events,
        |e| matches!(e,SessionEvent::Status{status,..} if status=="speaking"),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(audio.samples.load(Ordering::Relaxed), 2);
    while let Ok(e) = events.try_recv() {
        assert!(!matches!(e,SessionEvent::Status{status,..} if status=="listening"));
    }
    audio.drain.store(true, Ordering::Relaxed);
    wait_for(
        &mut events,
        |e| matches!(e,SessionEvent::Status{status,..} if status=="listening"),
    )
    .await;
    engine.stop().await.unwrap();
    text_server.await.unwrap();
    tts_server.await.unwrap();
}
