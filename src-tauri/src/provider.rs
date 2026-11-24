use crate::{
    model::{Message, Result},
    protocol::{self, Lines},
    settings::Settings,
};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{
    client::IntoClientRequest, http::HeaderValue, Message as WsMessage,
};
use tokio_util::sync::CancellationToken;

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(12))
        .timeout(Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "创建网络连接失败".into())
}

fn status_error(service: &str, status: u16) -> String {
    let hint = match status {
        401 | 403 => "请检查凭据与资源授权",
        429 => "请求受限，请检查配额后重试",
        500..=599 => "服务暂时不可用，请稍后重试",
        _ => "请检查服务地址、模型标识及资源配置",
    };
    format!("{service}返回 HTTP {status}，{hint}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reply {
    pub respond: bool,
    pub text: String,
}

pub async fn reply(
    settings: &Settings,
    prompt: &str,
    messages: &[Message],
    manual: bool,
    audio: bool,
    cancel: &CancellationToken,
) -> Result<Reply> {
    settings.validate_text()?;
    let instructions=format!("{prompt}\n\n[应用通信协议]\n你是实时助手。上述用户系统提示词定义你的职责、风格和主动程度。输入可能包含多个来源的对话转写；来源标签只表示设备，不是可靠的说话人身份。结合上下文判断是否应该回应，不要机械重复前文。{} {}\n只返回 JSON 对象：{{\"respond\":true或false,\"text\":\"实际回复内容\"}}。不需要回应时 respond=false、text为空。不要输出协议解释。",
        if manual {"用户本次主动要求回应，请针对最新上下文给出回复。"}else if audio {"本次在讲话停顿后由新语音触发；只有符合你的职责且值得补充时才开口回应。"}else{"本次由新的完整语音转写触发。你正在提供实时文字建议，可以在人们持续交谈时给出帮助，无需等待对方停顿或直接向你提问。根据上述职责判断是否有值得补充的新建议；已有建议足够、信息不足或不符合职责时保持倾听。"},
        if audio {"回复将被朗读，请输出适合口头表达的纯文本，避免 Markdown。"}else{"回复将作为文字展示，优先提供可直接使用的简洁内容。"});
    let mut wire = vec![json!({"role":"system","content":instructions})];
    let mut retained = Vec::new();
    let mut chars = 0;
    for message in messages.iter().rev().take(40) {
        let count = message.text.chars().count();
        if chars + count > 24000 {
            break;
        }
        chars += count;
        retained.push(json!({"role":message.role,"content":if message.role=="user"{format!("[来源：{}] {}",message.source,message.text)}else if message.interrupted{format!("{} [此回复已被打断，用户可能未听完]",message.text)}else{message.text.clone()}}));
    }
    wire.extend(retained.into_iter().rev());
    if messages.last().is_none_or(|m| m.role != "user") {
        // New transcripts can arrive before the previous suggestion is saved.
        // End the follow-up request with an explicit task rather than an assistant turn.
        wire.push(json!({"role":"user","content":if manual {
            "请根据当前上下文回应。"
        } else {
            "请结合最新对话和已有建议判断是否需要补充，只在有新的有效建议时回应。"
        }}));
    }
    let request=client()?.post(format!("{}/chat/completions",settings.text_base_url.trim_end_matches('/')))
        .bearer_auth(&settings.text_api_key).json(&json!({"model":settings.text_model,"messages":wire,"max_tokens":1000,"response_format":{"type":"json_object"}}));
    tokio::select! {
        _=cancel.cancelled()=>Err("已取消".into()),
        result=async {
            let response=request.send().await.map_err(|_| "文本模型连接失败，请检查网络")?;
            if !response.status().is_success(){return Err(status_error("文本模型",response.status().as_u16()));}
            let bytes=response.bytes().await.map_err(|_| "文本模型响应中断")?;
            if bytes.len()>1_000_000{return Err("文本模型响应超过限制".into());}
            let body:serde_json::Value=serde_json::from_slice(&bytes).map_err(|_| "文本模型返回格式错误")?;
            let content=body["choices"][0]["message"]["content"].as_str().ok_or("文本模型未返回内容，请检查模型兼容性")?;
            let content=content.trim().strip_prefix("```json").or_else(||content.trim().strip_prefix("```" )).unwrap_or(content).trim().trim_end_matches("```").trim();
            let mut reply:Reply=serde_json::from_str(content).map_err(|_| "模型未遵循 JSON 回复协议，请更换支持 JSON 输出的模型或调整提示词")?;
            reply.text=reply.text.trim().to_owned();
            if reply.text.chars().count()>12000{return Err("回复超过长度限制".into());}
            if reply.text.is_empty(){reply.respond=false;}
            Ok(reply)
        }=>result
    }
}

enum SpeechKind {
    Asr,
    Tts,
}

fn speech_headers(
    settings: &Settings,
    resource: &str,
    kind: SpeechKind,
) -> Result<reqwest::header::HeaderMap> {
    settings.validate_speech()?;
    let mut h = reqwest::header::HeaderMap::new();
    let mut add = |k: &'static str, v: &str| -> Result<()> {
        h.insert(
            k,
            HeaderValue::from_str(v).map_err(|_| "语音配置包含非法字符")?,
        );
        Ok(())
    };
    if !settings.speech_api_key.is_empty() {
        add("x-api-key", &settings.speech_api_key)?;
    } else {
        add(
            match kind {
                SpeechKind::Asr => "x-api-app-key",
                SpeechKind::Tts => "x-api-app-id",
            },
            &settings.speech_app_id,
        )?;
        add("x-api-access-key", &settings.speech_access_token)?;
    }
    add("x-api-resource-id", resource)?;
    add(
        match kind {
            SpeechKind::Asr => "x-api-connect-id",
            SpeechKind::Tts => "x-api-request-id",
        },
        &uuid::Uuid::new_v4().to_string(),
    )?;
    Ok(h)
}

