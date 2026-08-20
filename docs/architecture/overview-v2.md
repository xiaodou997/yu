# Yu 架构总览 v2

## 0. 这份文档的位置

本文取代 `docs/architecture/overview.md`（v1）。v1 文档与 `docs/adr/` 下的 183 篇 ADR
描述的是已被取代的架构，仅作历史参考。

| 项 | 值 |
| --- | --- |
| 分歧点 commit | `e8140be` |
| 分歧点 tag | `v1-final` |
| v1 完整状态 | 分支 `archive/v1-source-projection` |
| v2 演进分支 | `main` |

v1 分支冻结，不再接收提交。所有后续开发在 `main` 上按本文推进，允许破坏式重构，
不保留中间过渡形态，不考虑与 v1 的接口兼容。

---

## 1. 不变的部分

v2 不是推翻重来。v1 的产品模型与编辑协议是正确的，全部保留：

1. **Markdown source 是唯一持久化真源。** 不通过富文本模型往返序列化，
   `__hello__` 不会在保存后变成 `**hello**`。
2. **编辑器是实时 Source Projection，不是 Markdown → 富文本编辑器。**
   用户编辑的始终是原始 Markdown 字节，视觉表现只是源码的一种投影。
3. **所有永久修改经过 Transaction。** Transaction / ChangeSet / Revision / Anchor
   构成统一编辑协议，Undo/Redo、dirty、增量解析、保存共用同一个修改协议。
4. **所有派生数据绑定 Revision。** 过期结果整体拒绝，缓存只能提高性能，
   不能改变文档语义。
5. **IME composition 永远是 transient overlay。** 不写入 canonical source、
   不推进 Revision、不进入 Undo。
6. **只处理发生变化和当前可见的内容。**
7. **平台层不解析 Markdown。**

这七条在 v2 中原样成立，并在 `docs/specs/invariants.md` 中重新表述。

---

## 2. v2 的动因

v1 在 11 天内产出 215 个 commit、约 62,000 行 Rust、183 篇 ADR，产出速度不是问题。
问题是**每个功能的边际成本在持续上升**。根因有四条，都是可定点修复的结构性问题。

### 2.1 Markdown 语义泄漏到渲染层

v1 的类型分布暴露了这一点：

```text
yu-projection:  HeadingPresentation  BlockQuotePresentation  TableProjection  CodeProjection
yu-scene:       TablePrimitive  BlockQuotePrimitive  TaskCheckboxPrimitive  EditorDecorationPrimitive
```

Markdown 的语义一路穿透到 Scene。因此新增一种语法需要改动 7 处：

```text
CST 新节点 → Projection 新类型 → Layout 新分支 → Scene 新 Primitive
→ RenderCommand 新 tag → FFI count/fill 新函数对 → Swift capability mask + coverage 分支
```

v1 自己的不变量「渲染层不认识 Markdown」已经被破坏。

### 2.2 双渲染路径（TextKit fail-closed fallback）

v1 要求 Rust surface 对当前 viewport 拥有**完整 retained coverage**，否则整页回退
TextKit 绘制。代价是每个新 primitive 都必须回答一次「画不出来时谁兜底」，于是产生了
capability mask、block kind mask、coverage 查询、fallback reason、visual render state machine
这一整套机制。Phase 3 roadmap 中约半数条目在维护这套机制本身，而不是在增加编辑能力。

### 2.3 渲染循环在 Swift 侧，导致 FFI 膨胀

因为 RenderPlan 必须跨 C ABI 交给 Swift 绘制，每类 primitive 都需要一对 count/fill 函数：

| 文件 | 行数 |
| --- | --- |
| `crates/yu-storage-ffi/src/lib.rs` | **14,340**（单文件） |
| `experiments/.../YuMacDocumentHost/main.swift` | **11,355**（单文件） |
| `crates/yu-markdown`（整个解析器） | 6,026 |

25,700 行胶水对 6,000 行解析器。且产品壳至今仍在 `experiments/` 目录下。

### 2.4 分层已经反向依赖

```toml
# crates/yu-font/Cargo.toml
yu-layout     = { path = "../yu-layout" }
yu-projection = { path = "../yu-projection" }
```

