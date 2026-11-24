//! Volcengine streaming ASR binary frames. No raw audio is persisted.
use crate::model::Result;
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use std::io::{Read, Write};

pub fn encode(message_type: u8, flags: u8, serialization: u8, payload: &[u8]) -> Result<Vec<u8>> {
    let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
    gz.write_all(payload).map_err(|_| "音频编码失败")?;
    let payload = gz.finish().map_err(|_| "音频编码失败")?;
    let mut frame = vec![
        0x11,
        (message_type << 4) | flags,
        (serialization << 4) | 1,
        0,
    ];
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend(payload);
    Ok(frame)
}

pub fn decode(frame: &[u8]) -> Result<serde_json::Value> {
    if frame.len() < 8 || frame[0] >> 4 != 1 {
        return Err("语音服务返回了不支持的数据帧".into());
    }
    let header_len = (frame[0] & 15) as usize * 4;
    if header_len < 4 || header_len > frame.len() {
        return Err("语音数据帧头损坏".into());
    }
    let kind = frame[1] >> 4;
    let flags = frame[1] & 15;
    let mut offset = header_len;
    let mut error_code = None;
    if kind == 15 {
        let b = frame.get(offset..offset + 4).ok_or("语音错误数据不完整")?;
        error_code = Some(u32::from_be_bytes(b.try_into().unwrap()));
        offset += 4;
    } else if flags & 1 == 1 {
        offset += 4;
    }
    let len_bytes = frame.get(offset..offset + 4).ok_or("语音数据长度缺失")?;
    let size = u32::from_be_bytes(len_bytes.try_into().unwrap()) as usize;
    offset += 4;
    if size > 4_000_000 {
        return Err("语音服务数据超过限制".into());
    }
    let bytes = frame.get(offset..offset + size).ok_or("语音数据被截断")?;
    let mut data = Vec::new();
    match frame[2] & 15 {
        0 => data.extend_from_slice(bytes),
        1 => {
            GzDecoder::new(bytes)
                .take(4_000_001)
                .read_to_end(&mut data)
                .map_err(|_| "语音数据解压失败")?;
        }
        _ => return Err("不支持的语音压缩格式".into()),
    }
    if data.len() > 4_000_000 {
        return Err("解压后语音数据超过限制".into());
    }
    if let Some(code) = error_code {
        return Err(format!(
            "语音识别服务错误（{code}），请检查凭据、资源授权和配额"
        ));
    }
    serde_json::from_slice(&data).map_err(|_| "语音识别返回的数据格式错误".into())
}

/// Newline-delimited JSON / SSE accumulator with a hard memory bound.
#[derive(Default)]
pub struct Lines {
    buffer: Vec<u8>,
}
impl Lines {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>> {
        if self.buffer.len() + bytes.len() > 4_000_000 {
            return Err("模型流数据超过限制".into());
        }
        self.buffer.extend_from_slice(bytes);
        let mut lines = Vec::new();
        while let Some(end) = self.buffer.iter().position(|b| *b == b'\n') {
            let mut raw: Vec<u8> = self.buffer.drain(..=end).collect();
            raw.pop();
            if raw.last() == Some(&b'\r') {
                raw.pop();
            }
            lines.push(String::from_utf8(raw).map_err(|_| "模型返回了无效的 UTF-8")?);
        }
        Ok(lines)
    }
    pub fn finish(&mut self) -> Result<Option<String>> {
        if self.buffer.is_empty() {
            return Ok(None);
        }
        String::from_utf8(std::mem::take(&mut self.buffer))
            .map(Some)
            .map_err(|_| "模型返回了无效的 UTF-8".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_compression_sequence_and_truncation() {
        let j = br#"{"result":{"text":"hello"}}"#;
        let frame = encode(9, 0, 1, j).unwrap();
        assert_eq!(decode(&frame).unwrap()["result"]["text"], "hello");
        let mut seq = frame.clone();
        seq[1] |= 1;
        seq.splice(4..4, 7_i32.to_be_bytes());
        assert!(decode(&seq).is_ok());
        for n in 0..frame.len() {
            assert!(decode(&frame[..n]).is_err());
        }
    }
    #[test]
    fn utf8_split_across_network_chunks() {
        let mut l = Lines::default();
        let b = "data: 中文\n\n".as_bytes();
        assert!(l.push(&b[..8]).unwrap().is_empty());
        assert_eq!(l.push(&b[8..]).unwrap(), ["data: 中文", ""]);
    }
}
