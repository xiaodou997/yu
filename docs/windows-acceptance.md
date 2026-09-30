# Windows 产品壳验收记录

## 第二组：原生 Windows 产品壳

状态：**实现与 Windows x64 目标构建已完成；真实 Windows 会话窗口 smoke 待执行。**

本组只建立平台产品壳与文档生命周期，不提前实现后续组的 D3D 渲染、
完整 DirectWrite、TSF 或 UI Automation。编辑器区域是独立的原生 surface host，
不是 RichEdit/TextBox，也不是第二条文本渲染路径。

### 已完成

- Rust + `windows-rs` 原生 Win32 产品壳；不新增 C++/C# 胶水层。
- Windows GUI subsystem，可执行文件正常启动时不附带控制台窗口。
- 原生主窗口、系统标题栏、File/Edit/View 菜单与 Windows access key。
- Ctrl+N / O / S、Ctrl+Shift+S、Ctrl+Z / Y、Ctrl+Shift+L accelerator。
- 左侧栏、编辑器 surface host、状态栏的 DPI-aware 布局骨架。
- Per-Monitor DPI Awareness V2；`WM_DPICHANGED` 使用系统建议窗口矩形重新布局。
- 跟随系统 Light/Dark 事实，并把 Appearance 送回共享 Rust 产品状态；Windows 11
  标题栏同时设置 immersive dark 与 main-window backdrop。
- 原生 `IFileOpenDialog` / `IFileSaveDialog`，Markdown 文件过滤与 UTF-16 Windows
  路径完整保留。
- Windows 壳直接持有 `DocumentEditorSession`，canonical source / dirty / history /
  close state 仍只有 Rust 一份。
- untitled 文档不会把 `Untitled.md` 当真实路径写盘；首次保存必须走 Save As。
- 新增 `DocumentEditorSession::save_as_close`，让“关闭未命名文档 → Save As → Close”
  保持在同一 Rust close state machine 内。
- 文件关闭继续使用 Save / Discard / Cancel；检测到外部变化时不会静默覆盖。
- 发布语言与 macOS 对齐：English、简体中文、繁體中文、日本語、한국어。
- 新增独立 `windows-shell` CI job、PowerShell self-check 和
  `tools/check-platform-parity.py`，第二个平台不会再掉出验证面。

### 已执行验证

- `cargo test -p yu-shell-windows`：8 项通过。
- `yu-storage` 的 untitled close + Save As 生命周期回归通过。
- macOS 主机上的 `cargo clippy --workspace --all-targets -- -D warnings` 通过。
- 使用 `cargo-xwin` + Windows SDK/CRT 对
  `x86_64-pc-windows-msvc` 执行：
  - `cargo xwin check -p yu-shell-windows`：通过；
  - `cargo xwin clippy -p yu-shell-windows -- -D warnings`：通过；
  - `cargo xwin build -p yu-shell-windows`：通过。
- 生成的 `yu-shell-windows.exe` 已由 LLVM PE header 检查确认：
  `IMAGE_SUBSYSTEM_WINDOWS_GUI`。
- crate dependency、CI parity、platform parity 与 `git diff --check` 门禁通过。

### 仍需真实 Windows 会话确认

仓库已经提供：

```powershell
./platform/windows/yu-shell-windows/run-self-checks.ps1
```

它会在 Windows 上执行模型测试、原生编译并运行
`--window-self-check`：创建真实 HWND、菜单/子窗口/DPI/主题状态，然后对一个 clean
临时文档自动走正常关闭路径。

当前开发 Runner 是 macOS，且本次分支推送没有生成 GitHub workflow run，因此
**真实 HWND 启动 smoke 尚未被执行**。这项必须在可用 Windows runner/真机上补绿，
不能用交叉编译冒充。

### 明确不属于第二组

- D3D11/DXGI renderer 与 glyph atlas：第三组。
- 完整 DirectWrite analyzer/fallback/rasterizer 产品接线：第三组。
- TSF / `ITextStoreACP`、IME、完整键鼠编辑链：第四组。
- UI Automation / Narrator / 高对比度完整验收：第六组。
- MSIX、Store metadata、最终 exe icon/resource 与签名：第七组。