字体层依赖布局层与投影层。这个方向一旦成立，分层边界就不再有约束力。

### 2.5 附带发现的质量缺口

- `yu-layout` 只依赖 `unicode-segmentation`，**换行是按 grapheme 硬折**，
  没有 UAX #14 断行机会、没有 CJK 禁则，英文单词会被从中间切断。
- 没有 `unicode-bidi`，README 承诺的 RTL 支持实际不存在。
- `BlockKind` 只有 8 种，缺 setext heading、thematic break、HTML block、indented code、
  脚注、frontmatter；列表与引用是扁平的 `depth: u8`，没有真正的嵌套树。

---

## 3. v2 的核心变更

一句话：

> **把「Markdown」这个词从 6 个 crate 里删掉，只留在 1 个 crate 里。**

实现手段是引入一个中枢抽象取代 v1 的 `yu-projection`：

```text
v1:  每种 Markdown 语法 →  一个 Projection 类型 + 一个 Scene Primitive + 一条 FFI 通道
v2:  每种 Markdown 语法 →  一组 Decoration（数据），进入同一个 RangeSet
```

对照：

| 功能 | v1 | v2 |
| --- | --- | --- |
| 隐藏未聚焦的 `##` | `HeadingPresentation` + layout 分支 | `Decoration::Replace(0..3)` |
| task checkbox | Primitive + RenderCommand tag + Swift mask | `Decoration::Widget(Checkbox)` |
| 表格 | `TableProjection` + `TablePrimitive` + 几何 FFI | 一个 block widget |
| KaTeX / Mermaid | 专用 EmbeddedSvg 全链路 | 一个 block widget |
| 语法高亮 / 拼写检查 / AI diff | 各自一条全链路 | 各自一个 extension 产 RangeSet |

---

## 4. 目标架构

### 4.1 分层

```text
                    ┌──────────────────────────────────────────────┐
   平台专属          │  yu-shell-macos (Swift, 目标 < 2000 行)       │
   每端一份          │    NSWindow / 菜单 / NSTextInputClient / AX   │
                    │  yu-app-macos (Rust)                         │
                    │    持有 CAMetalLayer 与渲染循环，主线程 affinity│
                    │  yu-font-macos (CoreText shaping + 栅格化)    │
                    │  yu-render-macos (Metal 后端)                 │
                    └───────────────────┬──────────────────────────┘
                                        │ Rust → Rust，无 C ABI
   ┌────────────────────────────────────▼───────────────────────────────────┐
   │ yu-render     RenderPlan：Glyph / FillRect / Texture / Quad（永久冻结）  │
   │ yu-scene      retained primitives + damage 追踪                         │
   │ yu-layout     行盒 / widget 盒 / UAX#14 断行 / UAX#9 bidi / hit-test    │
   │               输入只有 StyledRun + Widget + LineStyle                   │
   ├────────────────────────────────────────────────────────────────────────┤
   │ yu-decoration ★ RangeSet<Decoration>                                    │
   │               Mark / Replace / Widget / Line，source↔visual 映射         │
   │ yu-state      EditorState{doc, selection, syntax, facets, decorations}  │
   │               Transaction / ChangeSet / Anchor / Revision / History     │
   ├────────────────────────────────────────────────────────────────────────┤
   │ yu-markdown   ★ extension 集合                                          │
   │               每种语法 = BlockParser/InlineParser + decoration 产出器    │
   │               **Markdown 只存在于这一层**                                │
   │ yu-syntax     增量 CST：block/inline 两级、fragment 复用、精确 range     │
   ├────────────────────────────────────────────────────────────────────────┤
   │ yu-text       ropey::Rope + Snapshot / Revision 包装                    │
   │ yu-core       ByteOffset / TextRange / Revision / Anchor                │
   └────────────────────────────────────────────────────────────────────────┘

   旁路（不在主链路上）：
     tree-sitter → 仅用于 fenced code block 内部的代码高亮
     comrak      → 仅用于 HTML 导出 + CommonMark spec 差分测试
```

### 4.2 依赖方向

严格 DAG，箭头不允许反向，由 CI 强制：

```text
yu-core → yu-text → yu-syntax → yu-markdown → yu-state → yu-decoration
        → yu-layout → yu-scene → yu-render → yu-font → platform
```