#[derive(Debug)]
pub struct Transcript {
    pub text: String,
    pub is_final: bool,
}

pub async fn recognize(
    settings: Settings,
    mut audio: mpsc::Receiver<Vec<i16>>,
    on_text: Arc<dyn Fn(Transcript) + Send + Sync>,
    cancel: CancellationToken,
) -> Result<()> {
    let mut request = settings
        .asr_url
        .clone()
        .into_client_request()
        .map_err(|_| "语音识别地址无效")?;
    for (name, value) in
        speech_headers(&settings, &settings.asr_resource_id, SpeechKind::Asr)?.iter()
    {
        request.headers_mut().insert(name, value.clone());
    }
    let (socket, _) = tokio::select! {
        _=cancel.cancelled()=>return Ok(()),
        result=tokio::time::timeout(Duration::from_secs(15),tokio_tungstenite::connect_async(request))=>result.map_err(|_|"语音识别连接超时")?.map_err(|_|"语音识别连接失败，请检查网络、凭据和资源授权")?
    };
    let (mut sink, mut stream) = socket.split();
    let initial = json!({"user":{"uid":"vibesetter"},"audio":{"format":"pcm","codec":"raw","rate":16000,"bits":16,"channel":1},"request":{"model_name":"bigmodel","enable_itn":true,"enable_punc":true,"show_utterances":true,"end_window_size":800}});
    sink.send(WsMessage::Binary(
        protocol::encode(
            1,
            0,
            1,
            &serde_json::to_vec(&initial).map_err(|_| "识别请求编码失败")?,
        )?
        .into(),
    ))
    .await
    .map_err(|_| "识别请求发送失败")?;
    let send_cancel = cancel.clone();
    let send = async move {
        let mut pacing = tokio::time::interval(Duration::from_millis(200));
        pacing.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _=send_cancel.cancelled()=>break,
                _=pacing.tick()=>{
                    let samples=match audio.try_recv(){Ok(samples)=>samples,Err(mpsc::error::TryRecvError::Empty)=>vec![0;3200],Err(mpsc::error::TryRecvError::Disconnected)=>{let _=sink.send(WsMessage::Binary(protocol::encode(2,2,0,&[])?.into())).await;break;}};
                    let bytes:Vec<u8>=samples.iter().flat_map(|v|v.to_le_bytes()).collect();
                    sink.send(WsMessage::Binary(protocol::encode(2,0,0,&bytes)?.into())).await.map_err(|_|"音频上传中断")?;
                }
            }
        }
        Ok::<(), String>(())
    };
    let receive = async move {
        let mut last_end = -1_i64;
        while let Some(packet) = stream.next().await {
            match packet.map_err(|_| "语音识别连接中断，请暂停并重新开始")? {
                WsMessage::Binary(bytes) => {
                    let value = protocol::decode(&bytes)?;
                    let result = &value["result"];
                    let mut partial = String::new();
                    if let Some(utterances) = result["utterances"].as_array() {
                        for u in utterances {
                            let text = u["text"].as_str().unwrap_or("");
                            let end = u["end_time"].as_i64().unwrap_or(-1);
                            if u["definite"].as_bool() == Some(true) {
                                if end > last_end && !text.trim().is_empty() {
                                    last_end = end;
                                    on_text(Transcript {
                                        text: text.into(),
                                        is_final: true,
                                    });
                                }
                            } else {
                                partial.push_str(text);
                            }
                        }
                    } else if let Some(text) = result["text"].as_str() {
                        partial = text.into();
                    }
                    on_text(Transcript {
                        text: partial,
                        is_final: false,
                    });
                }
                WsMessage::Close(_) => return Err("语音识别会话已关闭，请暂停并重新开始".into()),
                _ => {}
            }
        }
        Err::<(), String>("语音识别连接已结束".into())
    };
    tokio::select! {_=cancel.cancelled()=>Ok(()),r=send=>r,r=receive=>r}
}

