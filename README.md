# 羽 / Yu Editor

[![CI](https://github.com/xiaodou997/yu/actions/workflows/ci.yml/badge.svg)](https://github.com/xiaodou997/yu/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

Yu Editor 是一个开源、原生、Markdown-first 的桌面编辑器项目。项目以 Markdown 源码为
唯一持久化真源，通过增量语法、实时 Source Projection、自研编辑模型和 GPU 渲染，在
macOS、Windows 与 Linux 上提供低资源、低延迟的编辑体验。

macOS 是第一个产品级平台。共享编辑器内核使用 Rust；平台输入、窗口、Accessibility 等
能力允许使用 Swift、Objective-C 或其他适合该平台的语言实现。

> Yu 目前处于早期基础设施阶段，尚不能作为日常 Markdown 编辑器使用。

## 设计目标

- Markdown source 是唯一持久化真源，不通过富文本模型往返序列化；
- Lossless CST 保留 delimiter、空白、换行和未完成语法；
- task-list marker 保留 source range，视觉层只投影隐藏 `[ ]`/`[x]`，状态切换仍是普通 Transaction；
- 列表 Enter、空项 Backspace、缩进和反缩进都只通过 source Transaction 改写当前行；
- Undo/Redo 只保存有界 inverse Transaction，连续输入、删除和列表操作按 group 回放；
- Transaction、Snapshot、Anchor 与 Revision 构成统一编辑协议；
- selection/caret、Unicode grapheme command 与 Accessibility 查询共享同一个 Revision；
- macOS 原生快捷键先经过共享 Rust command route，普通字符仍交给 `NSTextInputClient`；
- 原生命令结果显式声明 `None/Range/Full` source sync，局部编辑只复制变化的 UTF-16 范围；
- `yu-editor` integration tests 使用 `EditorScenario` 标记 DSL 同时断言 source、caret/selection、
  Revision 和 composition overlay，新增编辑行为先固定可复现的行为契约；
- `yu-storage::DocumentSession` 统一 UTF-8 Markdown open/save/reload、BOM 元数据、Revision-bound
  dirty 和外部文件冲突检测；保存使用同目录临时文件加原子 rename，不覆盖外部修改；
- 打开软链接时保留用户可见路径，但读写、指纹和原子替换都指向 canonical target；macOS/Unix
  回归测试验证软链接本身不被替换且目标权限被保留；Windows replace semantics 仍待定义；
- `yu-storage::RecoveryStore` 提供调用方驱动的 autosave/recovery envelope；它只保存独立恢复候选，
  不自动覆盖目标文件，也不在共享核心内启动定时器；
- `yu-workspace::Workspace` 管理多个 `WorkspaceTab` 和唯一 active tab；每个 tab 只拥有一个
  `DocumentEditorSession`，重复打开、dirty close、save/discard/cancel 和外部冲突都在无窗口
  生命周期契约中处理；
- `yu-storage::FileWatchDebouncer` 与 `CloseStateMachine` 固定文件通知去抖、dirty close、取消/丢弃和
  外部冲突提示；macOS flag 适配不把 watcher 线程或 AppKit 对象带入共享核心；
- `yu-storage-ffi` 让 macOS 文档壳只消费 Rust-owned source snapshot、Revision/dirty 状态和 close
  结果；现在同一 handle 也承载 command、selection、key route、普通文本和 IME composition，
  `DocumentTextView` 只是可丢弃 native mirror，避免形成第二份 source；
- macOS 文档 host 的 copy/paste/cut/selectAll 已回到统一 session；copy/cut 同时发布 canonical
  Markdown UTI、纯文本和同一 source range 生成的语义 HTML，paste 优先保留 Markdown source；
- `yu-export` 固定 Revision-bound source selection 的 Markdown/纯文本/HTML clipboard payload，
  HTML 只消费当前 parser 已识别的语义，未识别语法按转义文本回退，不读取 TextKit mirror；
- macOS Accessibility 在现有文本快照之外提供 Revision-bound、source-backed Markdown semantic
  node count/fill 查询；Swift 将 owned 节点映射为实现 AppKit `NSAccessibilityElementProtocol` 的
  child，并提供 Heading/Link custom rotor、链接 URL 属性和 task checkbox press；文本、URL 和几何仍
  按节点 Revision 回查，不保存第二份文档；真正的外部链接打开策略和 VoiceOver 朗读仍需人工验收；
- `yu-workspace::ViewportFramePublisher` 把当前 `EditorDocument` 组装成带 Revision/serial 的
  owned publication，macOS host 只消费已验证的 publication；
- viewport render frame 通过不可变共享 handle 在 publisher cache、publication 和 macOS host
  之间传递，避免 scene/render plan 深拷贝；
- viewport publication 使用 staged `RenderPlanBuilder`，只有 frame、serial 和 cache 全部通过
  后才提交 atlas fingerprint，失败重试不会污染上传去重状态；
- IME preedit 通过 transient composition projection/layout 参与换行和 shaping，但不进入
  canonical source、Revision、缓存或 Undo；
- composition FFI 以 canonical Revision + transient generation 绑定 projected UTF-8、visual
  selection 和 caret，native mirror 不复制 Markdown parser；
- active composition 会以 transient CoreText shaped layout 进入同一 Rust RenderPlan、glyph atlas
  和持久 Metal surface；跨 block replacement 按受影响 block span 投影，首 block 承载 preedit，后续
  block 清除被替换 source，source/Revision 仍保持不变；
- macOS composition hit-test 通过 Revision + generation-bound transient projection 返回 block、
  document-space point、visual selection/replacement 与 source/visual UTF-16 round-trip；native host
  不复制跨 block preedit 偏移；
- visual scene/glyph/render-plan 的 count/fill header 同时绑定 composition generation；marked-text
  在两次 FFI 调用之间更新或取消时，Rust 拒绝 stale fill，避免旧容量与新 glyph 数据错配；
- opt-in visual mirror 额外消费 Rust generation-bound visual replacement range，让 marked-text
  preedit、`markedRange` 和 `attributedSubstring` 使用同一 visual 坐标；默认生产 view 仍走 source
  mirror，过期 generation 自动回退；
- macOS `NSTextInputClient` lifecycle 将 canonical replacement、当前 native marked range 和
  marked presentation 分开管理；`unmarkText` 不会误取消 Rust overlay，commit/cancel 均消费
  同一 generation-bound composition snapshot；
- macOS retained Metal 的 partial-damage frame 会在 native bridge 前按 command bounds 做
  backend-owned culling，保持 painter order 而减少无关命令编码；
- macOS `doCommand(by:)` 只允许 allowlist 内的 Selector 进入同一 Rust command/availability 入口；
- macOS Option/Control word movement 使用 Unicode word-boundary segment，不物化整份文档；
- 解析、投影、布局、绘制和资源加载都只处理受影响部分；
- 中文、日文、RTL、emoji、组合字符与原生 IME 是一等公民；
- 不依赖 Chromium、DOM 或常驻 JavaScript runtime；
- 大文件和高成本嵌入内容具备明确的预算与降级模式。

## 当前阶段

项目已完成 Phase 1、Phase 2 的主要 Contracts & Risk Spikes，当前进入 **Phase 3：Source Projection
& Native Layout**。这些阶段都不承诺完整 CommonMark 或产品 UI；它先固定最容易影响长期架构的契约：

- 强类型源码坐标、Revision 与稳定 Anchor；
- Snapshot、Transaction、ChangeSet 与可逆编辑；
- 引用源码范围的 lossless Markdown 结构；
- macOS `NSTextInputClient` 输入链路实验；
- 增量实现必须满足的等价性和源码保真不变量。

详细进度见 [Phase 1 路线](docs/roadmap/phase-1.md)。
当前存储/文档会话进度见 [Phase 2 路线](docs/roadmap/phase-2.md)。
当前 source projection/native layout 进度见 [Phase 3 路线](docs/roadmap/phase-3.md)。

## 仓库结构

```text
crates/yu-core          坐标、范围、Revision、Anchor
crates/yu-editor        EditorDocument、selection、commands、CompositionOverlay 和平台无关编辑状态
crates/yu-editor-ffi    原生平台调用的 CompositionOverlay 与 command C ABI static library
crates/yu-text          Snapshot、Transaction、Piece Tree 和候选文本存储
crates/yu-storage       UTF-8 Markdown 文档会话、BOM、原子保存和外部变更检测
crates/yu-storage-ffi   macOS 文档壳消费 DocumentSession 的窄 C ABI
platform/macos/yu-storage-macos macOS FSEvents/DispatchSource flag 适配与文件通知 debounce
crates/yu-markdown      lossless block/inline CST 与增量 Markdown parser
crates/yu-export        Revision-bound Markdown/纯文本/HTML clipboard payload exporter
crates/yu-projection    Source → Visual Markdown 投影
crates/yu-layout        block layout、caret/hit-test 和 viewport 高度索引
crates/yu-font          font fallback、GlyphRun、metrics/rasterization 契约与 CPU glyph atlas
crates/yu-scene         revision-bound retained primitives、viewport 与 damage tracking
crates/yu-render        backend-neutral render plan 与 atlas page upload boundary
crates/yu-workspace     EditorDocument → ViewportSceneInput → Scene → RenderPlan 集成层
platform/macos/yu-font-macos  macOS-only CoreText 字体目录、fallback、shaping 与 glyph rasterization 适配
platform/macos/yu-render-macos macOS-only Metal device、NSView layer attachment、clear/render plan frame、damage/scissor、pipeline 与 alpha atlas upload 适配
tools/yu-inspect        Markdown 结构检查 CLI
tools/yu-bench          可重复的第一阶段参考 workload
experiments/            可丢弃的平台风险实验
docs/                   架构规范、ADR 和阶段计划
```

`yu-text` 已选择持久化 Piece Tree 作为产品默认后端。平坦 UTF-8 后端继续作为正确性 oracle，
Persistent Rope 保留为实验对照；三者运行相同的 Transaction model tests。

## 获取源码

```bash
git clone git@github.com:xiaodou997/yu.git
cd yu
```

项目固定使用 Rust 1.97。macOS 输入实验还需要 Xcode/Swift 工具链。

## 本地验证

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p yu-editor --test editor_behavior
cargo test -p yu-render-macos -- --ignored  # 需要有 Metal device 的 macOS session
cargo run -p yu-inspect -- README.md
cargo run --release -p yu-bench -- --size-mib 1 --iterations 20 --random-edits 2000 --retained-snapshots 8
experiments/macos-text-input/build-rust-ffi.sh
swift build --package-path experiments/macos-text-input
experiments/macos-document-host/build-app.sh
```

macOS 输入实验的 Swift target 通过 `YuEditorFFI` C module 链接 Rust static library；因此必须
先运行 `build-rust-ffi.sh`，或直接使用会自动执行它的 `build-app.sh`。构建产物位于被忽略的
`experiments/macos-text-input/.rust/`，不会提交到仓库。

最小 macOS 文档 host 同样先构建 `yu-storage-ffi` static library，再由 Swift Package 链接；
它只验证产品壳生命周期，暂不提供可编辑文本或完整 Markdown 投影。构建产物位于被忽略的
`experiments/macos-document-host/.rust/` 和 `.build/`，不会提交到仓库。

## 文档

正式架构文档和代码一起进行版本管理，因为这些规范定义了模块边界和测试不变量：

- [架构总览](docs/architecture/overview.md)
- [Text Buffer](docs/architecture/text-buffer.md)
- [Markdown Parser](docs/architecture/markdown-parser.md)
- [核心不变量](docs/specs/invariants.md)
- [坐标与位置](docs/specs/coordinates.md)
- [Architecture Decision Records](docs/adr/)
- [macOS Metal surface boundary](docs/adr/0034-macos-metal-surface-boundary.md)
- [macOS clear frame lifecycle](docs/adr/0035-macos-clear-present-frame.md)
- [macOS retained Metal plan pipeline](docs/adr/0036-macos-retained-metal-plan-pipeline.md)
- [macOS AppKit attachment and damage frame](docs/adr/0037-macos-appkit-attachment-damage-frame.md)
- [macOS AppKit host probe](docs/adr/0038-macos-appkit-host-probe.md)
- [Markdown block CST v1](docs/adr/0039-markdown-block-cst-v1.md)
- [Markdown inline links and breaks](docs/adr/0040-markdown-inline-links-breaks.md)
- [Markdown line-break projection and layout](docs/adr/0041-markdown-line-break-projection-layout.md)
- [Markdown reference links and autolinks](docs/adr/0042-markdown-reference-links-autolinks.md)
- [Markdown reference definitions and shortcuts](docs/adr/0043-reference-definitions-shortcuts.md)
- [Markdown task-list projection](docs/adr/0044-task-list-projection.md)
- [Markdown list editing commands](docs/adr/0045-list-editing-commands.md)
- [Editor history and undo groups](docs/adr/0046-editor-history-and-undo-groups.md)
- [macOS key command routing](docs/adr/0047-macos-key-command-routing.md)
- [Native command source synchronization](docs/adr/0048-native-command-source-sync.md)
- [macOS Selector command bridge](docs/adr/0049-macos-selector-command-bridge.md)
- [Unicode word movement](docs/adr/0050-unicode-word-movement.md)
- [Vertical caret and preferred-X](docs/adr/0051-vertical-caret-preferred-x.md)
- [Shift vertical selection](docs/adr/0052-shift-vertical-selection.md)
- [Revision-bound caret scroll request](docs/adr/0053-caret-scroll-request.md)
- [macOS NSScrollView consumer](docs/adr/0054-macos-scrollview-consumer.md)
- [macOS NSScrollView host attachment](docs/adr/0055-macos-scrollview-host-attachment.md)
- [Viewport metrics FFI contract](docs/adr/0056-viewport-metrics-ffi-contract.md)
- [CoreText system UI viewport metrics](docs/adr/0057-coretext-system-ui-viewport-metrics.md)
- [macOS CoreText shaped line comparison](docs/adr/0058-macos-coretext-shaped-line-comparison.md)
- [Projection-aware shaped layout](docs/adr/0059-projection-aware-shaped-layout.md)
- [Revision-bound projection caret query](docs/adr/0060-revision-bound-projection-caret.md)
- [Block-local projection caret query](docs/adr/0061-block-local-projection-caret.md)
- [Block-local shaped caret geometry](docs/adr/0062-block-local-shaped-caret-geometry.md)
- [macOS shaped caret scroll request](docs/adr/0063-shaped-caret-scroll-request.md)
- [Shaped viewport block snapshot](docs/adr/0064-shaped-viewport-block-snapshot.md)
- [Viewport scene input](docs/adr/0065-viewport-scene-input.md)
- [Batched viewport scene assembly](docs/adr/0066-batched-viewport-scene-assembly.md)
- [Editor viewport scene integration](docs/adr/0067-editor-viewport-scene-integration.md)
- [Revision-aware viewport frame publication](docs/adr/0068-revision-aware-viewport-frame-publication.md)
- [macOS revision-aware Metal frame consumer](docs/adr/0069-macos-revision-aware-metal-frame-consumer.md)
- [macOS viewport frame submission](docs/adr/0070-macos-viewport-frame-submission.md)
- [macOS viewport host session](docs/adr/0071-macos-viewport-host-session.md)
- [Yu workspace viewport frame publisher](docs/adr/0072-yu-workspace-viewport-frame-publisher.md)
- [macOS command-level damage culling](docs/adr/0073-macos-command-level-damage-culling.md)
- [Shared viewport frame handle](docs/adr/0074-shared-viewport-frame-handle.md)
- [Atomic viewport publication](docs/adr/0075-atomic-viewport-publication.md)
- [Composition-aware projection/layout](docs/adr/0076-composition-aware-projection-layout.md)
- [Composition projection FFI](docs/adr/0077-composition-projection-ffi.md)
- [macOS NSTextInputClient composition lifecycle](docs/adr/0078-macos-nstextinputclient-composition-lifecycle.md)
- [Visual viewport scroll coordinate contract](docs/adr/0116-visual-viewport-scroll-coordinate.md)
- [Revision-bound visual scene snapshot bridge](docs/adr/0117-visual-scene-snapshot.md)
- [Shaped glyph RenderPlan publication](docs/adr/0118-visual-render-plan-publication.md)
- [CoreText to Metal frame preparation](docs/adr/0119-coretext-metal-frame-preparation.md)
- [macOS document host render lifecycle](docs/adr/0120-macos-render-host-lifecycle.md)
- [Revision-bound retained scene glyph bridge](docs/adr/0121-retained-scene-glyph-bridge.md)
- [macOS real surface submit self-check](docs/adr/0122-macos-real-surface-submit.md)
- [Persistent macOS native surface adapter](docs/adr/0123-persistent-macos-surface-adapter.md)
- [macOS product NSView surface lifecycle](docs/adr/0124-macos-product-surface-lifecycle.md)
- [macOS minimal visible RenderPlan projection](docs/adr/0125-macos-minimal-visible-render-plan.md)
- [macOS production visual pointer mapping](docs/adr/0126-macos-production-visual-pointer-mapping.md)
- [macOS projected selection and caret reveal](docs/adr/0127-macos-visual-selection-and-caret-reveal.md)
- [macOS shaped vertical editor command](docs/adr/0128-macos-shaped-vertical-command.md)
- [macOS CoreText-shaped pointer hit-test](docs/adr/0129-macos-shaped-pointer-hit-test.md)
- [macOS visual IME shaped caret geometry](docs/adr/0130-macos-visual-ime-shaped-caret.md)
- [macOS visual IME Metal preedit glyph publication](docs/adr/0131-macos-visual-ime-metal-preedit.md)
- [macOS visual count/fill composition generation guard](docs/adr/0132-macos-visual-count-fill-generation-guard.md)
- [macOS cross-block composition transient layout](docs/adr/0133-macos-cross-block-composition-layout.md)
- [macOS cross-block composition hit-test](docs/adr/0134-macos-cross-block-composition-hit-test.md)
- [macOS document-space RenderPlan viewport](docs/adr/0135-macos-document-space-render-viewport.md)
- [macOS visual decoration sibling](docs/adr/0136-macos-visual-decoration-sibling.md)
- [macOS Rust/CoreText-shaped decoration geometry](docs/adr/0137-macos-rust-shaped-decoration-geometry.md)
- [macOS visual selection anchor/focus](docs/adr/0138-macos-visual-selection-anchor-focus.md)
- [macOS primary Rust surface glyph gate](docs/adr/0139-macos-primary-rust-surface-glyph-gate.md)
- [macOS code block fill primitive](docs/adr/0140-macos-code-block-fill-primitive.md)
- [Editor behavior test DSL](docs/adr/0085-editor-behavior-test-dsl.md)
- [yu-storage document session](docs/adr/0086-yu-storage-document-session.md)
- [macOS file watch and close state](docs/adr/0087-macos-file-watch-close-state.md)
- [macOS minimal document host](docs/adr/0088-macos-document-host.md)
- [unified document editor session](docs/adr/0089-unified-document-editor-session.md)
- [macOS writable native mirror](docs/adr/0090-macos-writable-native-mirror.md)
- [source-backed Markdown Accessibility semantic tree](docs/adr/0101-source-backed-accessibility-semantic-tree.md)
- [macOS Accessibility semantic children](docs/adr/0102-macos-accessibility-semantic-children.md)
- [macOS Accessibility custom rotors](docs/adr/0103-macos-accessibility-custom-rotors.md)
- [macOS Accessibility semantic actions](docs/adr/0104-macos-accessibility-semantic-actions.md)
- [Phase 1 路线](docs/roadmap/phase-1.md)
- [Phase 2 路线](docs/roadmap/phase-2.md)
- [macOS IME 实测](docs/experiments/macos-ime-2026-08-09.md)
- [macOS IME 人工验收模板](docs/experiments/macos-ime-manual-acceptance-2026-08-13.md)
- [DocumentEditorSession headless benchmark](docs/experiments/yu-session-benchmark-2026-08-14.md)
- [macOS CompositionOverlay FFI 实验](docs/experiments/macos-composition-ffi-2026-08-10.md)
- [文本存储候选对比](docs/experiments/storage-candidates-2026-08-09.md)
- [增量 Markdown 实验](docs/experiments/incremental-markdown-2026-08-09.md)

个人笔记、临时调研和未整理草稿请放在本地 `.notes/`，该目录不会提交。

## 贡献

Yu 尚处在协议和基础数据结构快速演进期。提交实现前，请先阅读架构不变量与相关 ADR；新增
编辑行为应同时提供行为测试，增量算法应提供与完整算法的等价性验证。

## License

Yu Editor 使用 [Apache License 2.0](LICENSE) 发布。该许可证允许使用、修改、分发和商业
集成，并包含明确的专利授权；分发时需要保留许可证及适用的版权和归属声明。