`yu-font` 只能依赖 `yu-core`。它提供的是「给我一段文本和字体，返回 glyph 与 advance」，
它不需要、也不允许知道 layout 与 projection 的存在。

### 4.3 每层的职责与禁止事项

| 层 | 职责 | 明确禁止 |
| --- | --- | --- |
| `yu-core` | 坐标、Revision、Anchor | 依赖任何其他 crate |
| `yu-text` | Rope、Snapshot、Transaction 原语 | 泄漏 ropey 的 char index |
| `yu-syntax` | 增量解析、CST、fragment 复用 | 知道任何视觉概念 |
| `yu-markdown` | Markdown 语法定义与 decoration 产出 | 直接构造 Scene/Render 对象 |
| `yu-state` | EditorState、Transaction 应用、History、Facet | 知道具体语言 |
| `yu-decoration` | RangeSet、Decoration、map(changes)、source↔visual | 知道 Markdown |
| `yu-layout` | 断行、bidi、widget 盒、hit-test | 出现 Markdown 词汇 |
| `yu-scene` | retained primitive、damage | 出现 Markdown 词汇 |
| `yu-render` | 后端中立的绘制指令 | 新增语法专属指令 |
| `yu-font` | 字体解析、shaping、栅格化 | 断行、行布局 |
| platform | 窗口、输入、AX、GPU surface | 解析 Markdown、推导 source range |

---

## 5. 中枢：Decoration 与 RangeSet

这是 v2 唯一值得从零精心实现的数据结构，设计参考 CodeMirror 6 的 `RangeSet`/`Decoration`。
选它而不是 Helix 的 `text_annotations` 的原因：Helix 的模型是终端固定网格、grapheme 级的，
不支持任意 intrinsic size 的 widget，也不支持样式区间与替换区间的正交组合。

### 5.1 类型草案

```rust
pub enum Decoration {
    /// 不改变字符数量，只改变呈现样式。可叠加。
    Mark { style: StyleId },
    /// 从视觉上移除这段 source 字符。source 不变。
    /// 这是「隐藏未聚焦的 Markdown 语法」的唯一机制。
    Replace,
    /// 在该 range 位置放置一个视觉物件。range 为空则是插入，
    /// 非空则同时隐藏被覆盖的 source。
    Widget { widget: WidgetId, side: WidgetSide },
    /// 作用于整行/整块的样式（缩进、背景、行高、前缀装饰）。
    Line { style: LineStyleId },
}

pub struct DecorationSet(RangeSet<Decoration>);
```

### 5.2 必须满足的性质

1. **不可变 + 结构共享。** 与 Revision 绑定，旧 set 可安全并发读取。
2. **`map(&ChangeSet) -> DecorationSet`。** 随 Transaction 迁移，边界 bias 显式。
3. **带 summary 的平衡树。** 每个节点携带 `(source_len, visual_len)`，
   支持 O(log n) 的 source offset ↔ visual offset 双向映射。
   这是投影映射链的实现基础，取代 v1 的 `ProjectionMap`。
4. **分层合并。** 多个 extension 各自产出 `DecorationSet`，
   合并时保持确定性顺序（按 `(from, side, priority)`），不同 extension 之间不感知彼此。
5. **区间查询 O(log n + k)。** viewport 只取可见范围内的 decoration。

### 5.3 Widget 与异步资源

Widget 的 intrinsic size 可能依赖异步资源（图片解码、Math 渲染）。约定：

- Layout 通过 `WidgetRegistry` 查询 `measure(widget_id, constraints) -> Size`；
- 资源未就绪时返回 **placeholder size**，layout 正常完成，不阻塞；
- 资源就绪后发布一次 Revision-bound 通知，触发受影响 block 的重新 layout；
- 资源失败有有界退避重试，并保留可编辑的源码回退呈现。

图片、Math、Mermaid、表格、任务框在 v2 中全部是 Widget，走同一条通道。

---

## 6. 组件决策

对每一项都给出「引入了什么问题」，因为这是选型的真实成本。

### 6.1 决策表

