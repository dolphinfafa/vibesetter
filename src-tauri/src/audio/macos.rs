use super::*;
use screencapturekit::{prelude::*, stream::delegate_trait::ErrorHandler};

pub fn start(
    frames: mpsc::Sender<AudioFrame>,
    error: AudioError,
    cancel: CancellationToken,
    armed: Arc<AtomicBool>,
) -> Result<()> {
    let (ready, rx) = std::sync::mpsc::sync_channel(1);
    let timeout = cancel.clone();
    std::thread::Builder::new()
        .name("system-audio-sck".into())
        .spawn(move || {
            let result = (|| -> Result<()> {
                let content = SCShareableContent::get().map_err(|_| {
                    "无法读取系统音频来源，请在系统设置中允许屏幕与系统音频录制后重启应用"
                })?;
                let display = content
                    .displays()
                    .into_iter()
                    .next()
                    .ok_or("未发现显示器，无法初始化系统音频采集")?;
                let filter = SCContentFilter::create()
                    .with_display(&display)
                    .with_excluding_windows(&[])
                    .build();
                let configuration = SCStreamConfiguration::new()
                    .with_width(2)
                    .with_height(2)
                    .with_captures_audio(true)
                    .with_sample_rate(16000)
                    .with_channel_count(1)
                    .with_excludes_current_process_audio(true)
                    .with_shows_cursor(false);
                let report = error.clone();
                let c = cancel.clone();
                let delegate = ErrorHandler::new(move |_| {
                    if !c.is_cancelled() {
                        report(
                            "system".into(),
                            "系统音频采集中断，请检查录制权限后重新开始".into(),
                        );
                        c.cancel();
                    }
                });
                let mut stream = SCStream::new_with_delegate(&filter, &configuration, delegate);
                let pipe = Mutex::new(CapturePipe::new("system".into(), 16000, frames, armed)?);
                let report = error.clone();
                let c = cancel.clone();
                stream.add_output_handler(
                    move |sample: CMSampleBuffer, kind: SCStreamOutputType| {
                        if kind != SCStreamOutputType::Audio || c.is_cancelled() {
                            return;
                        }
                        if let Some(list) = sample.audio_buffer_list() {
                            if let Some(buffer) = list.get(0) {
                                let channels = buffer.number_channels.max(1) as usize;
                                let data: Vec<f32> = buffer
                                    .data()
                                    .chunks_exact(4 * channels)
                                    .map(|frame| {
                                        frame
                                            .as_chunks::<4>()
                                            .0
                                            .iter()
                                            .map(|b| f32::from_le_bytes(*b))
                                            .sum::<f32>()
                                            / channels as f32
                                    })
                                    .collect();
                                if let Err(e) =
                                    pipe.lock().unwrap_or_else(|p| p.into_inner()).push(&data)
                                {
                                    report("system".into(), e);
                                    c.cancel();
                                }
                            }
                        }
                    },
                    SCStreamOutputType::Audio,
                );
                stream
                    .start_capture()
                    .map_err(|_| "无法开始采集系统音频，请检查系统录制权限")?;
                let _ = ready.send(Ok(()));
                while !cancel.is_cancelled() {
                    std::thread::sleep(Duration::from_millis(25));
                }
                stream.stop_capture().map_err(|_| "停止系统音频失败")?;
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
        .map_err(|_| "创建 macOS 音频线程失败")?;
    rx.recv_timeout(Duration::from_secs(30))
        .unwrap_or_else(|_| {
            timeout.cancel();
            Err("系统音频授权超时，请在系统设置中检查权限后重启应用".into())
        })
}
