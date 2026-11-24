use super::*;
use wasapi::{AudioClient, Direction, SampleType, StreamMode, WaveFormat};

pub fn start(
    frames: mpsc::Sender<AudioFrame>,
    error: AudioError,
    cancel: CancellationToken,
    armed: Arc<AtomicBool>,
) -> Result<()> {
    let (ready, rx) = std::sync::mpsc::sync_channel(1);
    let timeout = cancel.clone();
    std::thread::Builder::new()
        .name("system-audio-wasapi".into())
        .spawn(move || {
            let result = (|| -> Result<()> {
                wasapi::initialize_mta()
                    .ok()
                    .map_err(|_| "初始化 Windows 音频环境失败")?;
                let mut client =
                    AudioClient::new_application_loopback_client(std::process::id(), false)
                        .map_err(|_| "系统音频采集初始化失败，需要 Windows 11")?;
                let format = WaveFormat::new(32, 32, &SampleType::Float, 48000, 2, None);
                client
                    .initialize_client(
                        &format,
                        &Direction::Capture,
                        &StreamMode::EventsShared {
                            autoconvert: true,
                            buffer_duration_hns: 200_000,
                        },
                    )
                    .map_err(|_| "系统音频格式不受支持")?;
                let event = client
                    .set_get_eventhandle()
                    .map_err(|_| "创建系统音频事件失败")?;
                let capture = client
                    .get_audiocaptureclient()
                    .map_err(|_| "无法打开系统音频采集")?;
                client.start_stream().map_err(|_| "无法开始采集电脑音频")?;
                let _ = ready.send(Ok(()));
                let mut pipe = CapturePipe::new("system".into(), 48000, frames, armed)?;
                let mut bytes = VecDeque::new();
                let mut last_data = std::time::Instant::now();
                while !cancel.is_cancelled() {
                    // A process-loopback client excludes our process tree and needs no render endpoint.
                    capture
                        .read_from_device_to_deque(&mut bytes)
                        .map_err(|_| "电脑音频采集中断，请重新开始会话")?;
                    if !bytes.is_empty() {
                        let data: Vec<u8> = bytes.drain(..).collect();
                        let mono: Vec<f32> = data
                            .as_chunks::<8>()
                            .0
                            .iter()
                            .map(|f| {
                                (f32::from_le_bytes(f[..4].try_into().unwrap())
                                    + f32::from_le_bytes(f[4..].try_into().unwrap()))
                                    * 0.5
                            })
                            .collect();
                        pipe.push(&mono)?;
                        last_data = std::time::Instant::now();
                    } else if last_data.elapsed() > Duration::from_millis(200) {
                        pipe.push(&vec![0.0; 9600])?;
                        last_data = std::time::Instant::now();
                    }
                    let _ = event.wait_for_event(50);
                }
                client.stop_stream().map_err(|_| "停止系统音频采集失败")?;
                Ok(())
            })();
            if let Err(e) = result {
                let _ = ready.send(Err(e.clone()));
                if !cancel.is_cancelled() {
                    error("system".into(), e);
                }
                cancel.cancel();
            }
        })
        .map_err(|_| "创建系统音频线程失败")?;
    rx.recv_timeout(Duration::from_secs(20))
        .unwrap_or_else(|_| {
            timeout.cancel();
            Err("系统音频采集启动超时".into())
        })
}