| 组件 | v1 | v2 决策 | 性质 |
| --- | --- | --- | --- |
| 文本存储 | 自研 Piece Tree + Rope + Flat | **采用 `ropey`** | 采用 |
| Markdown 解析 | 自研 block/inline parser | **移植 lezer-markdown 算法** | 移植 |
| 正确性 oracle | 自研等价性测试 | **comrak + CommonMark spec 652 用例差分** | 采用 |
| 编辑状态 | 自研 EditorDocument | 自研，参考 CM6 State/Facet + Helix Transaction | 自研 |
| 投影/装饰 | 自研 Projection 类型族 | **自研 RangeSet\<Decoration\>**，参考 CM6 | 自研 |
| 断行 / bidi / 分词 | grapheme 硬折 | **`unicode-linebreak` + `unicode-bidi` + `icu_segmenter`** | 采用 |
| shaping / 栅格化 | CoreText | 平台原生（CoreText / DirectWrite / HarfBuzz） | 保留 |
| 渲染 | Metal（循环在 Swift） | 平台原生，**渲染循环移入 Rust** | 重构 |
| 代码块高亮 | 无 | **tree-sitter**（仅用于 fenced code 内部） | 采用 |
| HTML 导出 | 自研 exporter | **comrak** | 采用 |

### 6.2 为什么不用 tree-sitter-markdown

这是 v2 讨论中被否决的一个方案，理由必须记录，避免日后重复讨论。

tree-sitter-markdown 的官方 README 明确声明：

> "there are still lots of inaccuracies in the output... **it is not recommended to
> use this parser where correctness is important**"
>
> "The main goal for this parser is to provide syntactical information for
> **syntax highlighting** in editors such as neovim and helix."

它是为**语法高亮**设计的。对 Helix/Neovim，emphasis 判断错只是颜色不对；对 Yu，
parse 错意味着**投影错、隐藏错、编辑落到错误的 source range**，直接破坏第 1 条不变量。

另有一条硬伤直接冲突于增量目标：

> "the grammar is based on the assumption that link label matchings will never fail...
> which causes it hard to do incremental parsing without this assumption"

**结论：tree-sitter 保留，但只用在 fenced code block 内部的代码高亮。**
那里高亮错误无害，而 tree-sitter 的多语言 grammar 生态正是最大优势。

### 6.3 为什么是 lezer-markdown

`@lezer/markdown`（MIT）是 CodeMirror 6 的 Markdown 解析器，**Obsidian 的 Live Preview
建立在它之上**——与 Yu 属于同一产品品类，且经过大规模生产验证。

| 特性 | 是否满足 |
| --- | --- |
| 为编辑器内增量解析设计（fragment 复用） | 是 |
| CommonMark 语义正确（仅一处已声明偏差） | 是 |
| 精确 document-relative position | 是 |
| Block parser / Inline parser / Extension 三层扩展机制 | 是 |
| 单遍、不回溯 | 是 |

采取**移植**而非依赖：把算法用 Rust 重新实现（预计 3,000–5,000 行），
得到的是被验证过的算法与扩展机制，而不是一个不可控的外部黑盒。

### 6.4 每项决策引入的问题与对策

**采用 ropey**

| 问题 | 对策 |
| --- | --- |
| ropey 主索引是 char，Yu 是 byte，混用是真实 bug 源 | `yu-text` 包一层只暴露 `ByteOffset`，ropey 类型禁止逃逸出 crate，CI 检查 |
| 丢失 Piece Tree 的「原文件区零拷贝」，打开时有一次 chunk 重建 | 可接受；必要时用 `Rope::from_reader` 流式加载 |
| 全内存，不支持 mmap / GB 级文件 | 百万行以内无问题；GB 级文件另立方案，不在 v2 范围 |
| ropey 不提供 Revision / Anchor / History | 本就由 `yu-text` / `yu-state` 提供，与 Helix 做法一致 |

**移植 lezer-markdown**

