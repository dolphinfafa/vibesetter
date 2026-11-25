use super::*;
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::oneshot,
};

struct Request {
    wire: String,
    at: Instant,
    response: oneshot::Sender<(u16, String)>,
}

impl Request {
    fn complete(self, respond: bool) {
        self.answer(
            200,
            json!({"choices":[{"message":{"content":json!({
                "respond":respond,
                "text":if respond { "自动建议" } else { "" }
            }).to_string()}}]})
            .to_string(),
        );
    }

    fn answer(self, status: u16, body: String) {
        // A cancelled model request may have already closed its connection.
        let _ = self.response.send((status, body));
    }
}

struct Service {
    url: String,
    incoming: mpsc::UnboundedReceiver<Request>,
    cancel: CancellationToken,
}

impl Service {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, incoming) = mpsc::unbounded_channel();
        let cancel = CancellationToken::new();
        let c = cancel.clone();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = tokio::select! {
                    _=c.cancelled()=>break,
                    accepted=listener.accept()=>accepted.unwrap(),
                };
                let tx = tx.clone();
                let c = c.clone();
                tokio::spawn(async move {
                    tokio::select! {
                        _=c.cancelled()=>{},
                        _=async move {
                            let mut bytes = Vec::new();
                            loop {
                                let mut buf = [0; 4096];
                                let n = socket.read(&mut buf).await.unwrap();
                                if n == 0 { return; }
                                bytes.extend_from_slice(&buf[..n]);
                                let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") else { continue; };
                                let header = String::from_utf8_lossy(&bytes[..end]);
                                let length: usize = header.lines().find_map(|line| {
                                    line.to_ascii_lowercase().strip_prefix("content-length:")?.trim().parse().ok()
                                }).unwrap();
                                if bytes.len() < end + 4 + length { continue; }
                                let (response, rx) = oneshot::channel();
                                tx.send(Request { wire: String::from_utf8(bytes).unwrap(), at: Instant::now(), response }).unwrap();
                                if let Ok((status, body)) = rx.await {
                                    let _ = socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
                                }
                                return;
                            }
                        }=>{},
                    }
                });
            }
        });
        Self {
            url,
            incoming,
            cancel,
        }
    }

    async fn request(&mut self) -> Request {
        tokio::time::timeout(Duration::from_secs(6), self.incoming.recv())
            .await
            .expect("expected a model or TTS request")
            .unwrap()
    }

    async fn no_request(&mut self, duration: Duration) {
        assert!(
            tokio::time::timeout(duration, self.incoming.recv())
                .await
                .is_err(),
            "unexpected repeated or concurrent request"
        );
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

struct Audio;
impl AudioBackend for Audio {
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
        unreachable!("tests inject transcription and voice events directly")
    }
    fn start_output(
        &self,
        _: &[String],
        mut samples: mpsc::Receiver<Vec<f32>>,
        _: AudioError,
        cancel: CancellationToken,
    ) -> Result<PlaybackDone> {
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _=cancel.cancelled()=>break,
                    Some(_)=samples.recv()=>{},
                }
            }
            let _ = tx.send(Ok(()));
        });
        Ok(rx)
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    engine: Arc<Engine>,
    session: Session,
    events: mpsc::UnboundedReceiver<SessionEvent>,
}

