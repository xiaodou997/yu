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

## 第三组：DirectWrite + D3D11 / DXGI

状态：**实现与 Windows x64 目标构建已完成；真实 Windows GPU/窗口 render smoke
仍必须在 Windows runner/真机执行。**

本组把第二组留下的 editor surface HWND 接成真正的 Yu 渲染链：

`DocumentEditorSession → ViewportFrameBuilder → RenderPlan → D3DRenderer → DXGI swapchain`。

没有引入 RichEdit、DirectWrite TextLayout 或 GDI 正文绘制；Markdown 正文依然只有
共享 Rust scene/render 一条路径。

### 已完成

- `yu-font-windows` 接入真实 DirectWrite COM：
  - `AnalyzeScript` / `AnalyzeBidi`；
  - first-strong paragraph base direction；
  - `IDWriteFontFallback::MapCharacters`；
  - `GetGlyphs` / `GetGlyphPlacements`；
  - many-to-many cluster、RTL 原生 glyph 顺序、UTF-16 → UTF-8 source 映射；
  - fallback `mapped_scale` 进入 face identity、metrics 与 rasterization，避免
    fallback 字体“排版尺寸对、位图尺寸错”。
- `DirectWriteGlyphRasterizer` 与 shaper 共用同一张 `SharedFaceTable`；
  普通文字以 DirectWrite 1x1 alpha texture 进入共享 glyph atlas，不建立一套
  Windows 私有 ClearType atlas。
- 新增 `yu-render-windows`：
  - D3D11 feature level 11_0；
  - DXGI flip-discard 双缓冲 swapchain；
  - glyph atlas / image texture residency；
  - FillRect、RoundedFillRect/Shadow、Glyph、Image、Polyline 的 GPU command；
  - resize 后重建 backbuffer target；
  - DXGI device removed/reset 显式上报并由 shell 重建 renderer。
- D3D 只消费共享 `yu-render::build_draw_commands` 的 `DrawCommand`。
  `GpuCommand` 只负责常量缓冲布局，不重新解释 Markdown、布局或场景语义。
- editor surface HWND 的 `WM_PAINT` 不走 GDI 绘制，只触发 Rust/D3D 重绘；
  surface class 也没有背景 brush，避免出现第二条视觉路径。
- DPI 改变时同时更新 `SurfaceConfig`、`ViewportRenderConfig` 与 glyph
  `raster_scale`；跨 DPI 时重建 shaper/atlas/renderer，防止混用旧 DPI 位图。

### 自动判据

- 非 Windows 主机：
  - `yu-font-windows` 的 14 条 cluster/run 翻译测试；
  - `yu-render-windows` 的 D3D constant-buffer 映射与 16-byte alignment 测试；
  - 共享 render backend、workspace、layout 的全仓测试继续作为语义 oracle。
- Windows：
  - 真实 `DirectWriteShaper` 跑 `yu_core::shaping_conformance::audit`；
  - 显式 shape Hebrew / Arabic / Devanagari；
  - shaper 铸出的 face id 必须被它自己的 rasterizer 解回并成功取 metrics/bitmap；
  - `--window-self-check` 会创建 HWND、DirectWrite shaper、D3D device/swapchain，
    构建一帧并 Present，然后自动关闭。
- macOS 开发 Runner 使用 `cargo-xwin` 对
  `x86_64-pc-windows-msvc` 做 check、all-targets clippy 与最终 PE 链接。

### 明确留到后续组

- TSF / IME / 完整键鼠编辑链：第四组。
- 图片、公式、Mermaid 等资源从产品调度器到 Windows GPU cache 的完整产品接线：
  第五组；D3D backend 的 texture upload/residency API 已存在，当前 shell 在没有
  publication 时仍按共享 RenderPlan 规则画 fallback，不会白屏或阻塞。
- UI Automation / Narrator / Contrast Theme 完整验收：第六组。
- 彩色 emoji 的 Windows color-glyph 专用栅格路径也放在产品功能对齐组；本组先
  保证 Unicode shaping、cluster/caret 几何与普通 glyph atlas 正确，不把
  ClearType/color-font 私有格式塞进共享 glyph seam。

## 第四组：TSF / IME / 完整编辑输入链

状态：**主体实现与 Windows x64 目标构建已完成；真实 Windows TSF/IME 交互 smoke
仍必须在 Windows runner/真机执行。**

