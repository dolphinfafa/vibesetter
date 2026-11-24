export interface Assistant {
  id: string;
  name: string;
  prompt: string;
  avatar: string | null;
  color: string;
  created_at: number;
  updated_at: number;
}
export interface AudioDevice {
  id: string;
  name: string;
  kind: 'input' | 'output';
  is_default: boolean;
}
export interface SessionConfig {
  microphone_ids: string[];
  system_audio: boolean;
  text_input: boolean;
  text_output: boolean;
  output_ids: string[];
}
export interface Session {
  id: string;
  assistant_id: string;
  started_at: number;
  ended_at: number | null;
}
export interface Message {
  id: string;
  session_id: string;
  role: string;
  source: string;
  text: string;
  created_at: number;
  interrupted: boolean;
}
export interface PublicSettings {
  has_text_api_key: boolean;
  text_model: string;
  text_base_url: string;
  speech_app_id: string;
  has_speech_access_token: boolean;
  has_speech_api_key: boolean;
  asr_resource_id: string;
  asr_url: string;
  tts_resource_id: string;
  tts_voice: string;
  tts_url: string;
}
export type SettingsPatch = Partial<
  Omit<PublicSettings, 'has_text_api_key' | 'has_speech_access_token' | 'has_speech_api_key'>
> & {
  text_api_key?: string;
  speech_access_token?: string;
  speech_api_key?: string;
};
export type SessionEvent =
  | { type: 'status'; session_id: string; status: string }
  | { type: 'transcript'; session_id: string; source: string; text: string; is_final: boolean }
  | { type: 'message'; message: Message; visible: boolean }
  | { type: 'level'; session_id: string; source: string; value: number }
  | { type: 'interrupted'; session_id: string; message_id: string }
  | { type: 'error'; session_id: string; source: string; message: string };
export const defaultRoute: SessionConfig = {
  microphone_ids: [],
  system_audio: false,
  text_input: true,
  text_output: true,
  output_ids: [],
};
export const defaultSettings: PublicSettings = {
  has_text_api_key: false,
  text_model: '',
  text_base_url: 'https://ark.cn-beijing.volces.com/api/v3',
  speech_app_id: '',
  has_speech_access_token: false,
  has_speech_api_key: false,
  asr_resource_id: 'volc.bigasr.sauc.duration',
  asr_url: 'wss://openspeech.bytedance.com/api/v3/sauc/bigmodel',
  tts_resource_id: 'seed-tts-2.0',
  tts_voice: 'zh_female_vv_uranus_bigtts',
  tts_url: 'https://openspeech.bytedance.com/api/v3/tts/unidirectional',
};
