use crate::{model::*, provider, settings::Settings, store::Store};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

mod text_context;
use text_context::TextContext;

pub type Emit = Arc<dyn Fn(SessionEvent) + Send + Sync>;
pub type AudioError = Arc<dyn Fn(String, String) + Send + Sync>;
pub struct AudioFrame {
    pub source: String,
    pub samples: Vec<i16>,
}
pub type PlaybackDone = tokio::sync::oneshot::Receiver<Result<()>>;

pub trait AudioBackend: Send + Sync {
    fn devices(&self) -> Result<Vec<AudioDevice>>;
    fn start_inputs(
        &self,
        config: &SessionConfig,
        frames: mpsc::Sender<AudioFrame>,
        error: AudioError,
        cancel: CancellationToken,
    ) -> Result<()>;
    fn start_output(
        &self,
        ids: &[String],
        samples: mpsc::Receiver<Vec<f32>>,
        error: AudioError,
        cancel: CancellationToken,
    ) -> Result<PlaybackDone>;
}

enum Input {
    Text(String),
    Trigger,
    Transcript {
        source: String,
        text: String,
        is_final: bool,
    },
    Voice,
    Ready {
        generation: u64,
        result: Result<provider::Reply>,
    },
    Played {
        generation: u64,
        result: Result<()>,
    },
    Interrupt,
}
struct Active {
    session: Session,
    config: SessionConfig,
    tx: mpsc::Sender<Input>,
    cancel: CancellationToken,
}

pub struct Engine {
    pub store: Arc<Store>,
    pub settings: Arc<RwLock<Settings>>,
    audio: Arc<dyn AudioBackend>,
    emit: Emit,
    active: Mutex<Option<Active>>,
}

