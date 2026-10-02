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

2026-10-02：侧栏改为“文件 / 大纲”两个 tab，文内搜索移到正文顶部的独立查找栏，
对齐 macOS 的导航结构。Ctrl+F 聚焦查询，Enter / Shift+Enter 与上下按钮循环切换匹配，
计数随当前选区更新；完成 / Escape 关闭查找栏并清除搜索高亮。
搜索直接使用共享 `SearchState` 的匹配范围，跳转继续复用原生选区及 caret 滚动。
切换或隐藏侧栏不关闭查找栏。

同日继续优化顶部菜单和正文几何：文件 / 编辑 / 显示入口使用与 Yu 主题一致的自绘按钮，
下拉部分保留系统弹出菜单与原有命令；Alt+F / E / V 打开菜单，F10 聚焦菜单入口，
左右键切换、上下键或 Enter 打开，Escape 返回正文。系统标题栏保留。
DirectWrite 字体 ascent / descent 决定正文基线、caret、选区及搜索高亮的文字范围，
段落行间距继续用于排版，不再算入光标与高亮高度；TSF / UIA 复用相同几何。
相关 layout / editor / workspace / DirectWrite / shell 回归及严格 clippy 通过，
新增断言覆盖标题的 caret、选区及当前搜索高亮高度和空行间距。
真实窗口 smoke、54 项独立进程 UIA 检查，以及 F10 / 菜单方向键 / Alt+F / 搜索命令
验证通过；当前可见效果在 192 DPI（200%）检查。

同日修复换行和段落留白过大：Windows 传入排版的基准由已乘倍率的行高改为 16pt
字号，正文及标题的行高倍率由共享样式应用一次。共享段落输入明确将末尾换行视为
段落终止符，不额外生成空行；原始文本布局仍保留末尾编辑行。CRLF 在字体后端拆成
两个 cluster / run 时合并为一个强制换行，保留源字节和真实内部空行。
新增 LF / CRLF、分拆样式 run、Yu 浅色 / 深色原生段落几何回归；已有主动空段落、
编辑、滚动及 caret / TSF 回归通过。layout / editor / workspace / Windows shell 完整
测试、严格 clippy、真实资源渲染集成、窗口 smoke 和 54 项 UIA 检查通过。
当前 200% 可见窗口确认标题、正文与下一节按主题间距衔接。

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
用户确认微软拼音及安排的五笔、日文、韩文基本项目正常，随后要求跳过缩放并进入
下一阶段、由代理自行验证。已补齐模型输入、Unicode 保存重开及持续原生渲染自动验证，
按本轮调整后的范围收口并进入第五组；原始全面 IME / DPI 人工矩阵仍保留未验收例外。**

2026-10-01 真机验收计划见 [Windows 第四组真机验收计划](windows-group4-manual-acceptance.md)。
本轮已确认使用 Windows 11 x64 本机键鼠、单显示器，当前实测为 200%。
100% / 125% / 150% 实际缩放按用户要求跳过，跨显示器不同 DPI 迁移仍需补测。
原始计划不构成通过记录。

