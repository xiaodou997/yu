# 羽 / Yu Editor

[![CI](https://github.com/xiaodou997/yu/actions/workflows/ci.yml/badge.svg)](https://github.com/xiaodou997/yu/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

Yu Editor 是一个开源、原生、Markdown-first 的桌面编辑器项目。项目以 Markdown 源码为
唯一持久化真源，通过增量语法、实时 Source Projection、自研编辑模型和 GPU 渲染，在
macOS、Windows 与 Linux 上提供低资源、低延迟的编辑体验。

macOS 是第一个产品级平台。共享编辑器内核使用 Rust；平台输入、窗口、Accessibility 等
能力允许使用 Swift、Objective-C 或其他适合该平台的语言实现。

> **状态：v2 架构重构中。** 当前不能作为日常 Markdown 编辑器使用。
> v1 已在 tag `v1-final` 冻结，完整状态保留在分支 `archive/v1-source-projection`。
> 重构依据见 [架构总览 v2](docs/architecture/overview-v2.md)。

## 设计目标

Yu 的技术本质是：

> 一个 Markdown 源码编辑器内核 + 一个实时 Source Projection 系统
> + 一个增量原生布局引擎 + 一个 retained GPU renderer
> + 少量平台原生输入与窗口适配。

它**不是** WebView Markdown 编辑器、富文本编辑器、HTML 编辑器或 Markdown 格式转换器。

不可破坏的原则（完整表述见[核心不变量](docs/specs/invariants.md)）：

1. Markdown source 永远是唯一真源，不通过富文本模型往返序列化；
2. 所有永久修改都经过 Transaction；
3. 视觉表现的唯一来源是 Decoration，Markdown 语义只存在于 `yu-markdown` 一个 crate；
4. 所有派生数据都绑定 Revision，过期结果整体拒绝；
5. IME composition 永远是 transient overlay；
6. 平台层不解析 Markdown；
7. 缓存、GPU 和异步资源不能改变编辑语义；
8. 只处理发生变化和当前可见的内容；
9. 不存在第二条渲染路径——Rust 渲染器是唯一渲染器；
10. crate 依赖图必须是严格 DAG，由 CI 强制。

中文、日文、RTL、emoji、组合字符与原生 IME 是一等公民。不依赖 Chromium、DOM
或常驻 JavaScript runtime。

## 当前阶段

v2 重构分 7 个阶段推进，每个阶段结束时 app 必须可运行、CI 必须全绿。
阶段定义与验收标准见[架构总览 v2 第 8 节](docs/architecture/overview-v2.md)。

| 阶段 | 内容 | 状态 |
| --- | --- | --- |
| S1 | 拆炸弹：删除 TextKit fallback 与诊断桥，帧调度移入 Rust，app 转正 | 进行中 |
| S2 | 地基：坐标收敛、`yu-text` 换 ropey、CI 强制依赖方向 | 未开始 |
| S3 | 解析器：移植 lezer-markdown 算法，建立 CommonMark spec 差分测试 | 未开始 |
| S4 | 中枢：`yu-decoration`（RangeSet + Decoration）与 `yu-state` | 未开始 |
| S5 | 布局重写：UAX #14 断行、UAX #9 bidi、widget 盒模型 | 未开始 |
| S6 | 语义 extension 化：每种语法收敛为一个 extension | 未开始 |
| S7 | 产品面：搜索、大纲、多光标、代码高亮、导出、第二平台 | 未开始 |

## 仓库结构

目标形态。依赖方向严格单向，反向依赖是 CI 失败。

```text
crates/yu-core          坐标、Revision、Anchor
crates/yu-text          Rope（ropey）、Snapshot、Transaction 原语
crates/yu-syntax        增量 CST：block/inline 两级、fragment 复用、精确 range
crates/yu-markdown      ★ Markdown 语法 extension 集合（Markdown 只存在于这一层）
crates/yu-state         EditorState、Transaction 应用、History、Facet
crates/yu-decoration    ★ RangeSet<Decoration>、source↔visual 映射
crates/yu-layout        行盒、widget 盒、UAX#14 断行、UAX#9 bidi、hit-test
crates/yu-scene         retained primitives 与 damage 追踪
crates/yu-render        后端中立 RenderPlan：Glyph / FillRect / Texture / Quad
crates/yu-font          字体解析、shaping、栅格化契约（只依赖 yu-core）
crates/yu-assets        图片/嵌入资源的异步调度、LRU 与内存预算
crates/yu-storage       UTF-8 Markdown 文档会话、原子保存、外部变更检测
crates/yu-workspace     tab 与 document session 生命周期
crates/yu-export        Revision-bound 剪贴板与 HTML 导出（comrak）
platform/macos/yu-font-macos    CoreText 字体目录、fallback、shaping、栅格化
platform/macos/yu-render-macos  Metal device、CAMetalLayer、render plan 编码
platform/macos/yu-storage-macos FSEvents 文件通知适配
platform/macos/yu-shell-macos   Swift 产品壳：NSWindow / 菜单 /
                                NSTextInputClient / Accessibility
tools/yu-inspect        Markdown 结构检查 CLI
tools/yu-bench          可重复的参考 workload
```

旁路依赖（不在主链路上）：`tree-sitter` 仅用于 fenced code block 内部的代码高亮；
`comrak` 仅用于 HTML 导出与 CommonMark spec 差分测试。

## 获取源码

```bash
git clone git@github.com:xiaodou997/yu.git
cd yu
```

项目固定使用 Rust 1.97。构建 macOS 产品壳还需要 Xcode/Swift 工具链。

## 本地验证

```bash
# 全量验证：fmt / clippy / test / FFI 头文件一致性 / 产品壳 self-check
tools/verify.sh
tools/verify.sh --rust-only     # 只跑 Rust 检查
tools/verify.sh --clean         # 产品壳用干净构建（改动 FFI 边界后必须）

# 构建并（重新）启动 Yu.app
platform/macos/yu-shell-macos/run-app.sh README.md

# 其它
cargo test -p yu-render-macos -- --ignored       # 需要有 Metal device 的 macOS session
cargo run -p yu-inspect -- README.md
cargo run --release -p yu-bench -- --size-mib 1 --iterations 20 --random-edits 2000 --retained-snapshots 8
```

`run-app.sh` 会先终止已在运行的实例：macOS 的 `open` 对运行中的 app 只会把它
带到前台、不会加载新二进制，直接 `open` 会让已修好的 bug 看起来仍在复现。

`verify.sh` 存在的理由是手敲验证命令容易漏。例如
`cargo test --workspace | grep "^test result: ok" | awk '{s+=$4}'` 会跳过
`test result: FAILED` 的行——失败被静默吞掉，还显示出一个看起来正常的用例数。
脚本以退出码为准，任一步失败立即中止。

Swift 产品壳通过 `YuStorageFFI` C module 链接 Rust static library，因此必须先运行
`build-rust-ffi.sh`，或使用会自动执行它的 `build-app.sh` / `run-self-checks.sh --build`。
构建产物位于被忽略的 `.rust/` 与 `.build/`，不会提交到仓库。

> **改动 FFI 边界后请用 `run-self-checks.sh --clean-build`。** SwiftPM 的增量构建
> 可能不会重编引用已删类型的文件，本地看到「构建通过」而 CI 的干净检出会失败。

## 文档

先读这两份，它们优先于代码和一切历史文档：

- **[架构总览 v2](docs/architecture/overview-v2.md)** — 分层、依赖方向、组件决策、迭代阶段
- **[核心不变量](docs/specs/invariants.md)** — 任何实现都不得违反的约束

其他：

- [坐标与位置](docs/specs/coordinates.md)
- [ADR 规范](docs/adr/README.md) — 编号从 0001 重新开始
- [v1 归档](docs/archive-v1/README.md) — 183 篇 v1 ADR 与设计文档，全部 superseded

v1 时期的风险实验记录已随其他 v1 文档归档到
[`docs/archive-v1/experiments/`](docs/archive-v1/experiments/)，其中的命令路径
反映当时的目录结构。

个人笔记、临时调研和未整理草稿请放在本地 `.notes/`，该目录不会提交。

## 贡献

Yu 正在进行 v2 架构重构，协议和基础数据结构快速演进。提交实现前请先阅读
[架构总览 v2](docs/architecture/overview-v2.md) 与[核心不变量](docs/specs/invariants.md)。

- 新增编辑行为必须同时提供行为测试；
- 增量算法必须提供与完整算法的等价性验证；
- 违反不变量的改动会被拒绝，即使功能正确；
- 不要为「实现了某功能」新增 ADR，规则见 [ADR 规范](docs/adr/README.md)。

## License

Yu Editor 使用 [Apache License 2.0](LICENSE) 发布。该许可证允许使用、修改、分发和商业
集成，并包含明确的专利授权；分发时需要保留许可证及适用的版权和归属声明。