| 问题 | 对策 |
| --- | --- |
| 不是零成本采用，是 3,000–5,000 行移植，且需连带设计 Tree/TreeFragment 的 Rust 版 | lezer 用 uint16 扁平数组编码，Rust 版改为 arena + 索引，更符合语言习惯 |
| lezer **不校验 link reference**，`[a][b]` 即使无定义也会被解析为链接 | **在 Yu 中修正**：parser 只产出「候选引用链接」，是否成立由文档级 reference table（增量维护的 facet）在 decoration 阶段决定。修正后比 lezer 更正确 |
| lezer 的树有 gap，未命名字符不进树，不是字面意义的 lossless CST | 因 position 精确，gap 可推导。**不变量中「lossless」重新定义为「source range 完备可推导」而非「每字节都有节点」** |
| 上游仓库 2026-04 已归档并迁至自托管 | 因为是移植不是依赖，反而不构成风险 |

**放弃 tree-sitter 解析 Markdown**

| 问题 | 对策 |
| --- | --- |
| 失去 tree-sitter query 语言带来的高亮便利 | 代码块高亮仍用 tree-sitter；Markdown 本身的高亮由 decoration 直接产出，本来就更精确 |

**自研 RangeSet**

| 问题 | 对策 |
| --- | --- |
| 无现成 Rust 实现，需自研约 1,000–1,500 行 | 这是编辑器中枢，值得投入 |
| `map(changes)` 的边界 bias 极易出错 | proptest property-based 测试；与既有 Anchor 语义对齐并交叉验证 |

**渲染循环移入 Rust**

| 问题 | 对策 |
| --- | --- |
| Rust 操作 `CAMetalLayer` / `NSView` 需要 objc2 或沿用现有 ObjC bridge | 优先沿用已验证的 `metal_bridge.m`，objc2 迁移作为后续独立议题 |
| `NSTextInputClient` 必须在 Swift/ObjC 侧实现 | 保留窄 FFI，边界重定义为：传入输入事件，传出候选窗矩形，不传渲染数据 |
| Metal 提交与 NSView 生命周期均在主线程 | Rust 侧显式建模 thread affinity，非 `Send` 类型标注清楚 |

**删除 TextKit fallback**

| 问题 | 对策 |
| --- | --- |
| 短期内未支持的语法会以源码原样显示，而非回退到 AppKit 排版 | 这是有意的取舍。未支持语法按**普通段落源码文本**绘制，永不白屏、永远可编辑，且不再需要第二个渲染器 |

---

## 7. 现有资产处置

| crate | v1 行数 | 处置 |
| --- | --- | --- |
| `yu-core` | 332 | **保留**，收敛坐标类型 |
| `yu-text` | 3,201 | **改造**：底层换 ropey，保留 Snapshot/Transaction 包装 |
| `yu-markdown` | 6,026 | **重写**：拆为 `yu-syntax`（lezer 移植）+ `yu-markdown`（extensions） |
| `yu-projection` | 3,775 | **删除**：由 `yu-decoration` 取代 |
| `yu-editor` | 9,127 | **拆分**：编辑状态入 `yu-state`，语法相关行为入 `yu-markdown` extension |
| `yu-layout` | 4,559 | **重写**：只接受 StyledRun/Widget/LineStyle，补 UAX #14 / #9 |
| `yu-scene` | 2,003 | **精简**：移除全部 Markdown 专属 primitive |
| `yu-render` | 1,356 | **保留**：`RenderCommand` 已足够干净，收敛 `EmbeddedSvg` 为通用 Texture |
| `yu-font` | 1,650 | **保留**：修正反向依赖，只依赖 `yu-core` |
| `yu-assets` | 2,385 | **保留**：接入 WidgetRegistry |
| `yu-embedded-math` | 694 | **保留**：改造为 Widget extension |
| `yu-storage` | 2,807 | **保留**：`DocumentSession` 是高质量资产，原子保存/冲突检测继续有效 |
| `yu-workspace` | 3,313 | **精简**：保留 tab/session 生命周期，移除 publication 拼装逻辑 |
| `yu-export` | 1,849 | **改造**：HTML 生成改用 comrak，保留 Revision-bound 剪贴板契约 |
| `yu-editor-ffi` | 4,694 | **删除**：渲染循环入 Rust 后不再需要 |
| `yu-storage-ffi` | 14,340 | **重写**：目标 < 1,000 行，只保留输入/文件/AX 窄接口 |
| `platform/macos/*` | — | **保留演进** |
| `experiments/macos-document-host` | 11,355 (Swift) | **重写转正**为 `platform/macos/`，Swift 目标 < 2,000 行 |

