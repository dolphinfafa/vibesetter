use crate::{
    engine::{AudioBackend, AudioError, AudioFrame, PlaybackDone},
    model::*,
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rubato::{FftFixedIn, Resampler};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;
pub struct NativeAudio;

impl AudioBackend for NativeAudio {
    fn devices(&self) -> Result<Vec<AudioDevice>> {
        let host = cpal::default_host();
        let mut result = Vec::new();
        for (kind, default) in [
            ("input", host.default_input_device()),
            ("output", host.default_output_device()),
        ] {
            let default_id = default.and_then(|d| d.id().ok()).map(|id| id.to_string());
            let devices = if kind == "input" {
                host.input_devices()
            } else {
                host.output_devices()
            }
            .map_err(|_| "读取音频设备失败")?;
            for device in devices {
                if let (Ok(id), Ok(description)) = (device.id(), device.description()) {
                    let raw = id.to_string();
                    result.push(AudioDevice {
                        id: format!("{kind}:{raw}"),
                        name: description.name().to_owned(),
                        kind: kind.into(),
                        is_default: default_id.as_ref() == Some(&raw),
                    });
                }
            }
        }
        Ok(result)
    }
    fn start_inputs(
        &self,
        config: &SessionConfig,
        frames: mpsc::Sender<AudioFrame>,
        error: AudioError,
        cancel: CancellationToken,
    ) -> Result<()> {
        let armed = Arc::new(AtomicBool::new(false));
        for id in &config.microphone_ids {
            start_microphone(
                id.clone(),
                frames.clone(),
                error.clone(),
                cancel.child_token(),
                armed.clone(),
            )?;
        }
        if config.system_audio {
            #[cfg(target_os = "windows")]
            windows::start(frames, error, cancel.child_token(), armed.clone())?;
            #[cfg(target_os = "macos")]
            macos::start(frames, error, cancel.child_token(), armed.clone())?;
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            return Err("系统音频采集仅支持 Windows 11 和 macOS 14.2+".into());
        }
        armed.store(true, Ordering::Release);
        Ok(())
    }
    fn start_output(
        &self,
        ids: &[String],
        samples: mpsc::Receiver<Vec<f32>>,
        error: AudioError,
        cancel: CancellationToken,
    ) -> Result<PlaybackDone> {
        let (ready, rx) = std::sync::mpsc::sync_channel(1);
        let (done, finished) = oneshot::channel();
        let ids = ids.to_vec();
        let timeout_cancel = cancel.clone();
        std::thread::Builder::new()
            .name("audio-output".into())
            .spawn(move || {
                let mut worker = match OutputWorker::new(&ids, error, &cancel) {
                    Ok(w) => {
                        let _ = ready.send(Ok(()));
                        w
                    }
                    Err(e) => {
                        let _ = ready.send(Err(e));
                        return;
                    }
                };
                let result = worker.run(samples, &cancel);
                let _ = done.send(result);
            })
            .map_err(|_| "无法启动播放线程")?;
        match rx.recv_timeout(Duration::from_secs(15)) {
            Ok(r) => {
                r?;
                Ok(finished)
            }
            Err(_) => {
                timeout_cancel.cancel();
                Err("启动音频输出超时".into())
            }
        }
    }
}
fn find_device(id: &str, kind: &str) -> Result<cpal::Device> {
    let raw = id
        .strip_prefix(&format!("{kind}:"))
        .ok_or("设备标识类型错误")?;
    let parsed = raw
        .parse::<cpal::DeviceId>()
        .map_err(|_| "设备标识无效，请重新选择")?;
    cpal::default_host()
        .device_by_id(&parsed)
        .ok_or("设备已断开，请重新选择".into())
}

pub(super) struct Converter {
    resampler: Option<FftFixedIn<f32>>,
    pending: VecDeque<f32>,
}
impl Converter {
    pub(super) fn new(from: u32, to: u32) -> Result<Self> {
        let resampler = if from == to {
            None
        } else {
            Some(
                FftFixedIn::new(from as usize, to as usize, 1024, 2, 1)
                    .map_err(|_| "不支持此设备的采样率")?,
            )
        };
        Ok(Self {
            resampler,
            pending: VecDeque::new(),
        })
    }
    fn push(&mut self, samples: &[f32]) -> Result<Vec<f32>> {
        let Some(resampler) = self.resampler.as_mut() else {
            return Ok(samples.to_vec());
        };
        self.pending.extend(samples.iter().copied());
        let mut out = Vec::new();
        while self.pending.len() >= resampler.input_frames_next() {
            let n = resampler.input_frames_next();
            let input: Vec<f32> = self.pending.drain(..n).collect();
            let chunk = resampler
                .process(&[input], None)
                .map_err(|_| "音频重采样失败")?;
            out.extend_from_slice(&chunk[0]);
        }
        Ok(out)
    }
    fn finish(&mut self) -> Result<Vec<f32>> {
        if self.pending.is_empty() {
            return Ok(vec![]);
        }
        let n = self
            .resampler
            .as_ref()
            .map(|r| r.input_frames_next())
            .unwrap_or(0);
        self.push(&vec![0.0; n.saturating_sub(self.pending.len())])
    }
}

pub(super) struct CapturePipe {
    source: String,
    converter: Converter,
    pending: VecDeque<i16>,
    frames: mpsc::Sender<AudioFrame>,
    armed: Arc<AtomicBool>,
}
impl CapturePipe {
    pub(super) fn new(
        source: String,
        rate: u32,
        frames: mpsc::Sender<AudioFrame>,
        armed: Arc<AtomicBool>,
    ) -> Result<Self> {
        Ok(Self {
            source,
            converter: Converter::new(rate, 16000)?,
            pending: VecDeque::new(),
            frames,
            armed,
        })
    }
    pub(super) fn push(&mut self, mono: &[f32]) -> Result<()> {
        // Wait until every selected source is authorized and ready before collecting input.
        if !self.armed.load(Ordering::Acquire) {
            return Ok(());
        }
        let samples = self.converter.push(mono)?;
        self.pending.extend(
            samples
                .iter()
                .map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16),
        );
        while self.pending.len() >= 3200 {
            let samples = self.pending.drain(..3200).collect();
            self.frames
                .try_send(AudioFrame {
                    source: self.source.clone(),
                    samples,
                })
                .map_err(|_| "音频缓冲已满，会话可能已结束，请暂停后重试")?;
        }
        Ok(())
    }
}