pub async fn synthesize(
    settings: &Settings,
    text: &str,
    samples: mpsc::Sender<Vec<f32>>,
    cancel: &CancellationToken,
) -> Result<()> {
    let response = tokio::select! {
        _=cancel.cancelled()=>return Ok(()),
        response=client()?.post(&settings.tts_url).timeout(Duration::from_secs(300)).headers(speech_headers(settings,&settings.tts_resource_id,SpeechKind::Tts)?).json(&json!({"user":{"uid":"vibesetter"},"namespace":"BidirectionalTTS","req_params":{"text":text,"speaker":settings.tts_voice,"audio_params":{"format":"pcm","sample_rate":24000}}})).send()=>response.map_err(|_|"语音合成连接失败")?
    };
    if !response.status().is_success() {
        return Err(status_error("语音合成", response.status().as_u16()));
    }
    let mut stream = response.bytes_stream();
    let mut lines = Lines::default();
    let mut completed = false;
    loop {
        let next = tokio::select! {_=cancel.cancelled()=>return Ok(()),n=stream.next()=>n};
        let Some(next) = next else {
            break;
        };
        for line in lines.push(&next.map_err(|_| "语音合成流中断")?)? {
            if parse_tts_line(&line, &samples, cancel).await? {
                completed = true;
            }
        }
    }
    if let Some(line) = lines.finish()? {
        if parse_tts_line(&line, &samples, cancel).await? {
            completed = true;
        }
    }
    if !completed && !cancel.is_cancelled() {
        return Err("语音合成未正常结束，请检查网络".into());
    }
    Ok(())
}

