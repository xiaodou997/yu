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
- macOS `doCommand(by:)` 只允许 allowlist 内的 Selector 进入同一 Rust command/availability 入口；
- macOS Option/Control word movement 使用 Unicode word-boundary segment，不物化整份文档；
- 解析、投影、布局、绘制和资源加载都只处理受影响部分；
- 中文、日文、RTL、emoji、组合字符与原生 IME 是一等公民；
- 不依赖 Chromium、DOM 或常驻 JavaScript runtime；
- 大文件和高成本嵌入内容具备明确的预算与降级模式。

## 当前阶段

项目正在进行 **Phase 1：Contracts & Risk Spikes**。这一阶段不承诺完整 CommonMark 或
产品 UI；它先固定最容易影响长期架构的契约：

- 强类型源码坐标、Revision 与稳定 Anchor；
- Snapshot、Transaction、ChangeSet 与可逆编辑；
- 引用源码范围的 lossless Markdown 结构；
- macOS `NSTextInputClient` 输入链路实验；
- 增量实现必须满足的等价性和源码保真不变量。

详细进度见 [Phase 1 路线](docs/roadmap/phase-1.md)。

## 仓库结构

```text
crates/yu-core          坐标、范围、Revision、Anchor
crates/yu-editor        EditorDocument、selection、commands、CompositionOverlay 和平台无关编辑状态
crates/yu-editor-ffi    原生平台调用的 CompositionOverlay 与 command C ABI static library
crates/yu-text          Snapshot、Transaction、Piece Tree 和候选文本存储
crates/yu-markdown      lossless block/inline CST 与增量 Markdown parser
crates/yu-projection    Source → Visual Markdown 投影
crates/yu-layout        block layout、caret/hit-test 和 viewport 高度索引
crates/yu-font          font fallback、GlyphRun、metrics/rasterization 契约与 CPU glyph atlas
crates/yu-scene         revision-bound retained primitives、viewport 与 damage tracking
crates/yu-render        backend-neutral render plan 与 atlas page upload boundary
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
cargo test -p yu-render-macos -- --ignored  # 需要有 Metal device 的 macOS session
cargo run -p yu-inspect -- README.md
cargo run --release -p yu-bench -- --size-mib 1 --iterations 20 --random-edits 2000 --retained-snapshots 8
experiments/macos-text-input/build-rust-ffi.sh
swift build --package-path experiments/macos-text-input
```

macOS 输入实验的 Swift target 通过 `YuEditorFFI` C module 链接 Rust static library；因此必须
先运行 `build-rust-ffi.sh`，或直接使用会自动执行它的 `build-app.sh`。构建产物位于被忽略的
`experiments/macos-text-input/.rust/`，不会提交到仓库。

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
- [Phase 1 路线](docs/roadmap/phase-1.md)
- [macOS IME 实测](docs/experiments/macos-ime-2026-08-09.md)
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