impl Fixture {
    async fn new(service: &Service, audio: bool, text_output: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("session.db")).unwrap());
        let assistant = store
            .save_assistant(Assistant {
                id: String::new(),
                name: "测试助手".into(),
                prompt: "根据对话提供建议".into(),
                avatar: None,
                color: "violet".into(),
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        let (tx, events) = mpsc::unbounded_channel();
        let engine = Engine::new(
            store,
            Arc::new(RwLock::new(Settings {
                text_api_key: "test".into(),
                text_model: "model".into(),
                text_base_url: service.url.clone(),
                speech_api_key: "test".into(),
                tts_url: format!("{}/tts", service.url),
                ..Default::default()
            })),
            Arc::new(Audio),
            Arc::new(move |event| {
                let _ = tx.send(event);
            }),
        );
        let session = engine
            .start(
                assistant.id,
                SessionConfig {
                    text_input: true,
                    text_output,
                    output_ids: if audio { vec!["out".into()] } else { vec![] },
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        Self {
            _dir: dir,
            engine,
            session,
            events,
        }
    }

    async fn transcript(&self, text: &str, is_final: bool) {
        self.transcript_from("test-mic", text, is_final).await;
    }

    async fn transcript_from(&self, source: &str, text: &str, is_final: bool) {
        self.engine
            .send(Input::Transcript {
                source: source.into(),
                text: text.into(),
                is_final,
            })
            .await
            .unwrap();
    }

    async fn wait(&mut self, predicate: impl Fn(&SessionEvent) -> bool) -> SessionEvent {
        tokio::time::timeout(Duration::from_secs(6), async {
            loop {
                let event = self.events.recv().await.unwrap();
                if predicate(&event) {
                    return event;
                }
            }
        })
        .await
        .expect("expected session event")
    }

    async fn decision(&mut self, expected: AutoReplyState) {
        self.wait(
            |event| matches!(event, SessionEvent::AutoReply { state, .. } if *state == expected),
        )
        .await;
    }

    fn messages(&self) -> Vec<Message> {
        self.engine.store.messages(&self.session.id).unwrap()
    }

    fn continuous_voice(&self) -> tokio::task::JoinHandle<()> {
        let engine = self.engine.clone();
        tokio::spawn(async move {
            loop {
                if engine.send(Input::Voice).await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
    }
}

#[tokio::test]
async fn text_suggestions_finish_during_continuous_speech_and_queue_new_context() {
    let mut service = Service::new().await;
    let mut f = Fixture::new(&service, false, true).await;
    let voice = f.continuous_voice();
    f.engine.send(Input::Voice).await.unwrap();
    f.transcript("第一个完整问题", true).await;
    let first = service.request().await;
    let first_at = first.at;
    assert!(first.wire.contains("可以在人们持续交谈时给出帮助"));
    f.transcript("还在变化的文字", false).await;
    f.transcript("生成期间的第二个问题", true).await;
    f.wait(|e| matches!(e, SessionEvent::Message { message, .. } if message.text == "生成期间的第二个问题")).await;
    service.no_request(Duration::from_millis(250)).await;
    first.complete(true);
    f.wait(|e| matches!(e, SessionEvent::Message { message, .. } if message.role == "assistant"))
        .await;
    f.decision(AutoReplyState::WaitingForInterval).await;
    let next = service.request().await;
    assert!(next.at.duration_since(first_at) >= Duration::from_millis(3900));
    assert!(next.wire.contains("生成期间的第二个问题"));
    assert!(next.wire.contains("自动建议"));
    assert!(!next.wire.contains("还在变化的文字"));
    let body: serde_json::Value =
        serde_json::from_str(next.wire.split_once("\r\n\r\n").unwrap().1).unwrap();
    let last = body["messages"].as_array().unwrap().last().unwrap();
    assert_eq!(last["role"], "user");
    assert!(last["content"]
        .as_str()
        .unwrap()
        .contains("只在有新的有效建议时回应"));
    next.complete(true);
    f.wait(|e| matches!(e, SessionEvent::Message { message, .. } if message.role == "assistant"))
        .await;
    assert_eq!(f.messages().len(), 4);
    voice.abort();
    f.engine.stop().await.unwrap();
}

async fn no_repeated_decision_without_new_input(failure: bool) {
    let mut service = Service::new().await;
    let mut f = Fixture::new(&service, false, true).await;
    f.transcript("暂时无需建议", true).await;
    let request = service.request().await;
    if failure {
        request.answer(503, "{}".into());
    } else {
        request.complete(false);
    }
    f.decision(if failure {
        AutoReplyState::Error
    } else {
        AutoReplyState::Skipped
    })
    .await;
    assert_eq!(f.messages().len(), 1);
    f.engine.send(Input::Voice).await.unwrap();
    f.transcript("", false).await;
    service.no_request(Duration::from_millis(4200)).await;
    f.transcript("现在出现了值得回应的新问题", true).await;
    service.request().await.complete(true);
    f.wait(|e| matches!(e, SessionEvent::Message { message, .. } if message.role == "assistant"))
        .await;
    assert_eq!(f.messages().len(), 3);
    f.engine.stop().await.unwrap();
}

#[tokio::test]
async fn skipped_decisions_wait_for_changed_input() {
    no_repeated_decision_without_new_input(false).await;
}

#[tokio::test]
async fn failed_decisions_do_not_retry_and_recover_on_new_input() {
    no_repeated_decision_without_new_input(true).await;
}

#[tokio::test]
async fn manual_stop_clears_pending_work_and_manual_or_new_input_can_resume() {
    let mut service = Service::new().await;
    let mut f = Fixture::new(&service, false, true).await;
    f.transcript("第一个问题", true).await;
    let delayed = service.request().await;
    f.transcript("等待中的第二个问题", true).await;
    f.wait(|e| matches!(e, SessionEvent::Message { message, .. } if message.text == "等待中的第二个问题")).await;
    f.engine.interrupt().await.unwrap();
    f.decision(AutoReplyState::Interrupted).await;
    delayed.complete(true);
    service.no_request(Duration::from_millis(4200)).await;
    assert_eq!(f.messages().len(), 2);
    f.engine.trigger().await.unwrap();
    let manual = service.request().await;
    assert!(manual.wire.contains("用户本次主动要求回应"));
    manual.complete(true);
    f.wait(|e| matches!(e, SessionEvent::Message { message, .. } if message.role == "assistant"))
        .await;
    f.engine.interrupt().await.unwrap();
    f.decision(AutoReplyState::Interrupted).await;
    f.transcript("停止后新的完整问题", true).await;
    let resumed = service.request().await;
    assert!(resumed.wire.contains("停止后新的完整问题"));
    resumed.complete(true);
    f.wait(|e| matches!(e, SessionEvent::Message { message, .. } if message.role == "assistant"))
        .await;
    f.engine.stop().await.unwrap();
}

async fn check_speech_routing(text_output: bool) {
    let mut service = Service::new().await;
    let mut f = Fixture::new(&service, true, text_output).await;
    let voice = f.continuous_voice();
    f.transcript("请在适当时机回应", true).await;
    f.decision(AutoReplyState::WaitingForPause).await;
    service.no_request(Duration::from_millis(1400)).await;
    voice.abort();
    let first = service.request().await;
    let first_at = first.at;
    assert!(first.wire.contains("讲话停顿后"));
    f.decision(AutoReplyState::Evaluating).await;
    f.engine.send(Input::Voice).await.unwrap();
    f.decision(AutoReplyState::WaitingForPause).await;
    first.complete(true);
    service.no_request(Duration::from_millis(500)).await;
    assert_eq!(
        f.messages().len(),
        1,
        "late cancelled reply must be discarded"
    );
    let retried = service.request().await;
    assert!(retried.at.duration_since(first_at) >= Duration::from_millis(3900));
    assert!(retried.wire.contains("请在适当时机回应"));
    retried.complete(true);
    f.wait(|e| matches!(e, SessionEvent::Message { message, visible } if message.role == "assistant" && *visible == text_output)).await;
    let tts = service.request().await;
    assert!(tts.wire.starts_with("POST /tts "));
    tts.answer(
        200,
        "{\"code\":0,\"data\":\"AAAAQA==\"}\n{\"code\":20000000}\n".into(),
    );
    f.engine.send(Input::Voice).await.unwrap();
    f.wait(|e| matches!(e, SessionEvent::Interrupted { .. }))
        .await;
    f.decision(AutoReplyState::WaitingForPause).await;
    assert!(f.messages().last().unwrap().interrupted);
    f.engine.interrupt().await.unwrap();
    f.decision(AutoReplyState::Interrupted).await;
    f.engine.stop().await.unwrap();
}

#[tokio::test]
async fn speech_only_waits_for_pause_and_preserves_context_after_voice_interrupts() {
    check_speech_routing(false).await;
}

#[tokio::test]
async fn mixed_output_waits_for_pause_and_preserves_context_after_voice_interrupts() {
    check_speech_routing(true).await;
}

#[tokio::test]
async fn stopping_session_discards_automatic_reply() {
    let mut service = Service::new().await;
    let f = Fixture::new(&service, false, true).await;
    f.transcript("自动回应前结束会话", true).await;
    let delayed = service.request().await;
    f.engine.stop().await.unwrap();
    delayed.complete(true);
    service.no_request(Duration::from_millis(200)).await;
    assert_eq!(f.messages().len(), 1);
    assert!(f.engine.active_session().await.is_none());
}

#[tokio::test]
async fn interim_only_speech_is_analyzed_periodically_and_corrections_retrigger() {
    let mut service = Service::new().await;
    let mut f = Fixture::new(&service, false, true).await;
    let voice = f.continuous_voice();
    let started = Instant::now();
    f.transcript("我想聊北京的生活", false).await;
    f.decision(AutoReplyState::WaitingForInterval).await;
    service.no_request(Duration::from_millis(1500)).await;
    let first = service.request().await;
    assert!(first.at.duration_since(started) >= Duration::from_millis(3900));
    assert!(first.wire.contains("我想聊北京的生活"));
    assert!(first.wire.contains("临时识别，尚未确认，可能修正"));
    assert!(first.wire.contains("每 4 秒的定时分析"));
    assert!(f.messages().is_empty(), "drafts must not be persisted");
    first.complete(true);
    f.wait(|e| matches!(e, SessionEvent::Message { message, .. } if message.role == "assistant"))
        .await;
    f.transcript("我想聊北京的生活", false).await;
    f.transcript("", false).await;
    service.no_request(Duration::from_millis(4200)).await;
    assert_eq!(f.messages().len(), 1);
    f.transcript("我想聊上海的生活", false).await;
    let revised = service.request().await;
    assert!(revised.wire.contains("我想聊上海的生活"));
    assert!(!revised.wire.contains("我想聊北京的生活"));
    revised.complete(false);
    f.decision(AutoReplyState::Skipped).await;
    f.transcript("我想聊上海的生活", true).await;
    f.transcript("", false).await;
    service.no_request(Duration::from_millis(4200)).await;
    assert_eq!(f.messages().len(), 2);
    voice.abort();
    f.engine.stop().await.unwrap();
}

#[tokio::test]
async fn slow_text_generation_skips_busy_ticks_and_uses_the_latest_sources() {
    let mut service = Service::new().await;
    let mut f = Fixture::new(&service, false, true).await;
    f.transcript("麦克风的原始片段", false).await;
    let first = service.request().await;
    let first_at = first.at;
    f.transcript("麦克风最新片段", false).await;
    f.transcript_from("system", "电脑音频同时说话", false).await;
    service.no_request(Duration::from_millis(4400)).await;
    first.complete(true);
    f.wait(|e| matches!(e, SessionEvent::Message { message, .. } if message.role == "assistant"))
        .await;
    service.no_request(Duration::from_millis(500)).await;
    let next = service.request().await;
    assert!(next.at.duration_since(first_at) >= Duration::from_millis(7900));
    assert!(next.wire.contains("麦克风最新片段"));
    assert!(next.wire.contains("电脑音频同时说话"));
    assert!(!next.wire.contains("麦克风的原始片段"));
    next.complete(false);
    f.decision(AutoReplyState::Skipped).await;
    f.transcript("麦克风最新片段", true).await;
    f.transcript_from("system", "电脑音频同时说话", true).await;
    service.no_request(Duration::from_millis(4200)).await;
    assert_eq!(f.messages().len(), 3);
    f.engine.stop().await.unwrap();
}

#[tokio::test]
async fn stopping_discards_pending_drafts_until_their_effective_content_changes() {
    let mut service = Service::new().await;
    let mut f = Fixture::new(&service, false, true).await;
    f.transcript("临时内容甲", false).await;
    let delayed = service.request().await;
    f.transcript("临时内容乙", false).await;
    f.wait(|e| matches!(e, SessionEvent::Transcript { text, .. } if text == "临时内容乙"))
        .await;
    f.engine.interrupt().await.unwrap();
    f.decision(AutoReplyState::Interrupted).await;
    delayed.complete(true);
    f.transcript("临时内容乙", false).await;
    f.transcript("临时内容乙", true).await;
    f.transcript("", false).await;
    service.no_request(Duration::from_millis(4200)).await;
    assert_eq!(f.messages().len(), 1);
    f.transcript("新的临时内容丙", false).await;
    let resumed = service.request().await;
    assert!(resumed.wire.contains("新的临时内容丙"));
    resumed.complete(true);
    f.wait(|e| matches!(e, SessionEvent::Message { message, .. } if message.role == "assistant"))
        .await;
    f.engine.stop().await.unwrap();
}

#[tokio::test]
async fn manual_reply_uses_current_drafts_without_waiting_for_the_timer() {
    let mut service = Service::new().await;
    let mut f = Fixture::new(&service, false, true).await;
    f.transcript("尚未断句的手动回应上下文", false).await;
    f.engine.trigger().await.unwrap();
    let request = tokio::time::timeout(Duration::from_secs(2), service.request())
        .await
        .unwrap();
    assert!(request.wire.contains("用户本次主动要求回应"));
    assert!(request.wire.contains("尚未断句的手动回应上下文"));
    assert!(request.wire.contains("尚未确认，可能修正"));
    assert!(f.messages().is_empty());
    request.complete(true);
    f.wait(|e| matches!(e, SessionEvent::Message { message, .. } if message.role == "assistant"))
        .await;
    service.no_request(Duration::from_millis(4200)).await;
    assert_eq!(f.messages().len(), 1);
    f.engine.stop().await.unwrap();
}
