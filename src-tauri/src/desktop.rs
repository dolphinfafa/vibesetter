use crate::{
    audio::NativeAudio,
    engine::Engine,
    model::*,
    settings::{PublicSettings, Settings, SettingsPatch},
    store::Store,
};
use std::{
    path::PathBuf,
    sync::{Arc, RwLock},
};
use tauri::{Emitter, Manager, State};

struct AppState {
    engine: Arc<Engine>,
    env_path: PathBuf,
}

#[tauri::command]
fn list_assistants(state: State<'_, AppState>) -> Result<Vec<Assistant>> {
    state.engine.store.assistants()
}
#[tauri::command]
fn save_assistant(state: State<'_, AppState>, assistant: Assistant) -> Result<Assistant> {
    state.engine.store.save_assistant(assistant)
}
#[tauri::command]
async fn delete_assistant(state: State<'_, AppState>, id: String) -> Result<()> {
    if state.engine.is_assistant_active(&id).await {
        return Err("请先结束此助手的会话".into());
    }
    state.engine.store.delete_assistant(&id)
}
#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Result<PublicSettings> {
    Ok(state
        .engine
        .settings
        .read()
        .map_err(|_| "配置正在恢复")?
        .public())
}
#[tauri::command]
fn save_settings(state: State<'_, AppState>, patch: SettingsPatch) -> Result<PublicSettings> {
    let mut guard = state.engine.settings.write().map_err(|_| "配置正在恢复")?;
    let mut settings = guard.clone();
    settings.patch(patch)?;
    settings.save(&state.env_path)?;
    let public = settings.public();
    *guard = settings;
    Ok(public)
}
#[tauri::command]
async fn test_connection(state: State<'_, AppState>, service: String) -> Result<String> {
    let settings = state
        .engine
        .settings
        .read()
        .map_err(|_| "配置正在恢复")?
        .clone();
    crate::provider::test_connection(&settings, &service).await
}
#[tauri::command]
async fn list_devices(state: State<'_, AppState>) -> Result<Vec<AudioDevice>> {
    let engine = state.engine.clone();
    tokio::task::spawn_blocking(move || engine.devices())
        .await
        .map_err(|_| "枚举设备任务失败")?
}
#[tauri::command]
fn get_route(state: State<'_, AppState>, id: String) -> Result<SessionConfig> {
    state.engine.store.route(&id)
}
#[tauri::command]
async fn save_route(state: State<'_, AppState>, id: String, config: SessionConfig) -> Result<()> {
    if state.engine.is_assistant_active(&id).await {
        return Err("请先暂停会话再调整路由".into());
    }
    state.engine.store.save_route(&id, &config)
}
#[tauri::command]
async fn start_session(
    state: State<'_, AppState>,
    assistant_id: String,
    config: SessionConfig,
) -> Result<Session> {
    state.engine.start(assistant_id, config).await
}
#[tauri::command]
async fn stop_session(state: State<'_, AppState>) -> Result<()> {
    state.engine.stop().await
}
#[tauri::command]
async fn submit_text(state: State<'_, AppState>, text: String) -> Result<()> {
    state.engine.submit(text).await
}
#[tauri::command]
async fn trigger_reply(state: State<'_, AppState>) -> Result<()> {
    state.engine.trigger().await
}
#[tauri::command]
async fn interrupt_reply(state: State<'_, AppState>) -> Result<()> {
    state.engine.interrupt().await
}
#[tauri::command]
fn list_sessions(state: State<'_, AppState>, assistant_id: String) -> Result<Vec<Session>> {
    state.engine.store.sessions(&assistant_id)
}
#[tauri::command]
fn list_messages(state: State<'_, AppState>, session_id: String) -> Result<Vec<Message>> {
    state.engine.store.messages(&session_id)
}
#[tauri::command]
async fn delete_session(state: State<'_, AppState>, session_id: String) -> Result<()> {
    if state
        .engine
        .active_session()
        .await
        .is_some_and(|s| s.id == session_id)
    {
        return Err("不能删除进行中的会话".into());
    }
    state.engine.store.delete_session(&session_id)
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
            }
            let env_path = dir.join(".env");
            let settings = Settings::load(&env_path).map_err(std::io::Error::other)?;
            let store =
                Arc::new(Store::open(&dir.join("vibesetter.db")).map_err(std::io::Error::other)?);
            let handle = app.handle().clone();
            let emit = Arc::new(move |event: SessionEvent| {
                let _ = handle.emit("session-event", event);
            });
            let engine = Engine::new(
                store,
                Arc::new(RwLock::new(settings)),
                Arc::new(NativeAudio),
                emit,
            );
            app.manage(AppState { engine, env_path });
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                if let Some(state) = window.app_handle().try_state::<AppState>() {
                    let engine = state.engine.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = engine.stop().await;
                    });
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_assistants,
            save_assistant,
            delete_assistant,
            get_settings,
            save_settings,
            test_connection,
            list_devices,
            get_route,
            save_route,
            start_session,
            stop_session,
            submit_text,
            trigger_reply,
            interrupt_reply,
            list_sessions,
            list_messages,
            delete_session
        ])
        .build(tauri::generate_context!())
        .expect("Vibesetter 无法初始化桌面环境")
        .run(|app, event| match event {
            tauri::RunEvent::ExitRequested { .. } => {
                if let Some(state) = app.try_state::<AppState>() {
                    tauri::async_runtime::block_on(state.engine.stop()).ok();
                }
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen {
                has_visible_windows: false,
                ..
            } => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                } else if let Some(config) = app.config().app.windows.first() {
                    if let Ok(builder) = tauri::WebviewWindowBuilder::from_config(app, config) {
                        let _ = builder.build();
                    }
                }
            }
            _ => {}
        });
}