v1 中被保留的高价值资产：Transaction/Revision/Anchor 编辑协议、`DocumentSession`
的原子保存与外部冲突检测、CoreText shaping 与 Metal retained 渲染的已验证路径、
IME composition 的 generation 模型、`EditorScenario` 标记 DSL 测试方法。

---

## 8. 迭代阶段

原则：**先拆炸弹，再自底向上重建。** 每个阶段结束时 app 必须能运行，CI 必须全绿。

### S1 · 拆炸弹（纯删除，不引入新设计）

渲染循环移入 Rust；删除 TextKit fallback、capability mask、coverage gate、
count/fill FFI；app 从 `experiments/` 转正为 `platform/macos/`。

- 验收：`yu-storage-ffi` < 1,000 行；Swift < 2,000 行；总代码量下降 ≥ 20,000 行；
  heading / emphasis / code / link / image / table / task / IME / AX 全部保持可用。

### S2 · 地基

`yu-core` 坐标类型收敛；`yu-text` 换 ropey；CI 强制依赖方向，
`yu-font` 的反向依赖必须消失。

- 验收：依赖图为严格 DAG 且方向正确；ropey 的 char index 不泄漏出 `yu-text`。

### S3 · 解析器

移植 lezer-markdown 算法为 `yu-syntax`；建立 CommonMark spec 652 用例 +
comrak 差分测试 + fuzz 差分。

- 验收：spec 通过率 ≥ 99%，每条偏差记入 invariants；
  单字符编辑的重解析范围有量化上界并有 bench 守护。

### S4 · 中枢

实现 `yu-decoration`（RangeSet + Decoration + map + source↔visual 映射）；
`yu-state` 收敛 EditorState / Transaction / Facet / History。

- 验收：proptest 验证 decoration 在任意 ChangeSet 序列下的迁移正确性；
  source↔visual 双向映射 round-trip 无损。

### S5 · 布局重写

`yu-layout` 只接受 StyledRun / Widget / LineStyle；引入 UAX #14 断行、
UAX #9 bidi、CJK 禁则、widget 盒模型（intrinsic size + baseline 对齐）。

- 验收：`grep -ri markdown crates/yu-layout crates/yu-scene crates/yu-render` 零命中。

### S6 · 语义 extension 化

将 heading / emphasis / list / quote / table / task / image / math 逐个改写为
`yu-markdown` 内的 extension（parser + decoration 产出器）。

- 验收：新增一种语法（如 `==高亮==`）的 diff 只落在 `yu-markdown` 内，且 < 200 行。

### S7 · 产品面

搜索、大纲、多光标、代码块高亮（tree-sitter 上场）、导出（comrak 上场）、
跨平台第二端。

---

## 9. 明确不做的事

- **不追求一份代码三端渲染。** macOS / Windows / Linux 各自适配原生 shaping 与 GPU 后端，
  换取原生文本质感。共享的是 core，不是 GUI。
- **不引入 WebView、DOM 或常驻 JavaScript runtime。**
- **不做 Markdown → 富文本 → Markdown 的往返序列化。**
- **不为跨平台预先抽象尚无第二实现的接口。** 抽象在第二端落地时再建立。
- **不保留 v1 的接口兼容层。** v1 完整状态在 `archive/v1-source-projection`。

---

## 10. 参考

| 项目 | 借鉴内容 | 许可 |
| --- | --- | --- |
| CodeMirror 6 | State / Transaction / Facet / **RangeSet / Decoration** | MIT |
| @lezer/markdown | **增量 Markdown 解析算法与 extension 机制** | MIT |
| Helix | Rust 编辑器内核形态、Transaction / Selection、doc_formatter | MPL-2.0 |
| ropey | Rope 实现，直接依赖 | MIT |
| comrak | CommonMark 正确性 oracle 与 HTML 导出，直接依赖 | BSD-2-Clause |
| tree-sitter | 代码块内部高亮，直接依赖 | MIT |
| Zed | SumTree、坐标系统思想（**仅思想，代码不可参考**） | GPL-3.0 |
| xi-editor | Rope Science、editor construction kit 思想 | Apache-2.0 |
| ProseMirror / Milkdown | 文档模型与插件化思想（不采用其 AST 往返模型） | MIT |