同日原生预检发现 DirectWrite RTL shaping 测试失败、窗口 self-check 退出 1；
详细结果见 [Windows 原生预检记录](windows-group4-preflight-20261001.md)。
随后已修复 TSF 可空焦点关联、RTL glyph 翻译、editor viewport 配置与选区/caret 接线，
原生字体 19/19、render 2/2、shell 16/16 和真实窗口 self-check 均通过。
详情见 [Windows 原生修复记录](windows-group4-native-fix-20261001.md)。
用户随后在侧栏修复版上确认 `zhongwen` 候选提交“中文”位置正确且只提交一次，
`ceshi` 按 Escape 完全取消无残留，Ctrl+Z / Ctrl+Y 正常。
随后用户确认 `main@fc4558ec` 新版第二轮正常：当前 200% 下候选框靠近光标，
选区内 composition 取消保留原文、候选仅替换一次、方向键/Enter 和 Undo/Redo 正常。
用户随后确认第三轮 emoji/补充平面字符旁输入与取消、搜索框拼音及焦点返回、
长文档滚动与最大化/还原后的候选定位正常（当前 200%）。
这三轮不等于四种 IME 全套或所有 Unicode/键鼠项目通过。
第四轮安排的 200%→150%、滚动/候选/拖选操作由用户报告正常；后续读取时
测试窗口已关闭、当前实测 200%，该次 150% DPI 快照与 150% 启动路径仍待补。
用户随后明确要求跳过缩放验收；本轮停止安排该 DPI 补采集/启动及 125%/100%
实际切换，未执行项按 NOT_RUN 记录并注明用户选择，不能视为 DPI 全面通过。
用户随后报告安排的微软五笔、日文与韩文基本输入、转换/组合、候选、取消及 Undo/Redo 正常；
结果按用户反馈记录，逐种模式/键序及完整 C01–C08 明细未另行采集。
用户随后要求直接进入下一阶段并自行验证。本轮使用真实 HWND / D3D 自动补测了
20 次模型 composition 提交、20 次取消、Undo/Redo、中文/空格/emoji 路径保存重开，
以及 600 秒、2,894 帧连续滚动渲染，全部通过。
这些是共享编辑模型与原生渲染的集成证据，不是微软 IME 实际键盘操作证据；
Yu 与记事本互拷、物理关闭对话框三分支及最后一轮完整人工操作未补采集。
按用户调整后的范围继续第五组，不再以等待人工回复阻塞推进，也不宣布原始全面矩阵通过。
自动补测详见 [第五组原生验收记录](windows-group5-native-acceptance-20261001.md)。

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

## 第五组：图片 / 公式 / Mermaid / GPU 资源与彩色 emoji

状态：**2026-10-01 Windows x64 产品资源链已实现，本机原生自动验收通过；
当前 200% 可见窗口已检查。完整图片格式、全部彩色字体格式、实际跨 DPI 和真实驱动
重置兼容性不包含在本次通过声明中。**

Windows 壳通过有界后台任务加载本地图片，复用共享资源缓存和现有原生公式 / Mermaid
helper；SVG 栅格化后与图片一起进入已有 D3D texture 路径。资源布局、caret 与输入
继续使用共享 Rust 文档布局，加载与失败回退均不修改 canonical Markdown。
DirectWrite COLR 调色板字形进入共享 RGBA atlas，本机 Segoe UI Emoji 的表情、
ZWJ 与肤色样本已验证彩色呈现。

本轮同时修复普通图片资源类型减法下溢、旧 flip swapchain 未释放导致重建失败、
切换文档后新 revision 被旧 frame gate 拒绝，以及缩略图改变原始布局尺寸的问题。
GPU 恢复验证读取实际渲染目标，并要求重建前后像素完全一致。

| 已执行验证 | 实际结果 |
| --- | --- |
| 原生 DirectWrite / D3D / shell 测试 | 20 / 4 / 21 项通过；另显式执行 1 项真实资源集成测试通过 |
| 完整共享 editor / workspace / assets / render / storage 回归 | 777 项通过、0 失败；1 项既有 ignored 用例未执行 |
| 持续原生资源渲染 | 600 秒、2,894 帧通过；含 20 次模型提交、20 次取消及 Unicode 保存重开 |
| 实际 GPU 读回、重建、资源重新上传 | 透明 SVG 有效彩色像素及重建前后像素一致性通过 |
| 丢失图片、非法公式 / Mermaid、缺失 helper、过期回包 | 保留源文、失败后稳定回退及旧 revision 隔离通过 |
| 可见本机窗口 | PNG、透明 SVG、行内 / 独立公式、中文 Mermaid、彩色 emoji 检查通过，192 DPI |
| 原生构建与最终窗口 smoke | helper / GUI 构建、check、真实 HWND / Present / clean close 通过 |
| 质量门禁 | all-targets clippy -D warnings、fmt、diff check 通过；PowerShell 按既有依赖策略核对 27 包 / 114 内部边 |

运行包须同时包含 `yu-shell-windows.exe` 与 `yu-document-renderer.exe`。
完整证据、复现命令、资源限制和剩余兼容性范围见
[第五组原生验收记录](windows-group5-native-acceptance-20261001.md)。
下一阶段为第六组 UI Automation / Narrator / Contrast Theme；签名与发布包装仍为第七组。

