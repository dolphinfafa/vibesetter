use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub type Result<T> = std::result::Result<T, String>;

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assistant {
    pub id: String,
    pub name: String,
    pub prompt: String,
    pub avatar: Option<String>,
    pub color: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Assistant {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() || self.name.chars().count() > 80 {
            return Err("助手名称应为 1–80 个字符".into());
        }
        if self.prompt.trim().is_empty() || self.prompt.chars().count() > 32000 {
            return Err("系统提示词应为 1–32000 个字符".into());
        }
        if let Some(avatar) = &self.avatar {
            if avatar.len() > 2_000_000
                || ![
                    "data:image/png;base64,",
                    "data:image/jpeg;base64,",
                    "data:image/webp;base64,",
                ]
                .iter()
                .any(|p| avatar.starts_with(p))
            {
                return Err("头像必须是小于 2 MB 的 PNG、JPEG 或 WebP 图片".into());
            }
        }
        if !["violet", "blue", "teal", "rose", "amber"].contains(&self.color.as_str()) {
            return Err("无效的头像颜色".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionConfig {
    pub microphone_ids: Vec<String>,
    pub system_audio: bool,
    pub text_input: bool,
    pub text_output: bool,
    pub output_ids: Vec<String>,
}

impl SessionConfig {
    pub fn validate(&self) -> Result<()> {
        if self.microphone_ids.is_empty() && !self.system_audio && !self.text_input {
            return Err("请至少选择一种输入".into());
        }
        if !self.text_output && self.output_ids.is_empty() {
            return Err("请至少选择一种输出".into());
        }
        if self.microphone_ids.len() > 8 || self.output_ids.len() > 8 {
            return Err("每类最多同时选择 8 个音频设备".into());
        }
        for ids in [&self.microphone_ids, &self.output_ids] {
            let unique: std::collections::HashSet<_> = ids.iter().collect();
            if unique.len() != ids.len() {
                return Err("不能重复选择同一设备".into());
            }
        }
        Ok(())
    }
    pub fn has_audio_input(&self) -> bool {
        self.system_audio || !self.microphone_ids.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub assistant_id: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub source: String,
    pub text: String,
    pub created_at: i64,
    pub interrupted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoReplyState {
    Listening,
    WaitingForPause,
    WaitingForInterval,
    Evaluating,
    Skipped,
    Interrupted,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    Status {
        session_id: String,
        status: String,
    },
    AutoReply {
        session_id: String,
        state: AutoReplyState,
    },
    Transcript {
        session_id: String,
        source: String,
        text: String,
        is_final: bool,
    },
    Message {
        message: Message,
        visible: bool,
    },
    Level {
        session_id: String,
        source: String,
        value: f32,
    },
    Interrupted {
        session_id: String,
        message_id: String,
    },
    Error {
        session_id: String,
        source: String,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routing_requires_real_input_and_output_and_no_duplicates() {
        let mut c = SessionConfig::default();
        assert!(c.validate().is_err());
        c.text_input = true;
        assert!(c.validate().is_err());
        c.text_output = true;
        assert!(c.validate().is_ok());
        c.microphone_ids = vec!["one".into(), "one".into()];
        assert!(c.validate().is_err());
    }
}