fn start_microphone(
    id: String,
    frames: mpsc::Sender<AudioFrame>,
    error: AudioError,
    cancel: CancellationToken,
    armed: Arc<AtomicBool>,
) -> Result<()> {
    let (ready, rx) = std::sync::mpsc::sync_channel(1);
    let timeout_cancel = cancel.clone();
    std::thread::Builder::new().name("microphone".into()).spawn(move||{
        let result=(||->Result<cpal::Stream>{
            let device=find_device(&id,"input")?;
            let supported=device.default_input_config().map_err(|_|"无法访问麦克风，请检查系统权限与设备状态")?;
            let config=supported.config();
            let (sender,receiver)=std::sync::mpsc::sync_channel::<Vec<f32>>(32);
            let failure=error.clone();let source=id.clone();let c=cancel.clone();
            let on_error=move|_:cpal::Error|{failure(source.clone(),"麦克风已断开或采集失败，请重新连接并开始会话".into());c.cancel();};
            let channels=config.channels as usize;
            macro_rules! build {($t:ty)=>{{
                let c=cancel.clone();let overflow=error.clone();let source=id.clone();
                device.build_input_stream::<$t,_,_>(config,move|data,_|{
                    if c.is_cancelled(){return;}
                    let mono=data.chunks_exact(channels).map(|frame|frame.iter().map(|v|<f32 as cpal::FromSample<$t>>::from_sample_(*v)).sum::<f32>()/channels as f32).collect();
                    if sender.try_send(mono).is_err(){overflow(source.clone(),"麦克风处理缓冲已满，请暂停后重新开始".into());c.cancel();}
                },on_error,None)
            }}}
            let stream=match supported.sample_format(){cpal::SampleFormat::F32=>build!(f32),cpal::SampleFormat::I16=>build!(i16),cpal::SampleFormat::U16=>build!(u16),cpal::SampleFormat::I32=>build!(i32),cpal::SampleFormat::F64=>build!(f64),_=>return Err("麦克风的采样格式暂不支持".into())}.map_err(|_|"麦克风启动失败，请检查系统授权")?;
            let mut pipe=CapturePipe::new(id.clone(),supported.sample_rate(),frames,armed)?;
            let c=cancel.clone();let e=error.clone();let source=id.clone();
            std::thread::spawn(move||{while !c.is_cancelled(){match receiver.recv_timeout(Duration::from_millis(100)){Ok(data)=>if let Err(err)=pipe.push(&data){e(source.clone(),err);c.cancel();},Err(std::sync::mpsc::RecvTimeoutError::Timeout)=>{},Err(_)=>break}}});
            stream.play().map_err(|_|"麦克风无法开始采集")?;Ok(stream)
        })();
        match result{Ok(_stream)=>{let _=ready.send(Ok(()));while !cancel.is_cancelled(){std::thread::sleep(Duration::from_millis(25));}},Err(e)=>{cancel.cancel();let _=ready.send(Err(e));}}
    }).map_err(|_|"创建麦克风线程失败")?;
    rx.recv_timeout(Duration::from_secs(20))
        .unwrap_or_else(|_| {
            timeout_cancel.cancel();
            Err("麦克风授权或启动超时，请在系统设置中检查权限".into())
        })
}

struct Sink {
    _stream: cpal::Stream,
    queue: Arc<Mutex<VecDeque<f32>>>,
    converter: Converter,
    alive: Arc<AtomicBool>,
    rate: u32,
}
struct OutputWorker {
    sinks: Vec<Sink>,
}

