# Tauri 控制面板

宠物继续使用 Rust / egui / Glow，控制面板迁移到独立的 Tauri 2 + Vue 3 进程。首页、偏好、日记、皮肤、剪切板和中转站采用暖白、淡紫与浅杏色，统一卡片、按钮、留白和详情预览。

## 生命周期和资源

- 仅运行宠物：不启动面板可执行文件，不创建 WebView，不运行前端轮询。
- 打开面板：启动相邻的 `desktop-pet-panel`，创建系统 WebView。重复打开会聚焦并切换页面，不启动第二个面板。
- 收起或关闭：退出面板进程，释放 WebView；宠物、剪切板监听与内容服务继续运行。
- 再次打开：重新创建窗口，从原生服务读取配置和记录；冷启动会比保留隐藏 WebView 稍慢。
- 退出宠物：停止记录、恢复桌面捣蛋、关闭面板，取消未完成内容任务并保存配置。父进程失联时面板检测管道断开后自行退出。

生产环境不启动 HTTP 服务器，不监听通信端口。宠物主程序没有 Tauri 依赖；主程序与面板使用私有 stdin/stdout JSON 管道，带请求编号、响应错误和导航事件。调试端口只在显式传入开发参数时开启。

## 后端和拖放

行为引擎、配置、桌面恢复、剪切板监听、SQLite 与文件内容库仍由宠物进程管理。面板不直接修改数据库；配置更新按字段校验，图标隐藏保留单独授权确认。正文使用纯文本预览，HTML/RTF 保留在内容表示中而不执行。图片通过后台生成的 PNG/base64 预览传输，皮肤预览从已校验定义生成。

Tauri 窗口复用 `src/native_content/` 的原生桥。Windows 关闭框架的普通文件拖放，覆盖 WebView2 子窗口的 OLE 接收目标；附件和虚拟文件继续通过文件描述符及内容流落地，拖出提供 COPY、文本/图片表示和真实导出路径。macOS 保留窗口事件转发，并清除 WebKit 自身接收注册，让原生窗口处理文件承诺；这部分本次未运行 macOS 验收。

复制由宠物进程的桥执行，避免把自身重新复制录成新历史。面板拖出期间和系统剪切板引用的导出资源保留引用，面板关闭释放不再使用的拖出引用。普通 ZIP 拖入中转站，皮肤只通过皮肤页专用区域加载。

## 构建与打包

需要 Rust、Node.js 24 与 Windows WebView2 Runtime。Node.js 仅用于构建，运行发布包不需要 Node.js、Python 或前端服务器。

```sh
npm --prefix panel ci
npm --prefix panel run build
cargo build --locked --workspace
cargo run -- --settings
cargo build --locked --release --workspace
python scripts/package.py --target x86_64-pc-windows-msvc
```

调试和发布构建都需要使用 `--workspace` 构建面板。普通 `cargo run` 默认只运行宠物；面板文件未构建时会显示错误而不自动启用旧界面。`--panel-binary PATH` 可指定面板位置。Windows 包包含 `Mochi.exe` 与 `desktop-pet-panel.exe`，必须保留在同一目录；macOS 包将两者放入 `Mochi.app/Contents/MacOS`。

## 验证

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --release --locked --workspace
cargo run --release --example seed_panel_demo -- .artifacts/tauri-demo
node panel/tests/native.mjs .artifacts/tauri-demo
python scripts/profile_tauri_windows.py --binary target/release/desktop-pet.exe --output .artifacts/tauri-perf
python scripts/ci_smoke.py --binary target/release/desktop-pet.exe --output .artifacts/tauri-smoke
```

`native.mjs` 连接真实 Tauri / WebView2 窗口，要求隔离演示数据库包含中文工作笔记和图片中转记录；不会记录系统剪切板或隐藏真实桌面图标。验证六页导航、中文组合输入、昵称、互动、授权、全文搜索、导出准备、图片和收藏、皮肤加载/重载/错误保留、系统窗口关闭保存草稿、立即重开、收起后宠物存活及进度清理。截图位于 `.artifacts/tauri-e2e`。本次 Windows 验证摘要见 [tauri-windows-verification.json](tauri-windows-verification.json)。

性能脚本使用乖巧模式，关闭剪切板记录和走动，隔离数据与热键；采样首次打开前和三次关闭后，检查每次面板/WebView 进程全部释放，并验证宠物正常退出。结果见 [PERFORMANCE.md](PERFORMANCE.md)。这属于短时对照，不替代长期性能监测；`--seconds 30` 可延长每段采样。

`--screenshot PATH` 在 Windows 使用 WebView2 CapturePreview 获取真实面板画面。`--panel-debug-port PORT` 和 `--control-stdin` 仅用于开发，默认关闭；`--no-shell` 避免测试热键干扰正在运行的宠物。`--legacy-panel` 仅保留旧 egui 面板作开发对照。

本次 macOS 检查按要求暂停，CI 默认仅 Windows；手动运行 CI 时可打开 `include_macos`。macOS 的 WebKit 截图尚未接入，现有 macOS smoke 明确标记为旧 egui 截图，不能用于声称新面板验收通过。真实邮件、浏览器和聊天应用的跨应用拖放仍按 [TESTING.md](TESTING.md) 人工验收。
