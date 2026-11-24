use crate::model::Result;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Write, path::Path};

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct SettingsPatch {
    pub text_api_key: Option<String>,
    pub text_model: Option<String>,
    pub text_base_url: Option<String>,
    pub speech_app_id: Option<String>,
    pub speech_access_token: Option<String>,
    pub speech_api_key: Option<String>,
    pub asr_resource_id: Option<String>,
    pub asr_url: Option<String>,
    pub tts_resource_id: Option<String>,
    pub tts_voice: Option<String>,
    pub tts_url: Option<String>,
}

#[derive(Clone)]
pub struct Settings {
    pub text_api_key: String,
    pub text_model: String,
    pub text_base_url: String,
    pub speech_app_id: String,
    pub speech_access_token: String,
    pub speech_api_key: String,
    pub asr_resource_id: String,
    pub asr_url: String,
    pub tts_resource_id: String,
    pub tts_voice: String,
    pub tts_url: String,
}

#[derive(Clone, Serialize)]
pub struct PublicSettings {
    pub has_text_api_key: bool,
    pub text_model: String,
    pub text_base_url: String,
    pub speech_app_id: String,
    pub has_speech_access_token: bool,
    pub has_speech_api_key: bool,
    pub asr_resource_id: String,
    pub asr_url: String,
    pub tts_resource_id: String,
    pub tts_voice: String,
    pub tts_url: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            text_api_key: String::new(),
            text_model: String::new(),
            text_base_url: "https://ark.cn-beijing.volces.com/api/v3".into(),
            speech_app_id: String::new(),
            speech_access_token: String::new(),
            speech_api_key: String::new(),
            asr_resource_id: "volc.bigasr.sauc.duration".into(),
            asr_url: "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel".into(),
            tts_resource_id: "seed-tts-2.0".into(),
            tts_voice: "zh_female_vv_uranus_bigtts".into(),
            tts_url: "https://openspeech.bytedance.com/api/v3/tts/unidirectional".into(),
        }
    }
}

impl Settings {
    pub fn public(&self) -> PublicSettings {
        PublicSettings {
            has_text_api_key: !self.text_api_key.is_empty(),
            text_model: self.text_model.clone(),
            text_base_url: self.text_base_url.clone(),
            speech_app_id: self.speech_app_id.clone(),
            has_speech_access_token: !self.speech_access_token.is_empty(),
            has_speech_api_key: !self.speech_api_key.is_empty(),
            asr_resource_id: self.asr_resource_id.clone(),
            asr_url: self.asr_url.clone(),
            tts_resource_id: self.tts_resource_id.clone(),
            tts_voice: self.tts_voice.clone(),
            tts_url: self.tts_url.clone(),
        }
    }
    pub fn patch(&mut self, p: SettingsPatch) -> Result<()> {
        macro_rules! set { ($($name:ident),*) => { $(if let Some(v)=p.$name { self.$name=v.trim().to_owned(); })* } }
        set!(
            text_api_key,
            text_model,
            text_base_url,
            speech_app_id,
            speech_access_token,
            speech_api_key,
            asr_resource_id,
            asr_url,
            tts_resource_id,
            tts_voice,
            tts_url
        );
        for (value, scheme) in [
            (&self.text_base_url, "https"),
            (&self.asr_url, "wss"),
            (&self.tts_url, "https"),
        ] {
            let url = reqwest::Url::parse(value).map_err(|_| "服务地址格式不正确")?;
            if url.scheme() != scheme
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err(format!("服务地址必须使用 {scheme} 且不能包含凭据"));
            }
        }
        Ok(())
    }
    pub fn validate_text(&self) -> Result<()> {
        if self.text_api_key.is_empty() || self.text_model.is_empty() {
            Err("请先在设置中配置豆包 API Key 和模型标识".into())
        } else {
            Ok(())
        }
    }
    pub fn validate_speech(&self) -> Result<()> {
        if self.speech_api_key.is_empty()
            && (self.speech_app_id.is_empty() || self.speech_access_token.is_empty())
        {
            Err("请配置语音 API Key，或 App ID 和 Access Token".into())
        } else {
            Ok(())
        }
    }
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path).map_err(|_| "读取 .env 失败")?;
        let mut map = BTreeMap::new();
        for line in text
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        {
            let (k, v) = line.split_once('=').ok_or(".env 格式错误")?;
            let val = if v.trim().starts_with('"') {
                serde_json::from_str::<String>(v.trim()).map_err(|_| ".env 字符转义错误")?
            } else {
                v.trim().to_owned()
            };
            map.insert(k.trim().to_owned(), val);
        }
        let mut settings = Self::default();
        macro_rules! read { ($($name:ident),*) => { $(if let Some(v)=map.remove(&stringify!($name).to_uppercase()) { settings.$name=v; })* } }
        read!(
            text_api_key,
            text_model,
            text_base_url,
            speech_app_id,
            speech_access_token,
            speech_api_key,
            asr_resource_id,
            asr_url,
            tts_resource_id,
            tts_voice,
            tts_url
        );
        Ok(settings)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        let tmp = path.with_extension(format!("env.{}.tmp", uuid::Uuid::new_v4()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp).map_err(|_| "创建凭据文件失败")?;
        let result = (|| -> Result<()> {
            macro_rules! write_fields { ($($name:ident),*) => { $(writeln!(file,"{}={}",stringify!($name).to_uppercase(),serde_json::to_string(&self.$name).map_err(|_| "配置编码失败")?).map_err(|_| "写入凭据失败")?;)* } }
            write_fields!(
                text_api_key,
                text_model,
                text_base_url,
                speech_app_id,
                speech_access_token,
                speech_api_key,
                asr_resource_id,
                asr_url,
                tts_resource_id,
                tts_voice,
                tts_url
            );
            file.sync_all().map_err(|_| "持久化凭据失败")?;
            std::fs::rename(&tmp, path).map_err(|_| "替换凭据文件失败")?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secrets_roundtrip_without_interpolation_and_public_redacted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        let s = Settings {
            text_api_key: "a$HOME\"\\\n中文".into(),
            ..Default::default()
        };
        s.save(&path).unwrap();
        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded.text_api_key, s.text_api_key);
        assert!(!serde_json::to_string(&loaded.public())
            .unwrap()
            .contains("$HOME"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn blank_clears_secret_and_omitted_preserves_it() {
        let mut s = Settings {
            text_api_key: "secret".into(),
            ..Default::default()
        };
        s.patch(SettingsPatch {
            text_model: Some("model".into()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(s.text_api_key, "secret");
        s.patch(SettingsPatch {
            text_api_key: Some("".into()),
            ..Default::default()
        })
        .unwrap();
        assert!(s.text_api_key.is_empty());
    }
}