async fn parse_tts_line(
    line: &str,
    samples: &mpsc::Sender<Vec<f32>>,
    cancel: &CancellationToken,
) -> Result<bool> {
    let line = line.trim();
    if line.is_empty() || line.starts_with("event:") || line.starts_with(':') {
        return Ok(false);
    }
    let line = line.strip_prefix("data:").unwrap_or(line).trim();
    let value: serde_json::Value =
        serde_json::from_str(line).map_err(|_| "语音合成返回格式错误")?;
    let code = value["code"].as_i64().ok_or("语音合成返回缺少状态码")?;
    if code == 20000000 {
        return Ok(true);
    }
    if code != 0 {
        return Err(format!("语音合成服务错误（{code}），请检查音色授权及配额"));
    }
    if let Some(data) = value["data"].as_str() {
        let pcm = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| "合成音频解码失败")?;
        if pcm.len() % 2 != 0 {
            return Err("合成 PCM 音频长度错误".into());
        }
        for chunk in pcm.chunks(4800) {
            let decoded = chunk
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
                .collect();
            tokio::select! {_=cancel.cancelled()=>return Ok(false),r=samples.send(decoded)=>r.map_err(|_|"播放已结束")?};
        }
    }
    Ok(false)
}

pub async fn test_connection(settings: &Settings, service: &str) -> Result<String> {
    match service {
        "text" => {
            let r = reply(
                settings,
                "请简短回应连接成功。",
                &[],
                true,
                false,
                &CancellationToken::new(),
            )
            .await?;
            if !r.respond {
                return Err("连接成功，但模型未返回可用回复".into());
            }
            Ok("豆包文本模型连接成功".into())
        }
        "asr" => {
            let (tx, rx) = mpsc::channel(4);
            let cancel = CancellationToken::new();
            let child = cancel.clone();
            let s = settings.clone();
            let (ok, mut receive) = mpsc::channel(1);
            let task = tokio::spawn(async move {
                recognize(
                    s,
                    rx,
                    Arc::new(move |_| {
                        let _ = ok.try_send(());
                    }),
                    child,
                )
                .await
            });
            let result=async {for _ in 0..3 {tx.send(vec![0;3200]).await.map_err(|_|"语音识别连接失败")?;tokio::time::sleep(Duration::from_millis(200)).await;}tokio::select!{
                _=receive.recv()=>Ok("语音识别连接成功".into()),
                _=tokio::time::sleep(Duration::from_secs(10))=>Err("语音识别检测超时，请检查授权与资源配置".into())
            }}.await;
            cancel.cancel();
            let task_result = task.await.map_err(|_| "识别检测任务异常")?;
            task_result?;
            result
        }
        "tts" => {
            let (tx, mut rx) = mpsc::channel(16);
            let s = settings.clone();
            let task = tokio::spawn(async move {
                synthesize(&s, "连接成功", tx, &CancellationToken::new()).await
            });
            let mut count = 0;
            while let Some(samples) = rx.recv().await {
                count += samples.len();
            }
            task.await.map_err(|_| "合成检测任务异常")??;
            if count == 0 {
                return Err("语音服务未返回音频".into());
            }
            Ok("语音合成连接成功（检测音频未播放）".into())
        }
        _ => Err("未知服务".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[test]
    fn speech_auth_matches_legacy_asr_and_http_tts_contracts() {
        let legacy = Settings {
            speech_app_id: "app".into(),
            speech_access_token: "token".into(),
            ..Default::default()
        };
        let asr = speech_headers(&legacy, "asr", SpeechKind::Asr).unwrap();
        assert_eq!(asr["x-api-app-key"], "app");
        assert!(asr.contains_key("x-api-connect-id"));
        let tts = speech_headers(&legacy, "tts", SpeechKind::Tts).unwrap();
        assert_eq!(tts["x-api-app-id"], "app");
        assert!(tts.contains_key("x-api-request-id"));
        assert!(!tts.contains_key("x-api-app-key"));
        let modern = Settings {
            speech_api_key: "new-key".into(),
            ..legacy
        };
        let h = speech_headers(&modern, "tts", SpeechKind::Tts).unwrap();
        assert_eq!(h["x-api-key"], "new-key");
        assert!(!h.contains_key("x-api-access-key"));
    }
    #[tokio::test]
    async fn llm_http_contract_and_cancellation() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0; 16000];
            let n = socket.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]);
            assert!(request.starts_with("POST /chat/completions"));
            assert!(request.contains("Bearer test-key"));
            let body=json!({"choices":[{"message":{"content":"{\"respond\":true,\"text\":\"你好，搭档\"}"}}]}).to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
        });
        let s = Settings {
            text_base_url: format!("http://{address}"),
            text_api_key: "test-key".into(),
            text_model: "model".into(),
            ..Settings::default()
        };
        let r = reply(&s, "中文助手", &[], true, false, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(r.text, "你好，搭档");
        server.await.unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(reply(&s, "", &[], true, false, &cancel).await.is_err());
    }
    #[tokio::test]
    async fn tts_decodes_pcm_and_reports_server_errors() {
        let (tx, mut rx) = mpsc::channel(4);
        let c = CancellationToken::new();
        let b = base64::engine::general_purpose::STANDARD.encode([0, 0, 0, 64]);
        assert!(
            !parse_tts_line(&format!("data: {}", json!({"code":0,"data":b})), &tx, &c)
                .await
                .unwrap()
        );
        assert_eq!(rx.recv().await.unwrap(), vec![0.0, 0.5]);
        assert!(parse_tts_line("{\"code\":45000000}", &tx, &c)
            .await
            .is_err());
        assert!(parse_tts_line("{\"code\":20000000}", &tx, &c)
            .await
            .unwrap());
    }
    #[tokio::test]
    #[allow(clippy::result_large_err)] // Tungstenite requires this callback error type.
    async fn asr_websocket_auth_pcm_and_duplicate_final_utterances() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_hdr_async(
                socket,
                |req: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                    assert_eq!(req.headers()["x-api-key"], "speech-test");
                    assert!(req.headers().contains_key("x-api-resource-id"));
                    Ok(response)
                },
            )
            .await
            .unwrap();
            let init = ws.next().await.unwrap().unwrap().into_data();
            assert_eq!(protocol::decode(&init).unwrap()["audio"]["rate"], 16000);
            let pcm = ws.next().await.unwrap().unwrap().into_data();
            assert_eq!(pcm[1] >> 4, 2);
            let data = json!({"result":{"utterances":[{"text":"已确认的中文","end_time":1000,"definite":true},{"text":"正在识别","end_time":1200,"definite":false}]}});
            let packet = protocol::encode(9, 0, 1, &serde_json::to_vec(&data).unwrap()).unwrap();
            ws.send(WsMessage::Binary(packet.clone().into()))
                .await
                .unwrap();
            ws.send(WsMessage::Binary(packet.into())).await.unwrap();
            let _ = tokio::time::timeout(Duration::from_secs(3), ws.next()).await;
        });
        let settings = Settings {
            asr_url: format!("ws://{address}"),
            speech_api_key: "speech-test".into(),
            ..Default::default()
        };
        let (audio, rx) = mpsc::channel(2);
        let (events, mut event_rx) = mpsc::unbounded_channel();
        let cancel = CancellationToken::new();
        let c = cancel.clone();
        let client = tokio::spawn(async move {
            recognize(
                settings,
                rx,
                Arc::new(move |t| {
                    let _ = events.send(t);
                }),
                c,
            )
            .await
        });
        audio.send(vec![123; 3200]).await.unwrap();
        let mut final_count = 0;
        for _ in 0..3 {
            let t = tokio::time::timeout(Duration::from_secs(3), event_rx.recv())
                .await
                .unwrap()
                .unwrap();
            if t.is_final {
                final_count += 1;
                assert_eq!(t.text, "已确认的中文");
            }
        }
        assert_eq!(final_count, 1);
        cancel.cancel();
        assert!(client.await.unwrap().is_ok());
        server.await.unwrap();
    }
}