本组让第三组的 D3D editor surface 从“可显示”进入“可编辑”：Windows 只负责
把 TSF、键鼠、剪贴板与屏幕几何翻译到共享 Rust 编辑模型，不建立 RichEdit、
TextBox 或第二份平台私有正文。

### 已完成

- 接入 TSF `ITfThreadMgr → ITfDocumentMgr → ITfContext`，editor surface 获得焦点时
  直接激活同一文档上下文，并通过 `ITfKeystrokeMgr` 先让活动输入法消费按键。
- 实现 `ITextStoreACP`：
  - read / read-write lock 与异步 lock upgrade；
  - selection、plain text、`GetEndACP`；
  - `SetText` / `InsertTextAtSelection`；
  - `GetTextExt` / `GetScreenExt` / `GetACPFromPoint` / `GetWnd`；
  - text / selection / layout sink 通知。
- ACP 坐标严格按 UTF-16 code unit 暴露；共享 `TextSnapshot` 仍是
  UTF-16 ↔ UTF-8 唯一坐标事实。代理对中点会被拒绝，不做就近取整。
- IME preedit 不写进 canonical Markdown source：
  `CompositionOverlay` 被投影进 TSF text stream，composition update 只更新 overlay，
  `OnEndComposition` 才通过共享 editor composition API 提交。
- `QueryInsert` 只返回当前文档内的有效 ACP range；分块 `GetText` 不会把 UTF-16
  surrogate pair 从中间截断。
- 候选框 / caret 几何来自共享 `LayoutSnapshot`，再转换到 per-monitor DPI 的
  screen coordinates；composition caret 走同一套 layout geometry。
- 完整基础键盘链已接入：Backspace/Delete、左右、上下、Home/End、
  Ctrl/Alt word move、Shift 扩选、Ctrl+Home/End、Enter、Tab、PageUp/PageDown、
  Undo/Redo、Select All。
- `WM_CHAR` 处理 BMP 与 surrogate pair；普通提交文字仍统一落到
  `EditorCommand::InsertText`。
- Windows Unicode clipboard 已接入 Copy / Cut / Paste；正文变更仍经过共享
  `DocumentEditorSession`。
- 鼠标单击定位、Shift-click 扩选与按住左键拖选已接到共享 layout hit-test，
  并在选择变化后通知 TSF。
- 新增 Windows 需要的行首/行尾编辑命令；命令语义仍位于 `yu-editor`，Win32
  只负责 native key → shared command 的映射。

### 已执行验证

- `cargo test -p yu-shell-windows`：14 项通过，其中包含 ACP surrogate、
  backward selection、composition projection、QueryInsert range 与 chunked
  UTF-16 read 回归。
- `cargo test -p yu-editor keymap`：5 项通过。
- `cargo test -p yu-editor`：完整编辑器回归通过，无失败。
- `cargo clippy -p yu-editor --all-targets -- -D warnings`：通过。
- `cargo xwin check -p yu-shell-windows --target x86_64-pc-windows-msvc`：通过。
- `cargo xwin clippy -p yu-shell-windows --target x86_64-pc-windows-msvc -- -D warnings`：
  通过。
- `cargo xwin build -p yu-shell-windows --target x86_64-pc-windows-msvc`：通过，
  Windows x64 可执行文件已完成最终链接。
- `cargo fmt --all -- --check` 与 `tools/check-deps.py`：通过。

### 仍需真实 Windows 会话确认

- 微软拼音 / 五笔候选窗跟随 caret，composition start/update/commit/cancel。
- 日文 Microsoft IME 与韩文 IME 的 preedit、转换、候选选择和 commit。
- emoji / surrogate pair 的直接输入与 IME 混合输入。
- 多显示器、不同 DPI、窗口移动/resize 后候选框几何。
- 鼠标、键盘与 TSF 抢键边界，特别是 composition 中的方向键、Enter、Escape。
- 第二、三组遗留的真实 HWND / DirectWrite / D3D Present smoke 同一轮补绿。

### 明确留到后续组

- 图片、公式、Mermaid 等资源到 Windows GPU cache 的完整产品接线与 color emoji：
  第五组。
- UI Automation / Narrator / Contrast Theme：第六组。
- MSIX、Store metadata、最终 exe icon/resource、签名与发布收尾：第七组。
