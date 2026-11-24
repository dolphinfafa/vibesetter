# Vibesetter

基于 Tauri 2 和 Rust 的实时个人助手，支持 Windows 11 x64 和 macOS 14.2+ Apple Silicon。

创建一个头像、名称和系统提示词，即可定义任意助手。会话支持多选麦克风、电脑音频、手动文本，以及文字和多个音频输出。声音通过外部虚拟声卡可以送入播客或会议软件。

## 开发

需要 Node.js 24、Rust stable（最低 1.90），以及 [Tauri 平台开发依赖](https://v2.tauri.app/start/prerequisites/)。Windows 使用 MSVC 工具链，Mac 安装 Xcode Command Line Tools（包含 Swift）。本项目正常开发不需要 Python；使用 Python 时必须进入 `vibesetter` conda 环境。

刚安装 Rust 后请打开新终端；Linux / macOS 也可先执行 `. "$HOME/.cargo/env"` 使 Cargo 命令可用。

```sh
npm ci
npm run dev          # 浏览器预览，可编辑助手；没有原生音频或模型连接
npm run tauri dev    # 桌面应用
```

在桌面应用设置中配置豆包文本模型 API Key 和模型 / Endpoint ID。音频功能需要开通火山语音服务，可配置新版语音 API Key，或旧版 App ID + Access Token。通过高级配置选择识别资源、合成资源与音色；默认音色对应 TTS 2.0，请保证账号已开通匹配的资源。文本模型必须支持 JSON 对象输出。连接检测会调用实际服务并产生少量用量。

生产凭据由设置页写入应用私有目录的 `.env`，仓库中的 `.env.example` 仅说明字段，不包含实际凭据。应用不会自动导入仓库的 `.env`。

## 构建

分别在目标系统执行：

```sh
# Windows 11，MSVC 环境
npm run tauri build -- --target x86_64-pc-windows-msvc --bundles nsis

# Apple Silicon Mac
npm run tauri build -- --target aarch64-apple-darwin --bundles dmg
```

产物位于 `src-tauri/target/<target>/release/bundle/`。GitHub Actions 包含两平台构建并上传安装包，不自动发布。首版构建没有发行签名或 Apple 公证；面向公开分发前需配置发行证书和公证。

## 验证

```sh
npm run build
npm test
npx playwright install chromium   # 首次运行浏览器测试
npm run test:e2e
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --locked
cargo check --manifest-path src-tauri/Cargo.toml
```

Playwright 优先使用 `PLAYWRIGHT_CHROMIUM_EXECUTABLE` 指定的浏览器；未指定时使用 Playwright 安装的 Chromium。

## 使用约定

- 一次运行一个助手会话。暂停会话会停止采集和请求，再次开始创建新的会话；历史文字记录可查看、删除。
- 正在会话时锁定路由。更新提示词从下一次请求生效；更改服务配置从下一次会话生效。
- 开始会话后默认自动判断是否需要回应。仅文字输出可在持续交谈时给建议；包含语音输出时，等待约 1.2 秒讲话停顿再判断。自动请求间隔至少 4 秒，是否回应仍由提示词与上下文决定。
- 文字建议生成期间继续接收对话，不会因新的人声被取消；语音生成或播放可被有效人声打断。模型决定暂不回应或请求失败后，等待新的完整语句再自动判断。
- 会话中显示自动判断状态。“回应一次”用于手动立即要求回应；“立即停止”取消当前与待处理回应，之后等待新输入或手动触发。
- 推荐耳机。Mac 首次使用需允许麦克风和屏幕 / 系统音频录制，修改权限后可能需要重启。
- 通过 BlackHole（Mac）或 VB-CABLE（Windows）等虚拟声卡接入其他软件：本应用选择其播放端，其他软件选择录音端，并避免回送到本应用。
- 系统采集排除本应用播放；无法排除其他软件重新播放的助手声音，路由仍需避免反馈环路。
- 本地保存助手及文字记录，不保存原始音频。启用的输入、提示词及上下文会发送到配置的云服务。

架构、接口与验证边界见 [项目总览](project-overview.md)，真机检查见 [平台验收](.agent/workflows/platform-validation.md)。