## 第六组：UI Automation / Narrator 接线 / Contrast Theme

状态：**2026-10-01 软件实现与本机自动验收完成。Narrator 实际语音和系统对比主题
实际切换尚未验收，不计入本轮通过声明。**

真实编辑 HWND 已暴露 TextPattern / TextPattern2、选区 / caret / 可见范围、共享
Markdown 语义节点、任务 Toggle、屏幕几何及事件；原生动作经过现有 editor model。
F6 / Shift+F6 可在正文与可见侧栏间切换；搜索框有系统 UIA 名称。
对比模式使用系统颜色，不透明选区下的文字通过实际 GPU 读回验证，普通模式恢复通过。

独立进程 UIA 客户端 37 项通过；字体 / D3D / shell 常规测试 20 / 4 / 22 项通过，
第五组真实资源集成显式重跑通过。全 workspace 1,532 项通过、0 失败，4 项默认
ignored（其中资源集成已另行执行）；全仓 all-targets clippy、fmt、diff 与依赖方向通过。
扩大验证时发现的 Windows 编译 cfg、PNG 导出目录身份和图片同长度改写漏检已修复。

完整证据、复现入口和未验范围见
[第六组原生验收记录](windows-group6-native-acceptance-20261001.md)。
下一阶段为第七组发布包装；Narrator / 系统主题人工体验仍保留独立验收项。

## 第七组：MSIX / EXE 资源 / Store 元数据 / 发布包装

状态：**2026-10-01 Windows x64 打包软件实现与本机自动验收通过。正式签名、
安装后的验收、WACK 和 Store 提交待发布身份及证书配置，暂不正式结项。**

EXE 已包含 Yu 图标、版本、PMv2 DPI、Common Controls v6 和权限声明；主窗口的大、
小图标也已接线。Release MSIX 包含主程序、公式 / Mermaid helper、45 项 payload、
多缩放 PNG / PRI 和许可证。开发身份与 Partner Center 正式身份分开配置；签名入口
检查 Publisher、证书有效期、私钥及代码签名用途，支持 HTTPS 时间戳与独立验签。

包内真实 HWND / Present、38 项外部 UIA、公式 / Mermaid helper 和 13 项打包防护
验证均通过；全 workspace 1,532 项无失败，真实 GPU 资源集成另行通过，clippy、
格式、依赖方向与 CI parity 通过。五语商店草稿与 Windows 隐私政策已准备。

当前生成物是**未签名开发候选包**；解包后运行验证通过，不代表 MSIX 安装、
文件关联、更新、卸载或 Store 认证已通过。
完整范围与证据见[第七组原生验收记录](windows-group7-native-acceptance-20261001.md)，
构建与签名配置见[Windows 打包说明](windows-packaging.md)。

### 发布路线调整：优先 GitHub Release EXE 安装版，另提供便携版

用户已选择 Windows 版先通过 GitHub Release 分发未签名 EXE 安装器与便携 ZIP，用户量增长后再
考虑 Microsoft Store。新增 `-Channel GitHub` 构建与实际 ZIP 解压验证；发布包同时
提供 `Yu.exe`、渲染 helper、许可证、说明和 SHA256 校验文件。正式构建要求干净
Release 源码；本地或 CI 的 Debug / 脏工作区须显式 `-Candidate`。

新增 Inno Setup EXE 打包及独立安装验证。2026-10-02 本机当前用户安装、开始菜单、
卸载注册、安装后 HWND / 38 项 UIA / 公式与 Mermaid、同版本重装、卸载及文档保留通过。
此路线无需证书或 Store 身份，MSIX 安装、商店签名及 WACK 不作为当前 GitHub 发布的
结项条件。普通未签名 EXE 可能受到 SmartScreen、Smart App Control 或企业策略
限制；软件不修改用户安全设置。跨机器兼容性仍需独立验收。
完整证据与限制见 [GitHub 分发验收记录](windows-github-release-acceptance-20261002.md)。
