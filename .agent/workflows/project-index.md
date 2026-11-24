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
- [首次实现记录](../../milestone/2026-11-04.md)、[文档整理与首次同步记录](../../milestone/2026-11-24.md)。新任务按实际日期维护日志，不覆盖历史验证记录。
- `src/`：React 界面、Tauri 桥接、共享 TypeScript 类型和前端单元测试；`tests/`：Playwright 操作测试。
- `src-tauri/src/`：会话引擎、服务协议、设置、SQLite 与 IPC；`audio/`：跨平台设备与平台系统音频采集。
- `src-tauri/tests/`：使用模拟云服务的会话集成测试；`.github/workflows/check.yml`：检查与双平台安装包构建。

## GitHub 与交付约定

- 仓库：<https://github.com/dolphinfafa/vibesetter>，主分支 `main`。
- 推送会触发 GitHub Actions 检查；检查通过后分别在 Windows 与 Mac runner 构建 NSIS / DMG，并上传工作流产物，不自动创建 Release。
- 提交代码、文档和依赖锁文件；不提交 `.env`、数据库、日志、`node_modules/`、`target/`、`dist/`、`.artifacts/` 与测试运行产物。
- 汇报同步成功前检查工作区，并确认远端 `refs/heads/main` 与本地 `HEAD` 一致；上传成功不等于安装包构建通过。

## 验证边界

Linux 可验证 Rust 核心、界面及模拟服务；系统音频、授权和安装包必须在目标平台验证。不得把未执行的真机检查描述为通过。

2026-11-24 复核：生产前端构建、15 项 Rust 测试、2 项前端单元测试、5 项 Chromium 操作测试、全目标 Clippy、Rustfmt、Prettier 与 `npm audit` 均通过。真实云服务、硬件路由和目标平台安装验收仍未执行，详见当日日志。