impl Engine {
    pub fn new(
        store: Arc<Store>,
        settings: Arc<RwLock<Settings>>,
        audio: Arc<dyn AudioBackend>,
        emit: Emit,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            settings,
            audio,
            emit,
            active: Mutex::new(None),
        })
    }
    pub fn devices(&self) -> Result<Vec<AudioDevice>> {
        self.audio.devices()
    }
    pub async fn start(
        self: &Arc<Self>,
        assistant_id: String,
        config: SessionConfig,
    ) -> Result<Session> {
        config.validate()?;
        self.store.assistant(&assistant_id)?;
        let settings = self.settings.read().map_err(|_| "配置正在恢复")?.clone();
        settings.validate_text()?;
        if config.has_audio_input() || !config.output_ids.is_empty() {
            settings.validate_speech()?;
        }
        if !config.output_ids.is_empty() {
            let devices = self.audio.devices()?;
            if config
                .output_ids
                .iter()
                .any(|id| !devices.iter().any(|d| d.id == *id && d.kind == "output"))
            {
                return Err("所选音频输出设备已断开，请重新选择".into());
            }
        }
        let mut active = self.active.lock().await;
        if active.is_some() {
            return Err("请先结束当前会话".into());
        }
        let session = self.store.create_session(&assistant_id)?;
        let cancel = CancellationToken::new();
        let (tx, rx) = mpsc::channel(128);
        let (frames, mut frame_rx) = mpsc::channel::<AudioFrame>(24);
        let root_cancel = cancel.clone();
        let mut source_ids = config.microphone_ids.clone();
        if config.system_audio {
            source_ids.push("system".into());
        }
        let source_cancels: HashMap<_, _> = source_ids
            .into_iter()
            .map(|id| (id, cancel.child_token()))
            .collect();
        let source_cancels = Arc::new(source_cancels);
        let failed_sources = source_cancels.clone();
        let error_emit = self.emit.clone();
        let error_id = session.id.clone();
        let errors: AudioError = Arc::new(move |source, message| {
            if let Some(token) = failed_sources.get(&source) {
                token.cancel();
            }
            if !root_cancel.is_cancelled() {
                error_emit(SessionEvent::Error {
                    session_id: error_id.clone(),
                    source,
                    message,
                });
            }
        });
        if config.has_audio_input() {
            let audio = self.audio.clone();
            let cfg = config.clone();
            let c = cancel.clone();
            let e = errors.clone();
            let start = tokio::task::spawn_blocking(move || audio.start_inputs(&cfg, frames, e, c))
                .await
                .unwrap_or_else(|_| Err("启动采集任务失败".into()));
            if let Err(err) = start {
                cancel.cancel();
                let _ = self.store.end_session(&session.id);
                return Err(err);
            }
        }
        if let Err(err) = self.store.save_route(&assistant_id, &config) {
            cancel.cancel();
            let _ = self.store.end_session(&session.id);
            return Err(err);
        }
        let mut sources = config.microphone_ids.clone();
        if config.system_audio {
            sources.push("system".into());
        }
        let mut audio_senders = HashMap::new();
        for source in sources {
            let (sender, receiver) = mpsc::channel(12);
            audio_senders.insert(source.clone(), sender);
            let callback_tx = tx.clone();
            let callback_source = source.clone();
            let c = source_cancels
                .get(&source)
                .expect("configured audio source")
                .clone();
            let cfg = settings.clone();
            let report = errors.clone();
            tokio::spawn(async move {
                let callback = Arc::new(move |t: provider::Transcript| {
                    let _ = callback_tx.try_send(Input::Transcript {
                        source: callback_source.clone(),
                        text: t.text,
                        is_final: t.is_final,
                    });
                });
                if let Err(error) = provider::recognize(cfg, receiver, callback, c).await {
                    report(source, error);
                }
            });
        }
        let frame_cancel = cancel.clone();
        let frame_emit = self.emit.clone();
        let frame_id = session.id.clone();
        let voice_tx = tx.clone();
        let frame_error = errors.clone();
        tokio::task::spawn_blocking(move || {
            let mut voice_frames: HashMap<String, u32> = HashMap::new();
            let mut detectors = HashMap::new();
            while !frame_cancel.is_cancelled() {
                let frame = match frame_rx.try_recv() {
                    Ok(f) => f,
                    Err(mpsc::error::TryRecvError::Disconnected) => break,
                    Err(mpsc::error::TryRecvError::Empty) => {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                };
                if source_cancels
                    .get(&frame.source)
                    .is_some_and(|c| c.is_cancelled())
                {
                    audio_senders.remove(&frame.source);
                    continue;
                }
                let rms = (frame
                    .samples
                    .iter()
                    .map(|v| (*v as f32 / 32768.0).powi(2))
                    .sum::<f32>()
                    / frame.samples.len().max(1) as f32)
                    .sqrt();
                frame_emit(SessionEvent::Level {
                    session_id: frame_id.clone(),
                    source: frame.source.clone(),
                    value: (rms * 5.0).min(1.0),
                });
                let count = voice_frames.entry(frame.source.clone()).or_default();
                let vad = detectors.entry(frame.source.clone()).or_insert_with(|| {
                    webrtc_vad::Vad::new_with_rate_and_mode(
                        webrtc_vad::SampleRate::Rate16kHz,
                        webrtc_vad::VadMode::Aggressive,
                    )
                });
                let voiced = frame
                    .samples
                    .as_chunks::<320>()
                    .0
                    .iter()
                    .filter(|chunk| vad.is_voice_segment(*chunk).unwrap_or(false))
                    .count()
                    >= 3;
                if voiced {
                    *count += 1;
                    if *count >= 2 {
                        let _ = voice_tx.try_send(Input::Voice);
                    }
                } else {
                    *count = 0;
                }
                if let Some(sender) = audio_senders.get(&frame.source) {
                    if sender.try_send(frame.samples).is_err() {
                        let source = frame.source;
                        audio_senders.remove(&source);
                        frame_error(
                            source,
                            "语音上传缓冲已满或连接中断，请暂停后重新开始此来源".into(),
                        );
                    }
                }
            }
        });
        *active = Some(Active {
            session: session.clone(),
            config: config.clone(),
            tx: tx.clone(),
            cancel: cancel.clone(),
        });
        let engine = self.clone();
        let s = session.clone();
        tokio::spawn(async move {
            engine
                .run_session(s, config, settings, rx, tx, cancel, errors)
                .await;
        });
        (self.emit)(SessionEvent::Status {
            session_id: session.id.clone(),
            status: "listening".into(),
        });
        Ok(session)
    }
    pub async fn stop(&self) -> Result<()> {
        if let Some(active) = self.active.lock().await.take() {
            active.cancel.cancel();
            let result = self.store.end_session(&active.session.id);
            (self.emit)(SessionEvent::Status {
                session_id: active.session.id,
                status: "stopped".into(),
            });
            return result;
        }
        Ok(())
    }
    pub async fn is_assistant_active(&self, id: &str) -> bool {
        self.active
            .lock()
            .await
            .as_ref()
            .is_some_and(|a| a.session.assistant_id == id)
    }
    pub async fn active_session(&self) -> Option<Session> {
        self.active.lock().await.as_ref().map(|a| a.session.clone())
    }
    pub async fn submit(&self, text: String) -> Result<()> {
        if text.trim().is_empty() || text.chars().count() > 12000 {
            return Err("请输入 1–12000 个字符".into());
        }
        let lock = self.active.lock().await;
        let active = lock.as_ref().ok_or("请先开始会话")?;
        if !active.config.text_input {
            return Err("当前未启用文字输入".into());
        }
        active
            .tx
            .send(Input::Text(text.trim().into()))
            .await
            .map_err(|_| "会话已结束".into())
    }
    pub async fn trigger(&self) -> Result<()> {
        self.send(Input::Trigger).await
    }
    pub async fn interrupt(&self) -> Result<()> {
        self.send(Input::Interrupt).await
    }
    async fn send(&self, input: Input) -> Result<()> {
        self.active
            .lock()
            .await
            .as_ref()
            .ok_or("请先开始会话")?
            .tx
            .send(input)
            .await
            .map_err(|_| "会话已结束".into())
    }
    #[allow(clippy::too_many_arguments)]
    async fn run_session(
        self: Arc<Self>,
        session: Session,
        config: SessionConfig,
        settings: Settings,
        mut rx: mpsc::Receiver<Input>,
        tx: mpsc::Sender<Input>,
        cancel: CancellationToken,
        error: AudioError,
    ) {
        let mut generation = 0_u64;
        let mut running: Option<CancellationToken> = None;
        let mut last_message: Option<String> = None;
        let mut pending = false;
        let mut manual = false;
        let mut last_voice = Instant::now();
        let mut last_attempt = Instant::now() - Duration::from_secs(10);
        let audio_reply = !config.output_ids.is_empty();
        let mut text_context = TextContext::default();
        let mut text_changed = false;
        let mut decision = AutoReplyState::Listening;
        let mut reported_decision = None;
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        let mut text_tick = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_secs(4),
            Duration::from_secs(4),
        );
        text_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let emit = self.emit.clone();
        let status = |s: &str| {
            if !cancel.is_cancelled() {
                emit(SessionEvent::Status {
                    session_id: session.id.clone(),
                    status: s.into(),
                });
            }
        };
        loop {
            if cancel.is_cancelled() {
                break;
            }
            if reported_decision != Some(decision) {
                emit(SessionEvent::AutoReply {
                    session_id: session.id.clone(),
                    state: decision,
                });
                reported_decision = Some(decision);
            }
            let (input, text_due) = tokio::select! {
                _=cancel.cancelled()=>break,
                _=tick.tick()=>(None, false),
                _=text_tick.tick(), if !audio_reply=>(None, true),
                i=rx.recv()=>match i { Some(i)=>(Some(i), false), None=>break },
            };
            let mut interrupt = false;
            match input {
                Some(Input::Text(text)) => {
                    let m = Message {
                        id: uuid::Uuid::new_v4().to_string(),
                        session_id: session.id.clone(),
                        role: "user".into(),
                        source: "文字输入".into(),
                        text,
                        created_at: now(),
                        interrupted: false,
                    };
                    match self.store.add_message(&m) {
                        Ok(()) => {
                            if !audio_reply {
                                text_context.confirm(&m.source, &m.text);
                                text_changed = text_context.changed();
                            }
                            emit(SessionEvent::Message {
                                message: m,
                                visible: true,
                            });
                            pending = true;
                            manual = true;
                            interrupt = true;
                        }
                        Err(e) => error("storage".into(), e),
                    }
                }
                Some(Input::Trigger) => {
                    pending = true;
                    manual = true;
                    interrupt = true;
                }
                Some(Input::Interrupt) => {
                    pending = false;
                    manual = false;
                    text_context.mark_handled();
                    text_changed = false;
                    decision = AutoReplyState::Interrupted;
                    interrupt = true;
                }
                Some(Input::Voice) => {
                    last_voice = Instant::now();
                    if audio_reply && running.is_some() {
                        // Revisit this context after the speaker pauses, even if ASR
                        // has not delivered another complete utterance yet.
                        pending = true;
                        interrupt = true;
                    }
                }
                Some(Input::Transcript {
                    source,
                    text,
                    is_final,
                }) => {
                    if !is_final {
                        if !audio_reply {
                            text_context.partial(&source, &text);
                            text_changed = text_context.changed();
                        }
                        emit(SessionEvent::Transcript {
                            session_id: session.id.clone(),
                            source,
                            text,
                            is_final: false,
                        });
                    } else if !text.trim().is_empty() {
                        let m = Message {
                            id: uuid::Uuid::new_v4().to_string(),
                            session_id: session.id.clone(),
                            role: "user".into(),
                            source: source.clone(),
                            text,
                            created_at: now(),
                            interrupted: false,
                        };
                        emit(SessionEvent::Transcript {
                            session_id: session.id.clone(),
                            source,
                            text: String::new(),
                            is_final: false,
                        });
                        match self.store.add_message(&m) {
                            Ok(()) => {
                                if !audio_reply {
                                    text_context.confirm(&m.source, &m.text);
                                    text_changed = text_context.changed();
                                }
                                emit(SessionEvent::Message {
                                    message: m,
                                    visible: true,
                                });
                                pending = audio_reply;
                                last_voice = Instant::now();
                                interrupt = audio_reply && running.is_some();
                            }
                            Err(e) => error("storage".into(), e),
                        }
                    }
                }
                Some(Input::Ready {
                    generation: g,
                    result,
                }) if g == generation => match result {
                    Ok(reply) if reply.respond => {
                        let m = Message {
                            id: uuid::Uuid::new_v4().to_string(),
                            session_id: session.id.clone(),
                            role: "assistant".into(),
                            source: "assistant".into(),
                            text: reply.text.clone(),
                            created_at: now(),
                            interrupted: false,
                        };
                        if let Err(e) = self.store.add_message(&m) {
                            error("storage".into(), e);
                            running = None;
                            decision = AutoReplyState::Error;
                            status("listening");
                            continue;
                        }
                        decision = AutoReplyState::Listening;
                        last_message = Some(m.id.clone());
                        emit(SessionEvent::Message {
                            message: m,
                            visible: config.text_output,
                        });
                        if config.output_ids.is_empty() {
                            running = None;
                            last_message = None;
                            status("listening");
                        } else {
                            status("speaking");
                            let c = running.as_ref().unwrap().clone();
                            let s = settings.clone();
                            let outputs = config.output_ids.clone();
                            let audio = self.audio.clone();
                            let report = error.clone();
                            let done = tx.clone();
                            tokio::spawn(async move {
                                let (samples, receiver) = mpsc::channel(8);
                                let play_cancel = c.clone();
                                let started = tokio::task::spawn_blocking(move || {
                                    audio.start_output(&outputs, receiver, report, play_cancel)
                                })
                                .await;
                                let result = match started {
                                    Ok(Ok(finished)) => {
                                        match provider::synthesize(&s, &reply.text, samples, &c)
                                            .await
                                        {
                                            Ok(()) => {
                                                tokio::select! {_=c.cancelled()=>Ok(()),done=finished=>done.unwrap_or_else(|_|Err("播放任务异常结束".into()))}
                                            }
                                            Err(e) => {
                                                c.cancel();
                                                Err(e)
                                            }
                                        }
                                    }
                                    Ok(Err(e)) => Err(e),
                                    Err(_) => Err("启动播放任务失败".into()),
                                };
                                let _ = done
                                    .send(Input::Played {
                                        generation: g,
                                        result,
                                    })
                                    .await;
                            });
                        }
                    }
                    Ok(_) => {
                        running = None;
                        decision = AutoReplyState::Skipped;
                        status("listening");
                    }
                    Err(e) => {
                        if running.as_ref().is_some_and(|c| !c.is_cancelled()) {
                            error("text".into(), e);
                        }
                        running = None;
                        decision = AutoReplyState::Error;
                        status("listening");
                    }
                },
                Some(Input::Played {
                    generation: g,
                    result,
                }) if g == generation => {
                    if let Err(e) = result {
                        error("tts".into(), e);
                        decision = AutoReplyState::Error;
                    } else {
                        decision = AutoReplyState::Listening;
                    }
                    running = None;
                    last_message = None;
                    status("listening");
                }
                _ => {}
            }
            if interrupt {
                if let Some(c) = running.take() {
                    c.cancel();
                    generation += 1;
                    if let Some(id) = last_message.take() {
                        if let Err(e) = self.store.interrupt_message(&id) {
                            error("storage".into(), e);
                        } else {
                            emit(SessionEvent::Interrupted {
                                session_id: session.id.clone(),
                                message_id: id,
                            });
                        }
                    }
                }
                status("listening");
            }
            if running.is_some() {
                continue;
            }
            if !audio_reply && !manual {
                if !text_changed {
                    if decision == AutoReplyState::WaitingForInterval {
                        decision = AutoReplyState::Listening;
                    }
                    continue;
                }
                if !text_due {
                    decision = AutoReplyState::WaitingForInterval;
                    continue;
                }
            } else if !pending {
                continue;
            }
            if !manual && audio_reply && last_voice.elapsed() < Duration::from_millis(1200) {
                decision = AutoReplyState::WaitingForPause;
            } else if !manual && audio_reply && last_attempt.elapsed() < Duration::from_secs(4) {
                decision = AutoReplyState::WaitingForInterval;
            } else {
                pending = false;
                let forced = std::mem::take(&mut manual);
                if !audio_reply {
                    // Includes skipped, failed and manually stopped contexts:
                    // only a different effective input can trigger another attempt.
                    text_context.mark_handled();
                    text_changed = false;
                    text_tick.reset_after(Duration::from_secs(4));
                }
                generation += 1;
                last_attempt = Instant::now();
                let prompt = match self.store.assistant(&session.assistant_id) {
                    Ok(a) => a.prompt,
                    Err(e) => {
                        error("assistant".into(), e);
                        decision = AutoReplyState::Error;
                        continue;
                    }
                };
                let mut messages = match self.store.messages(&session.id) {
                    Ok(m) => m,
                    Err(e) => {
                        error("storage".into(), e);
                        decision = AutoReplyState::Error;
                        continue;
                    }
                };
                if !audio_reply {
                    text_context.append_drafts(&mut messages, &session.id);
                }
                let c = cancel.child_token();
                running = Some(c.clone());
                decision = AutoReplyState::Evaluating;
                status("thinking");
                let s = settings.clone();
                let done = tx.clone();
                let g = generation;
                tokio::spawn(async move {
                    let result =
                        provider::reply(&s, &prompt, &messages, forced, audio_reply, &c).await;
                    let _ = done
                        .send(Input::Ready {
                            generation: g,
                            result,
                        })
                        .await;
                });
            }
        }
        if let Some(c) = running {
            c.cancel();
            if let Some(id) = last_message {
                let _ = self.store.interrupt_message(&id);
            }
        }
    }
}

#[cfg(test)]
mod tests;
