# Windows 产品壳验收记录

## 第二组：原生 Windows 产品壳

状态：**实现与 Windows x64 目标构建已完成；2026-10-01 真实 Windows 11 x64 HWND
启动与 clean close smoke 已通过；同日已补齐侧栏与状态栏的基础界面，
当前 200% DPI 下可见窗口检查通过，完整人工交互及其他缩放比例待验收。**

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

### 2026-10-01 可见界面缺口及修复

用户提供的当前窗口截图显示：侧栏与状态栏文字过小，侧栏缺少正常导航样式和内容。
源码核对确认侧栏只是单个 `STATIC` 控件中的“文件 / 大纲 / 搜索”占位字符串，
不是三个可切换的导航控件，也没有文件列表、大纲列表或搜索输入与结果面板。
侧栏和状态栏未设置 UI 字体，尺寸布局虽按 DPI 缩放，字体未随之配置；
同时缺少内边距、选中/悬停状态及侧栏/状态栏的完整主题绘制。
现有 HWND / Present / TSF smoke 不能作为这些视觉和功能项的通过证据。
该缺口属于产品壳补齐项，不能用第四组输入测试通过或第五组资源功能替代。

同日已在工作区修复：侧栏、状态栏使用按 DPI 获取的系统 UI 字体，增加内边距、
导航选中/悬停/焦点状态及浅色/深色绘制。三个导航接入当前目录 Markdown 文件列表、
共享 `OutlineTree` 大纲和共享 `SearchResults` 文内搜索；跳转复用共享选区与 caret 滚动。
同时修复渲染实测布局未同步回编辑器、scene viewport 未携带滚动原点的问题。
原生字体 19/19、render 2/2、shell 19/19 和真实窗口 smoke 通过。
当前 192 DPI（200%）已检查三个页面、Unicode 搜索、目标可见及文件打开的实际截图。
该检查使用定向 Win32 消息，不构成物理键鼠或真实 IME 通过记录。
文件面板目前是同目录 Markdown 列表，大纲目前按层级缩进展示；
文件夹树、大纲折叠等完整 macOS 面板功能对齐未包含在本轮验收。
详见 [侧栏与滚动修复记录](windows-chrome-native-fix-20261001.md)。

随后按用户反馈继续优化样式：采用共享 Yu 浅色/深色主题、圆角分段导航、
按界面语言选择的 UI 字体、辅助文字层级、左右分布的轻量状态栏和带留白的正文阅读列。
原生 DPI 字体、正文 HWND 层级、caret 屏幕坐标与 ACP 命中回归通过。
详见 [Windows 样式优化记录](windows-style-polish-20261001.md)。

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

此前 macOS 开发与交叉编译未覆盖真实 HWND 运行。2026-10-01 已在本机 Windows 11 x64
补绿，证据见 [原生修复记录](windows-group4-native-fix-20261001.md)。
文件对话框和完整交互人工清单仍按真机验收计划逐项记录。

### 明确不属于第二组

- D3D11/DXGI renderer 与 glyph atlas：第三组。
- 完整 DirectWrite analyzer/fallback/rasterizer 产品接线：第三组。
- TSF / `ITextStoreACP`、IME、完整键鼠编辑链：第四组。
- UI Automation / Narrator / 高对比度完整验收：第六组。
- MSIX、Store metadata、最终 exe icon/resource 与签名：第七组。

## 第三组：DirectWrite + D3D11 / DXGI

状态：**实现与 Windows x64 目标构建已完成；2026-10-01 原生 DirectWrite 测试及真实
D3D first Present / resize render smoke 已通过，人工交互与 DPI 清单继续执行。**

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
  - many-to-many cluster、RTL glyph 绘制顺序/offset 转换、UTF-16 → UTF-8 source 映射；
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

状态：**主体实现、Windows x64 原生构建与 TSF 初始化 smoke 已完成；2026-10-01
用户确认微软拼音第一轮及新版候选定位、选区取消/替换、Undo/Redo 第二轮通过；完整 IME 与 DPI 人工验收
仍未完成，暂不正式结项。**

2026-10-01 真机验收计划见 [Windows 第四组真机验收计划](windows-group4-manual-acceptance.md)。
本轮已确认使用 Windows 11 x64 本机键鼠、单显示器；先覆盖真实 IME 与单屏
100% / 125% / 150% DPI，跨显示器不同 DPI 迁移仍需补测。计划不构成通过记录。

同日原生预检发现 DirectWrite RTL shaping 测试失败、窗口 self-check 退出 1；
详细结果见 [Windows 原生预检记录](windows-group4-preflight-20261001.md)。
随后已修复 TSF 可空焦点关联、RTL glyph 翻译、editor viewport 配置与选区/caret 接线，
原生字体 19/19、render 2/2、shell 16/16 和真实窗口 self-check 均通过。
详情见 [Windows 原生修复记录](windows-group4-native-fix-20261001.md)。
用户随后在侧栏修复版上确认 `zhongwen` 候选提交“中文”位置正确且只提交一次，
`ceshi` 按 Escape 完全取消无残留，Ctrl+Z / Ctrl+Y 正常。
随后用户确认 `main@fc4558ec` 新版第二轮正常：当前 200% 下候选框靠近光标，
选区内 composition 取消保留原文、候选仅替换一次、方向键/Enter 和 Undo/Redo 正常。
这两轮不等于四种 IME 全套通过。其余真实 IME 与 DPI 人工清单待执行，第四组保持未正式结项。

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
- 鼠标编辑链已接到共享 layout/editor 语义：单击定位、Shift-click 扩选、
  按住左键拖选、双击按共享 UAX word boundary 选词、三击选完整物理源行、
  滚轮滚动与拖选越界自动滚动；viewport scroll 在 resize / DPI 同步时保持并
  重新 clamp。所有选择变化都会通知 TSF。
- `ITextStoreACP::GetACPFromPoint` 区分 nearest 请求；普通点命中不会把窗口外或
  无布局位置静默映射成 ACP 0。
- 新增 Windows 需要的行首/行尾编辑命令；命令语义仍位于 `yu-editor`，Win32
  只负责 native key → shared command 的映射。

### 已执行验证

- `cargo test -p yu-shell-windows`：14 项通过，其中包含 ACP surrogate、
  backward selection、composition projection、QueryInsert range 与 chunked
  UTF-16 read 回归。
- `cargo test -p yu-editor keymap`：5 项通过。
- `cargo test -p yu-editor`：完整编辑器回归通过，无失败；新增覆盖 Unicode 双击选词、
  emoji 与 CRLF 行选择的回归。
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