impl OutputWorker {
    fn new(ids: &[String], error: AudioError, cancel: &CancellationToken) -> Result<Self> {
        let mut sinks = Vec::new();
        for id in ids {
            let device = find_device(id, "output")?;
            let supported = device
                .default_output_config()
                .map_err(|_| "无法访问音频输出设备")?;
            let config = supported.config();
            let channels = config.channels as usize;
            let queue = Arc::new(Mutex::new(VecDeque::<f32>::new()));
            let alive = Arc::new(AtomicBool::new(true));
            let failed = alive.clone();
            let report = error.clone();
            let source = id.clone();
            let on_error = move |_: cpal::Error| {
                failed.store(false, Ordering::Relaxed);
                report(
                    source.clone(),
                    "音频输出设备断开或播放失败，其他输出将继续".into(),
                );
            };
            macro_rules! build {
                ($t:ty) => {{
                    let q = queue.clone();
                    let c = cancel.clone();
                    device.build_output_stream::<$t, _, _>(
                        config,
                        move |data, _| {
                            let mut queue = q.lock().unwrap_or_else(|p| p.into_inner());
                            for frame in data.chunks_mut(channels) {
                                let value = if c.is_cancelled() {
                                    0.0
                                } else {
                                    queue.pop_front().unwrap_or(0.0)
                                };
                                for sample in frame {
                                    *sample = <$t as cpal::FromSample<f32>>::from_sample_(value);
                                }
                            }
                        },
                        on_error,
                        None,
                    )
                }};
            }
            let stream = match supported.sample_format() {
                cpal::SampleFormat::F32 => build!(f32),
                cpal::SampleFormat::I16 => build!(i16),
                cpal::SampleFormat::U16 => build!(u16),
                cpal::SampleFormat::I32 => build!(i32),
                cpal::SampleFormat::F64 => build!(f64),
                _ => return Err("输出设备的采样格式暂不支持".into()),
            }
            .map_err(|_| "无法打开音频输出设备")?;
            stream.play().map_err(|_| "音频设备无法开始播放")?;
            sinks.push(Sink {
                _stream: stream,
                queue,
                converter: Converter::new(24000, supported.sample_rate())?,
                alive,
                rate: supported.sample_rate(),
            });
        }
        if sinks.is_empty() {
            return Err("未选择音频输出".into());
        }
        Ok(Self { sinks })
    }
    fn run(
        &mut self,
        mut samples: mpsc::Receiver<Vec<f32>>,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let mut closed = false;
        loop {
            if cancel.is_cancelled() {
                break;
            }
            if !self.sinks.iter().any(|s| s.alive.load(Ordering::Relaxed)) {
                return Err("所有音频输出设备均已断开".into());
            }
            let buffered = self
                .sinks
                .iter()
                .filter(|s| s.alive.load(Ordering::Relaxed))
                .any(|s| {
                    s.queue.lock().unwrap_or_else(|p| p.into_inner()).len() > s.rate as usize * 2
                });
            if !closed && !buffered {
                match samples.try_recv() {
                    Ok(chunk) => {
                        for sink in self
                            .sinks
                            .iter_mut()
                            .filter(|s| s.alive.load(Ordering::Relaxed))
                        {
                            let out = sink.converter.push(&chunk)?;
                            sink.queue
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .extend(out);
                        }
                    }
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        closed = true;
                        for sink in &mut self.sinks {
                            let out = sink.converter.finish()?;
                            sink.queue
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .extend(out);
                        }
                    }
                    Err(mpsc::error::TryRecvError::Empty) => {}
                }
            }
            if closed
                && self
                    .sinks
                    .iter()
                    .filter(|s| s.alive.load(Ordering::Relaxed))
                    .all(|s| s.queue.lock().unwrap_or_else(|p| p.into_inner()).is_empty())
            {
                std::thread::sleep(Duration::from_millis(80));
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn permission_setup_discards_audio_until_all_sources_are_ready() {
        let (tx, mut rx) = mpsc::channel(2);
        let armed = Arc::new(AtomicBool::new(false));
        let mut pipe = CapturePipe::new("mic".into(), 16000, tx, armed.clone()).unwrap();
        for _ in 0..100 {
            pipe.push(&vec![0.5; 3200]).unwrap();
        }
        assert!(rx.try_recv().is_err());
        armed.store(true, Ordering::Release);
        pipe.push(&vec![0.5; 3200]).unwrap();
        let frame = rx.try_recv().unwrap();
        assert_eq!(frame.source, "mic");
        assert_eq!(frame.samples.len(), 3200);
        assert_eq!(frame.samples[0], 16383);
    }
    #[test]
    fn resampling_handles_fragmented_input_and_flushes_partial_block() {
        let mut c = Converter::new(48000, 16000).unwrap();
        let mut out = Vec::new();
        for _ in 0..100 {
            out.extend(c.push(&vec![0.1; 480]).unwrap());
        }
        out.extend(c.finish().unwrap());
        assert!((out.len() as i64 - 16000).abs() < 400);
        assert!(out.iter().all(|f| f.is_finite()));
        assert!(out
            .iter()
            .skip(1000)
            .take(1000)
            .all(|v| (v - 0.1).abs() < 0.005));
    }
}
