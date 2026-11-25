# Project Index

## 行动规范

- 角色：首席工程师兼高级数据科学家。专业、简洁、结果导向。
- 修改前先列出三个计划要点。最小变更，复用成熟方案，新增依赖先确认。
- 未经验证不得报告完成。命令失败先分析根因再修复。
- Python 操作前确认激活 `vibesetter` conda 环境。本项目正常开发不需要 Python。
- 敏感信息仅存于被 Git 忽略的 `.env`；禁止写入日志或提交凭据。
- 持续维护本目录、根目录 `project-overview.md` 和 `milestone/YYYY-MM-DD.md`。
- 全部文本 UTF-8；界面和文档默认中文。

## 已确认技术决策

- Tauri 2、Rust、React、TypeScript、Vite；用户已授权实施方案所需依赖。
- Node.js 24、Rust stable（最低 1.90，当前环境验证 1.99.1）；当前服务器执行 Cargo 前需 `. "$HOME/.cargo/env"`。
- 目标平台 Windows 11 x64、macOS 14.2+ Apple Silicon。
- 助手职责由自定义系统提示词决定，没有固定面试或播客模式。
- 流式 ASR → 豆包文本模型 → 流式 TTS；允许数秒延迟。
- 输入支持多麦克风、一路系统音频、手动文本；输出支持文字和多个播放设备。
- 同时运行一个会话。标准语音环境为耳机；播客软件通过外部虚拟声卡接收。
- 默认自动判断回应：仅文字输出每 4 秒分析最新临时与确定转写，不等待静音或断句，生成期间不因人声取消；慢请求跳过忙碌时的定时点，完成后等下一轮，不并发或补发。相同有效内容不重复请求，识别修正可触发新判断。
- 包含语音输出时仍依据确定转写等待约 1.2 秒停顿并允许人声打断，自动请求间隔至少 4 秒。4 秒为分析周期 / 请求频率约束，不是回复时延保证，是否回应由提示词与上下文决定。
- “立即停止”清除当前及待处理回应；纯文字模式等待有效识别内容变化，语音模式等待新确定转写，或用户手动提交 / 要求回应。`auto_reply` 事件仅解释判断状态，不写入聊天历史。
- 本地 SQLite 保存助手和文字会话，不保存原始音频。生产凭据位于应用私有数据目录 `.env`。
- CPAL 使用稳定设备 ID；Rubato FFT 重采样；WebRTC VAD 判断人声，角色与是否值得回应仍由模型结合提示词决定。

## 常用命令

```sh
npm ci
npm run dev          # 浏览器界面预览；原生音频功能需桌面应用
npm run tauri dev    # 目标平台桌面开发
npm run build
npm test
npm run test:e2e
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo test --manifest-path src-tauri/Cargo.toml --all-targets --locked
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --locked -- -D warnings
npm run tauri build
```

没有桌面开发依赖的环境使用 `cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --locked`；该命令不覆盖原生音频模块。浏览器测试首次运行前执行 `npx playwright install --with-deps chromium`，或通过 `PLAYWRIGHT_CHROMIUM_EXECUTABLE` 指定已有 Chromium 浏览器。

## 文档与代码导航

- [项目总览](../../project-overview.md)：架构、IPC API、数据库、凭据与会话行为。
- [README](../../README.md)：安装依赖、配置服务、开发与目标平台打包。
- [平台验收](platform-validation.md)：Windows / Mac 的权限、设备、安装和真实服务检查。
- [首次实现记录](../../milestone/2026-11-04.md)、[首次同步与自动回应修复记录](../../milestone/2026-11-24.md)、[纯文字定时分析交付记录](../../milestone/2026-11-25.md)。新任务按实际日期维护日志，不覆盖历史验证记录。
- `src/`：React 界面、Tauri 桥接、共享 TypeScript 类型和前端单元测试；`tests/`：Playwright 操作测试。
- `src-tauri/src/`：会话引擎、服务协议、设置、SQLite 与 IPC；`audio/`：跨平台设备与平台系统音频采集。
- `src-tauri/tests/`：使用模拟云服务的会话集成测试；`.github/workflows/check.yml`：检查与双平台安装包构建。
- `src-tauri/src/engine/tests.rs`：自动调度回归测试，直接注入转写和人声事件，通过本地模拟 HTTP 服务验证请求、取消、上下文排队与间隔；不代表真实 ASR、设备或模型效果已验收。
- `src-tauri/src/engine/text_context.rs`：按来源缓存临时转写、比较有效内容并构造临时模型上下文；草稿不落盘，每来源比较缓存 24,000 字、每次请求草稿总预算 12,000 字。空包不视为新内容，相同文字由临时转正式不重复分析。

## GitHub 与交付约定

- 仓库：<https://github.com/dolphinfafa/vibesetter>，主分支 `main`。
- 推送会触发 GitHub Actions 检查；检查通过后分别在 Windows 与 Mac runner 构建 NSIS / DMG，并上传工作流产物，不自动创建 Release。
- 提交代码、文档和依赖锁文件；不提交 `.env`、数据库、日志、`node_modules/`、`target/`、`dist/`、`.artifacts/` 与测试运行产物。
- 汇报同步成功前检查工作区，并确认远端 `refs/heads/main` 与本地 `HEAD` 一致；上传成功不等于安装包构建通过。

## 验证边界

Linux 可验证 Rust 核心、界面及模拟服务；系统音频、授权和安装包必须在目标平台验证。不得把未执行的真机检查描述为通过。

2026-11-24 纯文字定时分析修复后通过生产构建、29 项 Rust 测试（无桌面依赖路径为 27 项）、2 项前端单元测试、7 项 Chromium 操作测试、Clippy、Rustfmt 和 Prettier。逐次验证结果见当日日志。首次同步时依赖审计为 0 项漏洞，后续修复未新增依赖。真实云服务、硬件路由和目标平台安装验收仍未执行。

2026-11-25 整理上述修复的 GitHub 交付，补充当日工作记录并复核文档、格式与提交范围；功能验证沿用 2026-11-24 的结果。后续验收需使用包含 `text_context.rs` 的版本，不能仅凭此前自动回应修复版本判断纯文字模式已消除断句依赖。
