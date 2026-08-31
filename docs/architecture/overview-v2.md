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

泄漏的代价不只是「改动要动 7 处」。S4 拿 v1 的行内扫描器与 `yu-syntax` 逐份
文档比对隐藏区间（见第 8 节 S4），76 份语料里 v1 有 11 份判错，成因只有一个：
**扫描器没有块级上下文**，于是在缩进代码块、`~~~` 围栏、HTML 注释、autolink
内部照样去找行内定界符，把它们隐藏掉。呈现层拿到的是错的输入，而它不报错。

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
| 任务框 / 引用条 | 各自一个 Scene Primitive | 一个渲染中立的 `OrnamentPrimitive` |
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
yu-core ─┬─ yu-text ── yu-syntax ─┐
         ├─ yu-decoration ────────┴─ yu-markdown ── yu-state ─┐
         └─ yu-font                                           │
                    yu-layout ── yu-scene ── yu-render ── platform
```

**`yu-decoration` 在 `yu-markdown` 下方。** 本节初版把它画在 `yu-state`
之上，与第 4.3 节冲突：那里要求 `yu-markdown` 产出 decoration，而产出就得
认识那个类型。`yu-decoration` 不知道 Markdown、不知道 layout，它是一个和
`yu-text` 同级的**原语**，被上面的人使用，不使用上面的人。第 4.1 节的分层图
按「谁是中枢」排版，容易读成依赖方向，这里以本节为准。

`yu-font` 只能依赖 `yu-core`。它提供的是「给我一段文本和字体，返回 glyph 与 advance」，
它不需要、也不允许知道 layout 与 projection 的存在。

### 4.3 每层的职责与禁止事项

| 层 | 职责 | 明确禁止 |
| --- | --- | --- |
| `yu-core` | 坐标（源码坐标与带空间参数的视觉坐标）、Revision、Anchor | 依赖任何其他 crate |
| `yu-text` | Rope、Snapshot、Transaction 原语 | 让 ropey 的类型或索引逃逸出 crate |
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
| ~~ropey 主索引是 char，Yu 是 byte，混用是真实 bug 源~~ | **已失效，见下。** |
| 用的是 `2.0.0-beta.1`，不是稳定版 | 版本锁在 `Cargo.lock`；适配层只有一个文件，上游 API 变动的影响面是它 |
| 丢失 Piece Tree 的「原文件区零拷贝」，打开时有一次 chunk 重建 | 可接受；必要时用 `Rope::from_reader` 流式加载 |
| 全内存，不支持 mmap / GB 级文件 | 百万行以内无问题；GB 级文件另立方案，不在 v2 范围 |
| ropey 不提供 Revision / Anchor / History | 本就由 `yu-text` / `yu-state` 提供，与 Helix 做法一致 |
| 新建文档整篇扫描比 Piece Tree 慢约 15% | Piece Tree 新建时是一整块连续内存，那是它最有利的一刻；构造快出的约 1ms 抵掉大半，而编辑与坐标查询快 15–60 倍 |

**char index 那一条为什么失效。** 上表初版针对的是 ropey 1.6：它的主索引是
char，`insert` / `remove` 收 char index，于是每一次 byte↔char 转换都是一个
「在某个 emoji 上悄悄切错位置」的机会，对策只能是「包一层 + 靠检查」。

ropey 2.x 是全字节索引的重写。`insert` / `remove` / `slice` /
`is_char_boundary` / `byte_to_utf16_idx` / `line_to_byte_idx` 收的全是字节
偏移；char 相关的 API 只在 `metric_chars` feature 下才编译进来，而 Yu 没有
开它。适配层里 byte↔char 转换点的数量因此是**零**——不是「都包起来了」，
是根本写不出来。E4 从一条要靠纪律守的约定，变成了一件编译期事实。

代价是 beta。权衡的结论是接受：这里要防的是一类不会 panic、只会在某个字符
上悄悄错位的 bug，而消除它的手段是「让它无法被表达」而不是「小心一点」。

**实际形状比预计的小得多。** `yu-text` 在 v1 时期就有可插拔的存储后端抽象，
契约是纯字节索引的。所以这一项不是重写 3,201 行，而是实现第四个后端、并入
已有的跨后端差分测试、然后删掉三个自研后端（1,323 行）与只为「有多个后端」
而存在的比较脚手架。`yu-text` 3,050 行 → 1,573 行。

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
| `yu-syntax` | — | **新建**：S3 从 `@lezer/markdown` 移植。4,104 行实现 + 2,637 行测试（含只服务规范比对的 HTML 渲染件） |
| `yu-core` | 332 | **已完成**：收敛坐标类型，视觉坐标带空间参数。现 1,308 行 |
| `yu-text` | 3,201 | **已完成**：底层换 ropey，保留 Snapshot/Transaction 包装。现 1,573 行 |
| `yu-markdown` | 6,026 | **进行中**：`yu-syntax`（lezer 移植，S3 已完成，4,104 行）已建立；`yu-markdown` 收敛为 extensions 是 S6 |
| `yu-projection` | 3,775 | **已删除**（S6）：由 `yu-decoration` + `yu-editor::VisualText` 取代 |
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

**已执行的删除。** `yu-text` 的三个自研存储后端（Flat / Piece Tree /
自研 Persistent Rope，共 1,323 行）已删除，连同 `StorageBackend` 与
`TextBuffer::with_backend` 这套只为比较候选而存在的选择机制。
`yu-editor-ffi` 与 `platform/macos/yu-storage-macos` 已删除
（两者都是零消费者）。`experiments/` 整个目录已删除：`macos-document-host` 转正
为 `platform/macos/yu-shell-macos`，`macos-text-input` 是 README 自称「可丢弃」的
风险实验，它验证的假设（自绘 `NSView` 接 `NSTextInputClient`）与产品最终采用的
`NSTextView` 子类相反，且它依赖已删除的 `yu-editor-ffi`。它承担的人工 IME 验收
改写为 [`docs/specs/manual-acceptance-macos.md`](../specs/manual-acceptance-macos.md)，
针对产品 app。

---

## 8. 迭代阶段

原则：**先拆炸弹，再自底向上重建。** 每个阶段结束时 app 必须能运行，CI 必须全绿。

### S1 · 拆炸弹（以删除为主）

删除 TextKit fallback、capability mask、coverage gate 与诊断桥；把帧调度决策
移入 Rust；app 从 `experiments/` 转正为 `platform/macos/`。

**「渲染循环移入 Rust」的澄清。** 本文初版把这一步理解为「把 Metal 编码从
Swift 搬到 Rust」。实际调查发现 Metal 编码本来就在 Rust
（`yu_metal_render_plan` 直接把 RenderPlan 编码成 Metal 命令），Swift 侧那
14,340 行 FFI 不是为了绘制，而是为了让 Swift **验证和兜底**。

真正需要移入 Rust 的是**帧调度决策**：`MacosSurfaceHostCoordinator`（928 行
Swift）在平台侧决定何时提交帧、如何处理 metrics/scroll/resize/资源刷新。
状态在 Rust、决策在 Swift，于是每个决策点都要一次 FFI 查询——这才是 FFI
函数数量膨胀的成因。

**行数目标的修正。** 初版定下「`yu-storage-ffi` < 1,000 行」，前提是认为其
体积主要来自 count/fill 诊断桥。逐个统计 81 个 FFI 的调用者后，这个前提不
成立：只有 4 个没有调用者，其余 77 个都被产品代码真实使用。实际构成是：

```text
mod tests        4,593 行 (36%)   测试，是资产，不计入削减目标
生产 FFI 代码    ~7,800 行        77 个函数 ≈ 100 行/函数
```

跨 C ABI 的每个函数都要做 null 检查、Revision 校验、类型转换与错误码映射，
100 行/函数并不异常。因此验收指标从「行数」改为「函数数量与职责」：

- FFI 全部落在 I3 允许的类别内
  （文件、输入事件、selection 查询、Accessibility、surface 生命周期），
  且没有冗余入口；
- Swift 产品代码 < 2,000 行，self-check 移出主文件；
- app 在 `platform/macos/` 且进入 CI；
- heading / emphasis / code / link / image / table / task / IME / AX 全部保持可用。

**函数数量目标的修正（第二次）。** 上一版把指标定成「≤ 40 个」。执行下来
78 → 43，剩下的每一个都落在 I3 的类别内，但要再往下走只剩一种手段：把参数
形状**不同**的函数塞进同一个带 action 的入口。

这两种合并不是一回事：

- 参数形状相同、只差「做什么」的一族，合并之后约束落在一处 action 分支上，
  边界更小也更难写错。已合并的有表格拖动的 update/finish/cancel、分隔线的
  探测/开始、关闭协商的 cancel/save/discard、剪贴板的纯文本/HTML。
- 参数形状不同的一族（典型是 composition 的 begin/update/commit/cancel，
  以及 `copy_*` 里各自带不同前置参数的几个），合并会得到一个十参数、大半
  参数在多数 action 下无意义的入口。那是把指标做好看，不是把边界做对。

因此数量不再作为硬性阈值。真正要守的是上面第一条：每个 FFI 都能说清自己属于
I3 的哪一类，且没有第二个入口能推导出它。当前 43 个函数全部满足；再有增长时
先问它属于哪一类，而不是先问总数。

### S2 · 地基

`yu-core` 坐标类型收敛；`yu-text` 换 ropey；CI 强制依赖方向，
`yu-font` 的反向依赖必须消失。

- 验收：依赖图为严格 DAG 且方向正确；ropey 不泄漏出 `yu-text`；
  视觉坐标只有一套实现且坐标空间在类型里。

**ropey 验收口径的修正。** 原文写的是「ropey 的 char index 不泄漏出
`yu-text`」。选了全字节索引的 ropey 2.x 之后，char index 在这套 feature 下
不存在，这条验收会永远为真而不说明任何事情。实际要守的是完整的 E4：ropey
的**类型与依赖**只能出现在一个文件里。`tools/check-rope-leak.py` 拆成四条
机械规则强制（依赖归属、路径引用归属、适配层不导出、不开 `metric_chars`），
已进 CI。

**「坐标类型收敛」的口径。** 第 7 节只写了「保留，收敛坐标类型」，没有给验收
标准。逐个看过之后，实际情况分成两半：

- **源码坐标早就收敛了。** `ByteOffset` / `TextRange` / `Utf16Offset` /
  `Utf16Range` / `LineIndex` 全在 `yu-core`，全仓库统一使用，没有第二套。
  这一半不需要动。
- **视觉坐标没有。** `yu-layout` 与 `yu-scene` 各写了一份结构完全相同的
  `Point` / `Rect`，`yu-editor` 与平台层则直接散着 `x/y/width/height: f32`
  四元组。

但把它们合成**一个** `Rect` 是错的，那会丢掉现在还在的信息：这些矩形分属
block 局部、文档、物理像素三个空间。两次真实事故都出在这条缝上——`768b5e3`
把 CTLine 的绝对坐标当成 run 内相对坐标，`5fac1fe` 在已经是逻辑坐标的位置上
又乘了一次 backing scale。都不报错，只是画错，都要靠真实窗口才能发现。

因此验收定为：**收敛实现，把空间放进类型。** `yu-core::geometry` 提供
`Point<S>` / `Size<S>` / `Rect<S>` / `Scale<From, To>`，算术与校验只写一遍；
`Block` / `Document` / `Device` 三个空间标记进入类型参数，跨空间只有
`translate_into`（平移原点）与 `scale` / `unscale`（换单位）两条显式通道。
混用不再是「看起来对」的调用，而是编译不过。这是第 10 节所说「借鉴 Zed 的
坐标系统思想」的具体落点。

由 `tools/check-geometry.py` 守住（已进 CI）：不得再出现散装的
`width/height: f32` 四元组，不得在 `yu-core` 之外定义第二个
`Point` / `Size` / `Rect`。例外只有跨 C ABI 的平铺结构体与非 f32 的整数量
（atlas 纹理坐标、图片自身像素尺寸），逐个登记并写明单位。

**已知未做。** `yu-core` 里还有 284 行 shaping 类型与 19 行 `TextStyle`，
占全 crate 近四分之一，而第 4.3 节给 `yu-core` 的职责是「坐标、Revision、Anchor」，
shaping 属于 `yu-font`。它们现在住在 `yu-core`，是为了让 `yu-layout` 能依赖
`ShapingProvider` 这个接口而不依赖 `yu-font` 的实现。这是「`yu-core` 职责
收敛」，不是「坐标类型收敛」，且会新增若干依赖边，另立议题。

S2 三项全部完成：`yu-font` 反向依赖消除并进 CI；`yu-text` 换 ropey；
`yu-core` 坐标类型收敛。

### S3 · 解析器

移植 lezer-markdown 算法为 `yu-syntax`；建立 CommonMark spec 652 用例 +
comrak 差分测试 + fuzz 差分。**已完成。**

**验收口径的修正。** 初版写的是「spec 通过率 ≥ 99%」。实测原始通过率是
**643/652（98.62%）**，差的那 9 条不是没做完，而是**本文自己的决策的直接
后果**：其中 5 条来自不变量 C6（引用链接的成立与否不由 parser 决定），
3 条来自 A1/A3（源码是唯一真源，因此制表符不展开）。要拿到 99% 就得推翻
C6，而 C6 换来的是增量性——一条 `[x]: /y` 的增删不会让全文档的行内解析失效。

把 99% 直接改成 98.62% 是没有意义的：一个可以随时往下调的百分比阈值等于
没有阈值。改成两条：

- **未登记偏差必须为零。** 这是不变量 C7 的原话，也是真正防「静默地做错
  事」的那条。`commonmark_spec.rs` 双向校验——未登记的失败让测试红，
  **已登记却通过了的用例同样让测试红**，登记表因此不会变成垃圾堆。
- **原始通过用例数只能往上走。** 门禁里是一个具体的数字（643）而不是
  百分比。百分比会被悄悄调松，一个具体的用例数一旦下降就必须在提交里显式
  改掉它，并说明是哪几条退化了。

**「bench 守护」的形态。** 单字符编辑的重解析上界，门禁断言的是**重新扫描
的字节数**而不是耗时：耗时随机器和负载浮动，拿它当门禁只会得到一条时不时
变红的检查，然后被调松到失去意义。3,628 字节的文档里改一个字符实测重扫
66 字节，上限定在 256；另有一条独立断言检查这个数不随文档大小增长。两条都
反向验证过——关掉 fragment 复用，实测跳到 3,373 与 22,357。
`tools/yu-bench` 里另有耗时**报告**，它不是门禁。

**一个要带进 S5 的观察。** 重扫字节数与文档大小无关（1 MiB 与 4 MiB 都是
65~68 字节），但耗时仍然随文档大小增长（0.6 ms 与 1.3 ms）。原因是复用虽然
不重新扫描字节，却要把每一个被复用的顶层块逐个重新挂进新的 Document 节点。
增量解析目前是 **O(块数)** 而不是 O(改动量)——相对全量（1 MiB：12.7 ms）
仍然快一个数量级以上，百万行以内够用，但如果 S5 的布局也按「整棵树重新
遍历」组织，两个 O(块数) 会叠加。去掉这一项需要把 Document 的子节点换成
可持久化序列结构，是一件独立的事。

**fuzz 怎么进门禁。** `tools/verify.sh` 是以退出码为准的确定性门禁，随机
fuzz 不是，混进去等于让门禁偶尔说谎。因此拆成两半：`tools/fuzz.sh` 随机、
有时间预算、单独的 CI job，负责**发现**；它找到的每个失败最小化后入库
`crates/yu-syntax/tests/corpus/`，由确定性的 `cargo test` 负责**不复发**。
verify.sh 里保留一个 `--fuzz` 分支，好让 `check-ci-parity.py` 能确认本地
门禁知道 CI 的每一条命令。

**yu-syntax 现阶段没有消费者。** 产品链路仍然走 `yu-markdown` 的扫描器。
接线要等 S4 的 `yu-decoration` 与 S5 的布局重写：现在换掉它会把
`yu-projection`（S4 要删）与 `yu-layout`（S5 要重写）一起拖进来，顺序是反的。
S3 与 S2 的 ropey 那次不同——ropey 能「先并存跑差分再删旧的」，是因为两个
后端实现的是同一个字节索引契约；而扫描器产出的是扁平的行级块序列，
`yu-syntax` 产出的是嵌套 CST，两者没有共同契约可比。真正的差分对象是
CommonMark 规范用例与 comrak，不是扫描器——拿扫描器当 oracle 只会把它的
非规范行为固化成期望。

### S4 · 中枢

实现 `yu-decoration`（RangeSet + Decoration + map + source↔visual 映射）；
`yu-state` 收敛编辑状态。**已完成。**

> 本条初版写的是「`yu-state` 收敛 EditorState / Transaction / Facet /
> History」。四项里有三项要改，理由在下面的「yu-state 收了什么」一节，
> 每一项都是往后推或换位置，没有一项是悄悄放宽验收。

- 验收：proptest 验证 decoration 在任意 ChangeSet 序列下的迁移正确性；
  source↔visual 双向映射 round-trip 无损。

**做法：先打一条薄纵切，而不是按文档顺序做完。** S4 与 S3 最大的不同是
**有 oracle**：`yu-projection` 的 source↔visual 映射已经在产品里跑着，
而 S3 没有可比的既有实现。验收条目里的 round-trip 是个**自证**性质——一份
把所有东西映射到 0 的实现也满足它。真正要问的「映射到的位置对不对」只能靠
oracle 回答，所以先把 `yu-syntax → yu-markdown → yu-decoration` 这条链端到端
接通一次，与 v1 逐点比对，再去做剩下的宽度。

已完成的两块（**下面这两条差分随 `yu-projection` 一起在 S6 删掉了**，
它们证明过的事写在这里，用例本身留在 git 里）：

- **映射的差分**（`crates/yu-decoration/tests/projection_differential.rs`）。
  隐藏区间从真实 Projection 里取，原样喂给 `DecorationSet`，两边输入完全
  一致，因此任何差异都只能来自映射本身。`yu-projection` 在查询时沿后继找
  相邻隐藏区间，`yu-decoration` 在构造期合并——两条不同的路，同一个答案。
- **隐藏区间的差分**（`crates/yu-projection/tests/decoration_parity.rs`）。
  回答上一条刻意回避的另一半：**`yu-syntax` 的标记节点范围能不能真的驱动
  「隐藏语法」？** 76 份语料，答案是能，而且没有一条是 `yu-syntax` 错。
- **分层合并**（D6，`DecorationSet::merge`）。第 5.2 节第 4 条要求多个
  extension 各自产出集合、合并顺序确定。定序键早就是全序的，缺的是入口和
  一个真实的多 extension 消费者。后者由把 `yu-markdown` 的产出器拆成
  emphasis 与 code 两个独立 extension 提供——`` *`a`* `` 产出的四条区间分属
  两个集合且彼此相邻，正好压到跨集合的相邻隐藏区间合并。

  这里有一个要说明白的边界：两个产出器**共用同一个遍历**，所以「拆开再合并
  等于不拆」在产出侧几乎是恒真的，拿它当拆分的验证会是一条什么都没测的
  测试。它压的是 `merge`。而 `merge` 内部调用 `DecorationSet::new`，所以
  `new` 自身的 bug 两条路会一起错——合并路径因此也接上了 v1 这个外部
  oracle（`decoration_parity` 两条路都跑）。反向验证过：把相邻合并改成只合
  重叠，8 条测试红，其中就有 v1 差分那几条。

**关于「拿扫描器当 oracle」。** 上一节 S3 的末尾写着「拿扫描器当 oracle
只会把它的非规范行为固化成期望」，这里看起来是反过来做了。区别在于比的是
什么：S3 要比的是**解析结果**，扫描器的扁平块序列与 CST 没有共同契约，
比它等于把非规范行为写进期望；这里比的是**哪些字节被隐藏**，两条路各自
产出一组 source 区间，契约相同、可逐字节对齐。而且差异不是被吸收成期望，
是被逐条归因登记的——`DIVERGENCES` 表里每一行都写明是谁错、为什么。

**这轮拿到的实证。** 76 份语料里 60 份逐字节一致，16 份登记了差异，分两类：

- **v1 扫描器错（10 条）。** 它没有块级上下文，于是在不该解析行内语法的
  地方解析了：四空格与制表符缩进的代码块、`~~~` 围栏、HTML 注释、autolink
  内部、多重反引号代码跨度的内部——用户看到的代码会静静少掉两个字符。
  另有一类是遇到三个以上连续定界符（`***both***`、`**a*b***`）就整段放弃，
  同一行里前面的强调仍然正确，**失败是局部的、静默的**。
- **有意的不同（5 条）。** v1 隐藏的语法**种类**更多：链接括号与目标、
  图片、autolink 尖括号、硬换行的尾随空格。`yu-markdown::decorations` 现在
  只做强调与行内代码，其余种类是 S6 逐个 extension 的工作。

剩下 1 条两者兼有：`<http://a.com/*b*>` 里 v1 隐藏尖括号是有意的，隐藏
autolink 内部的 `*` 是错的。这几个数字由 `decoration_parity.rs` 的一条计数
断言钉住，语料增删时会红。

第一类是第 2.1 节「Markdown 语义泄漏」的直接实证：v1 的行内扫描器不是
「差一点」，是**结构上拿不到判断所需的信息**。登记表两个方向都紧——差异
消失了也要红，好让 S6 补齐装饰时必须回来删掉对应的行。

**yu-state 收了什么。** 建 `yu-state`，搬进 history（235 行）、selection
（287）、caret 绑定（196）、composition（310），合计 1,028 行。边界不是猜的：
这四个模块的 `use` 里只有 `yu-core` 与 `yu-text`，一个布局或投影类型都没有，
搬迁前逐个文件核对过。`yu-editor` 依赖它并再导出，平台层与 FFI 的路径不变。

初版那句话里的四项，三项要改：

- **`Transaction` 留在 `yu-text`，不搬。** 它是文本编辑的原语，
  `TextBuffer::apply` 就以它为输入；往上搬会让 `yu-text` 反过来依赖
  `yu-state`。它出现在原句里是把「事务」和「编辑器状态」混为一谈了。
- **`Facet` 不建，推迟到 S6。** 它零消费者，而 S4 的两条验收标准都不涉及
  它。真实的配置聚合需求要等 extension 化才出现——S3 就是为同样的理由
  没有移植 lezer 的 `configure`，那条教训是「建一份没有使用者也没有测试的
  抽象会烂掉」。
- **`EditorState` 推迟到 S5。** `EditorDocument` 的十个字段分得很干净：七个
  是编辑状态（buffer / markdown / composition / selection / preferred_x /
  last_source_change / history），三个是缓存与布局（projections / layouts /
  viewport）。现在抽 `EditorState` 是纯机械重构，要动 4,266 行，产物是一个
  `EditorState` 加一个**仍然 4,266 行**的 `EditorDocument`，而 S5 把那三个
  字段挪走之后剩下的正好就是 `EditorState`，那时抽取几乎免费。顺序反了。

**顺带修正一处坐标文档偏差。** `docs/specs/coordinates.md` 一直写着源码坐标
「全部定义在 `yu-core`」，表里列着 `SourceCaretPosition` 与
`NativeCaretPosition`，而它们实际住在 `yu-editor/src/caret.rs`。与上一轮
`VisualOffset` / `VisualRange` 是同一种情况，处理也相同：三个纯坐标类型
（含 `CaretAffinity`）迁进 `yu-core`，把逻辑（`CaretPositionMap`）留给
`yu-state`。迁移时它们的构造函数从 `pub(crate)` 变成 `pub`，所以在类型的
文档里写明了「构造只是记录，不代表位置合法」——校验属于 caret map。

**Decoration 的三个变体暂时冻结。** `Mark` / `Line` / `Widget` 在
`yu-decoration` 之外零消费者，只有 `Replace` 有。它们是不变量 D1 的必然
要求，S5 与 S6 一定会用，所以不删；但在第一个真实消费者出现之前不该继续
给它们加能力（Widget 的 measure、Mark 的样式合并规则）。D7 也因此仍然只有
字节层面的语义：widget 覆盖的 source 不占视觉字节，宽度是 layout 的事。

**F3 查清楚了，结论是它不属于 S4。** S3 把「引用标签的 Unicode full case
folding」交给 S4 决定。查下来那句交接把三件事混在了一起：让规范用例 540
失败的是**对照用的参考渲染**（`yu-syntax/tests/support/html.rs` 的
`normalize_label` 用 simple lowercase）；`yu-syntax` 的产品链路里根本没有
引用标签匹配（不变量 C6 规定 parser 只产出候选引用）；而
`yu-markdown/src/reference.rs` 里那个 `to_ascii_lowercase` 是 v1 扫描器自己
的 reference table，与 540 无关，随 v1 一起被 S6 取代。

要让 540 变绿只需改参考渲染，但 Rust 标准库没有 full case folding，得为一条
用例给测试支撑代码加一个依赖——**决定是不加**。真正要决定的事推到 S6：
v2 的 reference table 建在装饰阶段，那时才需要选归一化方式。已改写
`docs/specs/invariants.md` 的 F3 登记，并补了一节说明，免得下一个人再按
「S4 重新评估」这条线索去找一个不在那里的东西。

**验收逐条核对。**

- 「proptest 验证 decoration 在任意 ChangeSet **序列**下的迁移正确性」——
  `crates/yu-decoration/tests/map_properties.rs`，512 用例，每例 1~8 步随机
  编辑**累积**应用（`set = mapped`），每一步都校验三件事：装饰的每一端与
  同位置的 `TextAnchor` 落在一处、集合结构自洽、记录的文档长度与实际相符。
  边界语义因此不是另写一套，而是钉在既有的 `ChangeSet::map_anchor` 上。
- 「source↔visual 双向映射 round-trip 无损」——满足，而且比这条更强。
  round-trip 是自证性质，所以另有两条 oracle 差分：
  `projection_differential.rs`（同一组隐藏区间，只比映射）与
  `decoration_parity.rs`（两条链各自从源码走完，比最终结果）。

**移交给后面阶段的四件事。**

| 事项 | 去向 | 卡在哪 |
| --- | --- | --- |
| `EditorState` | S5 | 要等 projections / layouts / viewport 从 `EditorDocument` 挪走 |
| `Facet` | S6 | 零消费者，配置聚合的需求要等 extension 化 |
| 删除 `yu-projection` | S6 | `yu-storage-ffi` 有 6 处消费者，替代它们需要 Mark / Line 的真实装饰产出 |
| 不变量 F3 | S6 | 见上，v2 的 reference table 还不存在 |

第三项是 S4 唯一没能收干净的：第 5.2 节说 decoration 的双向映射「取代 v1 的
`ProjectionMap`」，现在是两套并存。取代的**能力**已经具备并逐点验证过，
缺的是把 FFI 那 6 处消费者迁过去，而那需要 S6 的装饰产出器先到位。在那之前
`yu-decoration → yu-projection` 与 `yu-projection → yu-decoration/yu-syntax`
两条临时 dev-dep 继续存在，`tools/check-deps.py` 里都写明了存续条件。

**一条不会自己成立的性质。** D2 说装饰集合「可安全并发读取」，在 Rust 里就是
`Send + Sync`，而它是由字段推导出来的，不是声明出来的。往树里塞一个 `Rc` 或
`Cell` 做缓存，编译照过、测试全绿，只有把集合发给后台任务时才炸——而那条
路径此刻还不存在（G1 的后台快照读取要到后面才接），所以没有任何现有测试会
拦住它。`set.rs` 里一行编译期断言守着，反向验证过。

### S5 · 布局重写

`yu-layout` 只接受 StyledRun / Widget / LineStyle；引入 UAX #14 断行、
UAX #9 bidi、CJK 禁则、widget 盒模型（intrinsic size + baseline 对齐）。

- 验收：`grep -ri markdown crates/yu-layout crates/yu-scene crates/yu-render` 零命中。

**完成。** 验收 grep 零命中，而且不是靠改词做到的：`yu-layout` 的依赖只剩
`yu-core`，它**拿不到**判断语法语义所需的信息。

| 刀 | commit | 内容 |
| --- | --- | --- |
| 1 | `d9c5164` | `BlockLayout` 的输入契约与几何差分 |
| 2 | `8d2927f` | UAX #14 断行、CJK 禁则 |
| 3 | `fcd6b2c` | UAX #9 bidi |
| 4 | `9dfd858` | widget 盒模型与 D7 placeholder |
| 5 | `c149670` | 行级样式（缩进 + 行高倍率） |
| 6 | `df99158` | `yu-scene` / `yu-render` 的第一轮词汇清零 |
| 7 | `2506fb1` | `BlockLayout` 的 shaping 路径，字形跟着簇走 |
| 8 | `e1ee5f9` | 输入装配器 `BlockLayoutInput` 进 `yu-editor` |
| 9 | `d42e92c` | `BlockView` 接管产品侧，表格与图片几何离开 `yu-layout` |
| 10 | `656a703` | 删掉 `LayoutSnapshot`，三个 crate 的词汇清零 |
| 11 | `e9ec908` | 任务框的画家顺序（真实窗口对比抓到的） |

#### 三个开放问题的答案

**`StyleId` / `LineStyleId` / `WidgetId` 的解释权归产出装饰的那一层。** 它填
`StyleTable` / `LineStyleTable` / `WidgetMeasure`，布局层只查表拿到「斜体、
1.6 倍字号、缩进 2.0、宽 120 高 80 基线 72」。这是 E1 在布局层的落法——不是
「不写 markdown 这个词」，是**拿不到**判断语法语义所需的信息。查不到的 id
一律报错，不给默认值：那种 bug 只会画得不对，不会响。

词汇（`StyleId` / `LineStyleId` / `WidgetId` / `WidgetSide` / `TextAttrs`）
迁进了 `yu-core::style`。这不是偏好而是硬约束：`yu-font` 实现 `ClusterMetrics`
时要用 `TextAttrs`，而 4.2 节规定 `yu-font` 只能依赖 `yu-core`。

**几何差分按「有没有发生软换行」分两个口径。** v1 布局的断行是按 grapheme
贪心，没有 UAX #14；补上之后新旧断点必然不同，所以强口径只覆盖「换行全部
来自强制换行符」的组合，弱口径退化为「断行只改变 grapheme 分到哪一行，
不改变 grapheme 本身」。两个口径各自钉了组合数。

**渐进替换，新旧并存，靠差分守着。** 一次性重写没有 oracle 可用。

#### 三层各自的落点

| 层 | 输入 | 不知道的事 |
| --- | --- | --- |
| `yu-layout::BlockLayout` | 视觉文本 + `StyledRun` + `WidgetSpan` + `LineSpan` + 三张表 | 语法、源码坐标 |
| `yu-editor::BlockLayoutInput` | `Projection` | —（它就是翻译的那一层） |
| `yu-editor::BlockView` | 上面两样 + `Projection` 的映射 | — |

`BlockLayout` 的输出**只有视觉坐标**。source ↔ visual 是 `DecorationSet` 的
双向映射（不变量 D4「这是投影映射链的唯一实现」），布局再做一遍就会有第二套。
`BlockView` 在拿到结果之后向 `Projection` 问源码区间，自己不算。

`yu-scene` 的输入是 `SceneGlyph`（字面 / 字形 id / block 局部原点 / 字号
倍率）与 `ViewportBlockContent`（底色 → 装饰 → 字形 → 图片 → 覆盖层）。
它不认识布局的盒子类型，也不认识源码坐标。

#### 这一轮的实证

- **v1 的断行不是「差一点」，是没有断行算法。** 24 个语料×宽度组合的断点在
  UAX #14 落地后全变了。行尾空白现在悬在行外，代价是这样的行宽会超过
  `max_width`，这是有意的。
- **CJK 禁则不需要 tailoring。** UAX #14 的默认对表已经覆盖。
- **方向变化处的 caret 取层级更低的那一侧**（UAX #9 §3.4）。`caret` 与 `hit`
  共用同一个 `caret_x`——两处各写一遍规则，在方向变化处就会对不上。
- **同一条毛病在 `BlockView` 上又犯了一次。** v1 的 `hit_test` 自己算 x
  （点在 gutter 里报 0、点在行末报行宽），与它自己的 `caret_for_source`
  对不上。现在 `hit_test` 只决定落在哪个视觉偏移上，几何位置回头问
  `caret_for_visual`。软换行两侧的 upstream / downstream 也随之按不变量 H5 报。
- **`tools/check-geometry.py` 拦了一次。** `LineBox` 因为多了 `height` 变成
  散装四元组，改成持有一个 `LayoutRect`。
- **变异验证 31 次，7 次第一次没被抓到。** 最有代表性的三个：把「行尾空白
  不参与排不排得下的判断」去掉，三条用例全绿（它们那一段都是行首第一段，
  行宽为 0 时怎么算都放得下）；引用竖条宽度的公式在 `line_height=1` 时怎么
  写都得 1；「标题一律排粗体」用 `MonospaceMetrics` 断言等于没断言。
- **真实窗口抓到一件 headless 看不见的事。** 前后两张 2000×2600 截图只有两处
  不同，其中一处是任务复选框跑到了文字**底下**——不 panic、不报错，只是画面
  变了。`macos-task-checkbox` 那条 self-check 断言的是命中判定，与画家顺序
  无关。修完之后剩下的差异只有约 150 个像素，全落在最长那一行末尾一个字形的
  抗锯齿边缘：`Σadvance + gutter` 与 `gutter + Σadvance` 差最后一位。

#### 几何差分随 `LayoutSnapshot` 一起删除

它守住了整轮迁移，但 oracle 没了就是没了。顶替它的是
`crates/yu-editor/tests/block_view_properties.rs`：簇铺满视觉文本、行铺满块
源码、caret 与 hit-test 说同一件事、块高盖得住每一行与每张图、表格的每个簇
都属于某个格。**压不住的是「画出来好不好看」**——那件事从来只能靠真实窗口
（`docs/specs/manual-acceptance-macos.md`）。差分历史留在 git 里。

#### 已登记的四个未做项

- **RTL 段落不右对齐。** 重排给出的是行内相对顺序；把整行推到 `max_width`
  那一侧是**对齐**，属于 `LineStyle`。
- **方向变化处只给一个 caret 位置。** 要两个得给 `caret` 再带一个方向参数。
- **表格与图片还不是 widget。** 第 3 节的对照表说它们是。真做成 widget 要求
  被替代的那段 source 从视觉文本里**消失**（`Decoration::Replace`），而 v1 的
  `Projection` 表达不了「隐藏但仍可被光标穿越」——它的隐藏 run 是给语法标记
  用的。给一个 S6 就要删的类型加这个能力不划算。两者的几何算法原样搬进了
  `yu-editor`（`table.rs` / `image.rs`），S6 随 `DecorationSet` 一起 widget 化，
  届时列宽算法成为 widget measurer 的内部实现。
- **`BlockView::hit_test` 没有 bidi。** 它照搬 v1 的按 x 扫描；bidi 正确的那条
  在 `BlockLayout::hit` 里，但它只认视觉坐标。两者合流要等源码映射换成
  `DecorationSet`。RTL 文档在 v1 里同样是这个行为，不是新引入的。

#### `EditorState` 没有抽

S4 的判断是「`EditorDocument` 的十个字段里三个（projections / layouts /
viewport）是缓存与视图，挪走之后剩下的正好就是 `EditorState`，那时抽取几乎
免费」。这一轮换掉了 `layouts` 装的东西（`LayoutSnapshot` → `BlockView`），
没有把这三个字段挪走——挪走它们是「编辑状态与视图分家」，与布局重写不是
同一件事。前提没满足就不抽，理由与 S4 相同。


### S6 · 语义 extension 化

将 heading / emphasis / list / quote / table / task / image / math 逐个改写为
`yu-markdown` 内的 extension（parser + decoration 产出器）。

- 验收：新增一种语法（如 `==高亮==`）的 diff 只落在 `yu-markdown` 内，且 < 200 行。

S5 结束时留给它的入口是清楚的：`yu-editor::BlockLayoutInput` 现在从 v1 的
`Projection` 派生布局输入，S6 把这个来源换成 `DecorationSet`。换掉之后
`yu-projection` 就没有使用者了——目前只剩 `yu-editor` 与两条差分用的临时
dev-dep 在用它。表格与图片的 widget 化、`BlockView::hit_test` 的 bidi 也在
同一次换源里落地，理由见 S5 的「已登记的四个未做项」。

**验收已达成。** 十二刀半落地：`yu-markdown` 成为 extension 集合；
`BlockLayoutInput` 多一条从 `DecorationSet` 派生的路；消费者换完，`BlockView`
向装饰问源码区间；`yu-projection` 删除；增量解析接上；`hit_test` 的 bidi 与
`BlockLayout::hit` 合流；图片成为 widget；表格的每一格自己排一次；GFM 的
TaskList 进 `yu-syntax`；复选框成为 widget；语法树跟着 `MarkdownDocument`
走；块的 kind 由树给；缩进独立成一条装饰。

验收那句话现在有数了：`crates/yu-markdown/src/extension/` 下**一种语法一个
文件，31 到 128 行**（`code_span` 31、`emphasis` 36、`link` 42、`line_break`
54、`quote` 61、`image` 68、`fenced_code` 80、`list` 94、`task` 94、`table`
104、`heading` 128），加上注册表里的一行。`syntax.rs`（180 行）是共用的树视图，
`mod.rs` 是注册表与共用类型，都不是「一种语法」的成本。
`tests/extension_decorations.rs::a_new_syntax_needs_nothing_outside_its_own_extension`
真的加了一种（`==高亮==`）来压这句话——**它只在测试文件里**，`yu-markdown`
本体一行没动。

S6 列的八种语法都在。math 走的是围栏那条路：```` ```math ```` 由
`BlockOrnament::FencedCode { info, content }` 把语言名与正文带给
`yu-embedded-math`，端到端有用例
（`yu-workspace::published_math_is_consumed_by_viewport_scene_and_render_plan`）。

**剩下的两条欠账在同一个闸门后面**：块的**边界**还是行扫描器定的，于是引用块
里的任务项没有复选框、引用块里的标题也不放大，Setext 下划线的容器标记还会露出
一个 `>`。切法与代价见下面的「块结构合并：调查结论」——**要有人真的抱怨那几件
事，才值得付「改嵌套列表里的一个字要重排整个外层项」的代价。**

> **S7 第一刀之后，这个闸门离「有人抱怨」近了一步。** 大纲只收
> `BlockKind::Heading` 的块，而容器里的标题不是那个 kind——实测
> `> # 引用里`、`- # 列表里`、`a\n===\nb` 都产出 **0 条**大纲。在这之前
> 这几件事只是「画得不够好」；大纲面板上线之后，它们变成面板上**看得见的
> 缺失**。闸门的条件没变，但触发它的那个抱怨现在有了具体形状。

#### 第一刀：extension 集合，建在 `yu-syntax` 上

`crates/yu-markdown/src/extension/` 是十个 extension 加一张注册表。一种语法
一个文件：它自己认识自己的语法，自己产出自己的装饰，拿不到别的 extension 的
产出，也拿不到排版几何（那要 `LayoutConfig`，是 `yu-editor` 的事）。

`ExtensionOutput` 的 id 是**局部**的，合并时由注册表按注册顺序整体平移。共用
一张样式表的话，`StyleId(1)` 是谁的就取决于谁先跑——那就是 D6 禁止的相互
感知，而且是静默的那种：换个注册顺序，斜体会变成等宽，不报错。

#### 这一刀最重要的一个判断：解析器换成 `yu-syntax`

第一版 extension 建在 `yu-markdown::inline` 上——那是 **v1 自己的**行内扫描
器。它编译、能跑、看起来对，与 `BlockProjection` 的隐藏区间差分**全绿**：46
份语料里连 `    indented *em*` 这种 v1 公认判错的都逐字节一致。

全绿正是问题所在。两条路共用同一个 `InlineDocument`，差分比较的是同一份解析
结果的两种读法——「共用代码路径的差分是自证的」。而 `decoration_parity.rs`
早就登记过：v1 扫描器没有块级上下文，在缩进代码块、`~~~` 围栏、HTML 注释、
autolink 内部、多重反引号里都会解析出不存在的行内语法，共 11 条。建在它上面
就是把这 11 条 bug 原样搬进 v2，而且没有任何 oracle 能发现。

换成 `yu-syntax` 的语法树之后：

- **11 条 bug 不是「修好了」，是不存在了。** 树里 `CodeBlock` 的内容是一个
  `CodeText` 叶子，`CommentBlock` 没有子节点——遍历不到就产不出装饰。v1 需要
  调用方逐个块判断「这里要不要解析行内语法」，漏一个就是「代码里的星号被吃
  掉了」；换成树之后这件事由树的形状保证，不依赖任何人记得判断。
- **两条路真的分开了**，差分才有意义。`extension_parity.rs` 48 份语料、12 条
  登记差异（11 条 v1 判错、1 条表格还没做），**没有一条是 extension 错**。
- **`yu-syntax` 有了第一个产品消费者。** S3 建成之后它一直零消费者，
  `decoration_parity.rs` 里那张登记表也一直没处兑现。

代价是十个 extension 的内部全部重写。`Projection` 与 v1 扫描器仍然在产品链路
上，删它们要等消费者换完。

#### 六种成对行内语法是同一个形状

强调、加粗、行内代码、链接、图片、autolink 在树里都是「**头两个标记子节点夹住
内容**，节点里除内容之外的部分全是语法」。`DelimitedSpan` 把这句话写了一遍，
六个 extension 共用：

```text
  [文字](目标)      Link(LinkMark 0..1, LinkMark 7..8, LinkMark, Url, LinkMark)
  ├┤    ├──────┤    opening = 0..1   closing = 7..16
    ├──┤              content = 1..7
```

`](目标)` 里还有三个标记子节点，但它们全落在 `closing` 里。按「内容之外都是
语法」取，比逐个列举子节点少一份会漏的清单——autolink 的第二个标记前面隔着
一个 `Url`，逐个列举就会漏掉它。

#### 这一轮的实证

- **`# 标题 #` 的收尾 `#` v1 从来没隐藏过。** CommonMark §4.2 说它是语法。
  树里它是第二个 `HeaderMark`，一取就有。
- **`> > 两层` v1 只隐藏了第一个 `> `。** 它的块序列记的是 `depth=1`；树里那是
  嵌套的第二个 `Blockquote`。顺带：续行的 `>` 嵌在 `Paragraph` 里而不是
  `Blockquote` 的直接子节点，所以**层数按嵌套数、标记按块内全部 `QuoteMark`
  取**——两者数得不一样，混用会把 `> a\n> b` 报成两层。
- **`- [x]` 里的 `[x]` 会被 lezer 解析成一个 shortcut `Link`。** 于是 link 与
  task 两个互不感知的 extension 盖在同一段 source 上。结果靠取并集收敛，恰好
  等于整个 `[x]`——D6 说的「不得相互感知」在这里是真的被考了一次。
- **`yu-syntax` 不认 CRLF 的硬换行。** `"a  \r\nb"` 连 `HardBreak` 节点都没有，
  换过去会让 CRLF 文档的硬换行整个失效——两个尾随空格变成可见内容。这是相对
  v1 的**真回归**，在这一刀里一起修了（`line_ending_at`）。CommonMark 的 spec
  用例只用 `\n`，压不住这一条，所以守护测试在 `tree_invariants.rs`。
- **块的 range 带着行尾换行符，语法节点不带。** 拿没修剪的 range 去找「完整
  包含它的最深节点」，几乎每个块都会一路退到 `Document`：`# 标题\n` 的块是
  0..10，而 `AtxHeading1` 只有 0..9。退到根之后结果**仍然是对的**（`nodes()`
  会把块外的节点裁掉），所以它不报错、不画错——唯一的症状是每个块都要遍历整篇
  文档，长文档变成 O(块数 × 文档长度)。这是「静默地做错事」的一种新形状：
  症状只有慢。变异验证抓到了它，用例是「块必须定位到它自己那个节点」。
- **两道防线会互相遮蔽。** `nodes()` 裁剪与注册表的越界兜底各去掉一道，另一道
  都会把后果兜住，于是两个变异**都活了下来**。现在各有一条用例直接压着自己
  那一道。
- **扫空白一次读到块末，是同一种 O(n²)。** 引用块每个 `QuoteMark` 都要调一次
  `skip_spaces`，一个五百行的引用块就要复制五百次半个块。改成按 64 字节的
  窗口分段读之后，又冒出一条**静默**的：窗口起点是按字节算的
  （`cursor - 64`），会落在多字节字符中间，`read_range` 在那里失败，而循环
  能做的只有就地放弃——于是 `# 标题` 后跟 63 个空格时，收尾标记前的空白一个
  都不隐藏。不 panic、不报错，只是画面里多出一串空格。是「run 跨过窗口边界」
  那条用例抓到的。
- **23 个变异，第一轮 6 个没被抓到。** 其中 5 个是真缺口（上面那两条、CRLF、
  块定位、链接不继承外层字型），都补了用例并反向验证过；剩下 1 个
  （`DelimitedSpan::opening` 从节点起点算起）是**等价变异**——六种成对语法的
  第一个标记都恰好在节点起点上，换掉逐字节相同，不该有测试会红。理由写在
  `syntax.rs` 里，没有当成缺口糊过去。

#### 任务项与列表项的归属，按块类型划分

任务项画成 `- ☐ 待办`，普通列表项画成 `• 项目`：任务项的 `- ` 原样留着。
这是 v1 的行为，保持不变。

值得记的是**怎么**实现的。第一版里 list 与 task 都认树上的 `ListItem`
节点，于是任务项同时拿到了替代标记与复选框，变成 `• ☐ 待办`。让 list 去问
「这个块是不是任务项、有没有 task extension」是最直接的修法，也正是不变量
D6 禁止的相互感知。

改成按**块类型**划分定义域：`BlockKind::ListItem` 归 `list.rs`，
`BlockKind::TaskListItem` 归 `task.rs`。两个集合不相交，各自读的都是同一份
共享输入，谁也不需要知道对方存在。差异消失了，D6 也没被绕过。

顺带记下：`[ ]` 在语法树里**没有节点**（`- [ ] x` 就是一个普通 `ListItem`），
所以任务这一种语法的块类型与标记范围只能来自 `block_sequence`。这是 extension
层里仅剩的一处 v1 依赖。

#### 第二刀：`BlockLayoutInput` 多一条从 `DecorationSet` 派生的路

`BlockLayoutInput::from_decorations` 与原有的 `derive(&Projection)` 并存，靠
`crates/yu-editor/tests/blockinput_differential.rs` 守着。28 份语料里 **20 份
两条路逐项完全一致**（视觉文本、样式段连同解析出来的 `TextAttrs`、行级几何、
三种装饰），8 条登记差异全部是第一刀那批 v1 bug 在这一层的表现，加上表格。

**这张登记表就是下一刀的验收清单。** S5 那种「前后截图只应有零处不同」在换源
那一刀不成立——画面**本来就该**变，变的正是这 8 处。先把清单钉住，再去看窗口，
才分得清哪些是修复、哪些是回归。

**差分里哪两条路真的分开了。** 分开的是派生：视觉文本怎么拼、哪些字节进得去、
每一段排什么字型。不分开的是几何算术——`heading_metrics` /
`block_quote_metrics` / `measure_marker_parts` 两条路**共用同一批函数**。那是
有意的：比对「2.0 倍字号抄了两遍还相等」什么都证明不了。

**标题的字号倍率由装配层盖在整张样式表上**，不让 heading extension 产一条覆盖
全块的 `Strong` Mark。理由是分层：「几级标题」是语义，归 `yu-markdown`；
「1.7 倍、排粗体」是呈现，只有这一层有 `LayoutConfig` 说得出来。产 Mark 也能
工作，但那等于把呈现决定塞回刚划清界限的那一层。

**重叠的 `Mark` 压平成不重叠的 `StyledRun`**（`yu-editor/src/marks.rs`）：
优先级高的赢，同级窄的赢，再同就按 `StyleId` 定序。第三条不是为了「对」，
是为了**确定**——结果不能取决于哪个 extension 先跑（D6）。「窄的赢」就是 v1
`style_for` 的「取最内层」，也是 link/image 那句「正文显式排 `Plain`」之所以
有效的原因。

- **变异验证 12 个，2 个活下来，都是等价变异。** 一个（`flatten` 的平局回退
  顺序）本来就被 `StyleId` 定序压死了；另一个（预先合并隐藏区间）是**死代码**
  ——`visible_pieces` 里的 `cursor.max(to)` 早就把重叠吃掉了。后者直接删掉，
  没有当成「等价」留着；那条性质补了一条用例单独压着。

#### 第三刀：换消费者

`BlockView` 不再持有 `BlockProjection`，改成持有一份 `BlockDecorations` 加一份
`VisualText`；`EditorDocument` 的 `ProjectionCache` 换成 `DecorationCache`。
`map_through` / `caret_for_source` / `hit_test` 全部走 `DecorationSet` 的双向
映射——不变量 D4 说那是投影映射链的唯一实现，这一刀把两套收敛回一套。

**`VisualText` 不是第二个映射实现。** 它做的是 `DecorationSet` 按定义做不了的
三件事，一件都不是「再算一遍」：

| | 为什么装饰集合做不了 |
| --- | --- |
| 换原点 | 装饰集合的视觉偏移是**整篇文档**的，`BlockLayout` 排的是一个块 |
| 拿出文本 | 装饰集合不持有源码，它是一组区间加一个 Revision |
| 叠 composition | preedit 是往视觉文本里**插入**一段不在 source 里的文字，§5.1 的四个变体都表达不了（不变量 H1 说它是 transient overlay） |

边界校验也留在这一层：装饰集合回答不了「这个偏移是不是字符边界」，而
`docs/specs/coordinates.md` 不许静默取整。

#### 语法树归 `EditorDocument` 的缓存

`ExtensionSet::decorate` 要一棵整篇文档的树。两个可选的持有者：

- `MarkdownDocument` 是每次解析重建的**值类型**，装不下「上一版的树」——而
  增量解析（`parse_with_fragments`）恰恰需要那个；而且 `yu-export` 这类只要
  块序列的调用方会被迫付解析的钱；
- `EditorDocument` 的 `DecorationCache`：要装饰的人才付钱，旧树也有地方待着。

选了后者。**增量解析还没接上**——这一版每换一个 Revision 整篇重解析一次，
位置留好了而已。`yu-editor → yu-syntax` 因此从 dev-dependency 升成产品依赖。

#### 顺手补齐的三种语法

换源之前必须先把还挂在 v1 上的东西接过来，否则删掉 `Projection` 就是删掉功能：

- **表格成了 extension。** `table_projection_hidden_ranges` 原样搬进
  `yu-markdown`，网格由 `BlockOrnament::Table` 带给 `yu-editor`（它是块级的，
  走 `Decoration::Line` 那条已有的通道）。两条差分各自逼出了一行登记：
  `extension_parity` 12 → 11，`blockinput_differential` 8 → 7，`Pending` 整类
  清空。
- **图片多了一条语义标注。** 装饰说不出「这一段是一张图」——image extension
  产的三条改的是字型与可见性，没有一条是这句话本身。于是加了
  `BlockAnnotation::Image`，不进 `DecorationSet`。硬塞一条 `Decoration::Widget`
  也能把信息带过去，但那会让「装饰集合里的每一条都真的改了点什么」不再成立。
  图片真正 widget 化要等布局层能问 widget 要尺寸（§5.3）。**第七刀做了，
  `BlockAnnotation` 随之消失。**
- **围栏代码块多了 `BlockOrnament::FencedCode`。** 隐藏区间说得出「围栏那两行
  不进视觉文本」，说不出「哪一段是语言名」，而 KaTeX / Mermaid 要按语言名决定
  这个块渲染成什么。

#### 这一刀的实证

- **整篇文档的视觉字节流换了基准。** 原生镜像与 IME 用的那一份此前是
  `Projection::inline`——只藏行内语法，`#`、`>`、`- ` 原样留着。现在它是每个块
  装饰的合并，结构前缀也不见了。它安全是因为 Swift 侧的 `NSTextInputClient`
  镜像持有的是 **canonical source**，视觉 UTF-16 只是拖选时内部往返的一个锚点
  （`visualUTF16` → `projection_source_selection` → 源码区间），同一个
  Revision 内自洽即可。
- **preedit 的平移量可以是负数。** 把三个字替换成一个字符的 preedit 时，后面的
  文字往前挪。第一版用 `u64` 算这一步，在那种情况下饱和到 0——不 panic、不
  报错，只是 preedit 之后的每一个光标位置都差几个字节。用例是
  `a_shorter_preedit_pulls_the_following_offsets_back`。
- **改一条 reference definition 不再让所有缓存整表作废。** v1 的投影要先查表
  才知道 `[id]` 是不是一个链接；语法树里 `[id]` 的 `LinkLabel` 是结构，隐藏
  区间不依赖索引（不变量 C6 说的「解析目标」才需要）。那条整表作废现在没有
  任何东西要作废，删掉了。
- **块整体平移，不逐个区间问锚点。** 编辑落在块外时块内每一个偏移都在每一处
  改动的同一侧，平移量是个常量。逐个区间去问锚点也对，只是把一个常量算了几百
  遍——而算错其中一份的表现是「点击落在别处」，不报错。
- **簇的源码区间两端取不同的 bias。** 起点 `After`、终点 `Before`：一个簇不该
  把它两边被隐藏的语法也吞进来，否则选中一个字会连带选中它旁边的 `*`。

#### 第四刀：删 `yu-projection`

3,775 行 v1 投影，连同它的三条差分（`extension_parity` / `decoration_parity` /
`projection_differential`）、`BlockLayoutInput` 里吃 `Projection` 的那条派生
路、以及三条临时 dev-dep 一起消失。第 2.4 节那条反向依赖（`yu-font` →
`yu-projection`）的最后一点痕迹也没了。

**删掉的是三个 oracle，得说清楚剩下什么。** 每条差分都是「拿一个在产品里
跑着的实现比对新实现」，删掉 v1 就删掉了比对的对象：

| 原来压的事 | 现在压它的是 |
| --- | --- |
| `DecorationSet` 的 source↔visual 映射对不对 | `yu-decoration/src/hidden.rs` 里不走树的**线性参照实现**，树的下降与它逐点一致——两份独立的推理互相校验 |
| extension 隐藏了哪些字节 | CommonMark 官方用例（`yu-syntax`，643/652 逐字节，不变量 C7）+ `extension_decorations.rs` 逐条钉死的用例 |
| 两条派生路排出了什么 | `block_layout_input.rs`：语料原样留着，断言换成自洽性质（样式段无缝铺满、视觉文本等于源码减隐藏、每个 id 查得到） |

**丢掉的覆盖面要写下来，不能假装还在。** 三条差分是逐字节、逐偏移、逐 bias
的比对，换上来的自洽性质抓不住「隐藏错了字节」——它们抓的是越界、崩、
id 脱节。真正的替代是上表第二行：CommonMark 用例压的是解析，而隐藏区间现在
**由树的形状导出**，不再有一层独立的判断可以单独出错。

两处只为测试活着的代码顺手归位：`yu-markdown::decorations`（S4 提前落地的
一小块产出器）在产品链路上已经没有调用者，搬进了它唯一的使用者
`tests/extension_merge.rs`——那三个函数的实际身份是「给 `merge` 喂真实交错
输入的夹具」，留在产品里只会是一段没人调用的公开 API。

#### 第五刀：增量解析接上，J1 有了可断言量

`DecorationCache` 此前每换一个 Revision 就整篇重解析一次。`yu-syntax` 的
`parse_with_fragments` 早就在了，缺的只是把 fragment 传下去。这一刀补上三处：

- `shift_through`（一次编辑之后把没被碰到的块整体平移的那个方法）多做一件事：
  把 `ChangeSet` 应用到上一棵树的 `TreeFragment` 上。
- `tree()` 拿对得上这一版 Revision 的 fragment 调 `parse_with_fragments`。
- `DecorationCacheStats` 多一个 `reparsed_bytes`。**不变量 J1 在编辑器这一层
  的可断言量就是它**：512 个块（三万多字节）的文档里改一个字符，重扫 ~60 字节。

**复用的来源有两个，按新鲜程度取。** 树正好在这次编辑的基准 Revision 上时，
fragment 从那棵树现取；只有树落后于编辑（连着编辑几次都没人要过树，比如批量
替换）时，才拿上一批 fragment 接着往下平移。两个都对不上基准 Revision 时**不
猜**——丢掉 fragment，下一次整篇重解析。多扫一遍是慢，猜错是树悄悄不对。

##### 接上去才发现的：上游 `apply_changes` 会把洞的边界说成文档末尾

`TreeFragment` 的 `open_start` / `open_end` 记的是「这一端是被改动切出来的，
不是文档的自然边界」。上游 `@lezer/common` 每次切 fragment 时**无条件重写**这两
个标记（`openStart: cI > 0`、`openEnd: !!nextC`）——只看这一次的改动，把传进来
的 fragment 已经带着的标记丢掉。

每次编辑之后都重新 `from_tree` 时这不要紧：那时标记本来就都是 false。**而链式
调用恰恰是这一刀新引入的用法。** 连着编辑两次而中间没人要过树时，第二次
`apply_changes` 会把上一次留下的那个洞的边界说成「文档末尾」，于是紧挨着洞的
那个块被原样复用。

后果可见但不报错：在空行处插一个 `X`，上一个段落会把这一行吃成延续行；再在它
前面插一个字符，第二次解析就把那个段落从洞的位置一刀两断，得到两个段落而不是
一个。这是本阶段第二例「静默地做错事」（第一例是 preedit 的无符号平移量饱和
到 0）。

修法是让**每一端的 openness 跟它自己那个来源走**：边界由这次改动切出来的就是
开的，边界还是 fragment 原来的端点就沿用原来的标记，两者重合时取并。这是与上游
的第二处分歧（第一处是 `move_to` 不判 `open_start`），说明写在
`crates/yu-syntax/src/fragment.rs` 的模块文档里，回归测试是
`incremental.rs::chained_edits_keep_earlier_holes_open`——它在修复前确实是红的。

**这个 bug 是等价性差分找出来的，不是字节数门禁。** 编辑器层「增量树必须等于
全量树」第一次跑就红了，再在 `yu-syntax` 层收敛成两次编辑的最小复现。要是只
断言「重扫字节数够小」，它会原样活下来：它让复用**变多**，字节数只会更好看。

#### 第六刀：`hit_test` 的 bidi，与守着它的那条性质

`BlockView::hit_test` 一直照搬 v1 的做法：按 x 从左到右扫簇，「过了中点算下
一个」。那条规则默认 **x 随逻辑顺序单调递增**——bidi 重排之后不成立。
`abc مرحبا def` 里点在阿拉伯语那一段上，光标最远会停到十个像素以外的另一个
位置。S5 登记过这一项，源码映射收敛成一套之后两条就能合流：文字流那一路
现在交给 `BlockLayout::hit`，它枚举这一行所有 caret 位置取最近的一个，与
`BlockLayout::caret` 用同一条规则。

表格不走那条路：它的簇已经被搬进单元格了，文字流的 x 对不上网格的 x。那一路
仍然是按 x 扫描，与 `table_point_for_visual` 配对。

##### 守着它的那条性质本来是自证的

`hit_test_lands_on_a_caret_position` 断的是「`hit_test` 给的点等于
`caret_for_visual(hit.visual, hit.bias)` 给的点」。而 `hit_test` 的返回值**本来
就是拿后者算出来的**——比的是同一次计算的两遍读法。它一直全绿，而 bidi 行里
差着十个像素。这是「共用代码路径的差分是自证的」在本阶段的第二个实例。

换成的判据来自 `hit_test` 之外：**这一行上所有够得着的 caret 位置里，没有哪个
比它给的那个更靠近点击处**。候选由簇的两端经 `caret_for_visual` 得出，与被测
的那条路无关。语料加了三条 RTL/混排。

#### 第七刀：图片成为 widget

装饰集合多了**第三张表**。样式表（`StyleId` → `TextAttrs`）、行级装饰表
（`LineStyleId` → `BlockOrnament`）之外是 widget 表（`WidgetId` →
`BlockWidget`），三张同构：局部 id、合并时由注册表整体平移（D6）。

`BlockAnnotation` 因此消失。它存在的唯一理由是「装饰说不出这一段是一张图」
——第三刀加它的时候，能说出这句话的 `Decoration::Widget` 还没有人实现
`WidgetMeasure`，硬塞一条进去会让「集合里的每一条都真的改了点什么」不成立。
现在它真的改了：整段 `![替代](目标)` 由 widget 覆盖，从视觉文本里消失，
位置上留一个盒子。

盒子由 `BlockLayout` 连同文字一起排，尺寸由 `yu-editor::BlockWidgets` 给：
解码到位给固有尺寸（`Ready`），没到位给四个行高宽的 placeholder。
**不变量 D7 的 placeholder → ready → 重排在这一刀第一次真的跑起来**：
`LayoutCache` 命中时问一句 `needs_widget_rebuild`，只朝一个方向走——「还在画
placeholder 而现在量得出真尺寸」才重排。反过来不行：不关心图片的调用方
（命中测试、Accessibility、纯度量排版）传的是空表，按尺寸表建缓存键的话
它们每帧都会把就绪的那一份换成 placeholder，然后下一帧再换回来，图片一直
在闪，而两份都是「对的」。

**光标进来时 widget 让位**，整段源码原样露出来可编辑。D7 说的「保留可编辑的
源码回退」就是这一条，用的是行内语法定界符那条已有的规则（`reveals`），
不是第二套呈现。此前替代文字一直留在视觉文本里、盒子画在它上面，于是同一张
图在替代文字长短不同时宽度不一样，而且两样都画出来。

##### 这一刀的实证

- **widget 有宽度，所以同一个视觉偏移在它两侧是两个 x。** 被隐藏的语法在
  字节流里塌成一个点，点的两侧 x 相同——所以「落在哪一侧」一直可以交给行的
  规则（H5 管的是软换行的两侧）。widget 不是：`![alt](x)` 也塌成一个点，
  而这个点的左边与右边差着整张图的宽度。`BlockLayout::hit` 因此要把「落在
  哪一沿」带出来给调用方（`CaretBox::widget_affinity`）——只看视觉偏移的话，
  点在图片右边光标会画回它左边去。第六刀立的那条「最近的 caret」性质第一次
  跑就红了，这个 bug 是它抓的。
- **整块就是一张图时，行上一个簇都没有。** `caret_positions` 原来只从簇里产
  候选，于是 `hit` 一路退到行首。widget 的两沿现在也是候选。
- **锚不到任何簇起点上的 widget 会被静静跳过。** `place_widgets_at` 只在每个
  簇的起点与视觉末尾被调用，锚在别处（一个组合字符簇的中间）的那个不会被
  放下，**连同排在它后面的每一个 widget 一起**——画面上少了一张图，不
  panic、不报错。现在它报 `LayoutError::WidgetNotAnchored`。
- **块高不再需要另取一次 max。** 图片撑高的是它所在的那一行，而行高本来就
  进了累加。此前图片是排完之后另贴上去的盒子，行不知道它有多高，
  `BlockView::height` 要 `max(行盒累加, 图片下沿)` 才盖得住——代价是图片压在
  块尾那一行上面。同一份语料的块高因此从 40 变成 50（图片 40 + 块尾那个换行
  符自己的行 10）。这是**修复**，不是回归。
- **「排完再改尺寸」整条路消失。** `apply_image_intrinsic_sizes` 此前每帧在一
  份克隆上重跑一遍；现在尺寸是排版的**输入**。四个 `visible_blocks_*` 循环各
  少一次克隆。
- **表格里的单元格要把 widget 的宽度算进去。** widget 在视觉字节流里不占位，
  按样式段切出来的单元格内容一个字节都切不到它。不算的话，一格里只有一张图
  的那一列会被压成一条缝，而图片照样按自己的宽度画出去，压在下一列上。
- **登记一条行为变化。** 目标解析不出来的图片（`![替代][没定义的引用]`）
  此前画的是替代文字，现在画一个空框。源码仍然拿得回来——光标进去就露
  出来。要保住旧行为得让 extension 去判「这个引用有没有定义」，而那要先
  回答 F3 的归一化，是另一件事。

  **那个空框是真实窗口比对逼出来的。** 第一版这里什么都不画：替代文字已经
  进了 widget，而 scene 层查不到 `ImageKey` 就 `continue`——于是布局上占着
  一个四行高宽的盒子，画面上一个像素都没有。自动化测试一条都不会红（盒子
  在、几何对、id 查得到），截图一眼就看见。现在由一条
  `OrnamentPrimitive`（`Border`）补上。

##### 真实窗口比对

方法见 S5 那一节。这一刀两份语料：

- **预期一处都不该变**的那份（不含图片的全语法文档）在**内容区逐字节
  相同**。噪声全在窗口 chrome——标题栏与底部圆角的抗锯齿，同一个二进制换
  一次运行也有（最大通道差 61）。所以比对要**先切掉 chrome**：这一轮取
  y ∈ [90, 1340]，切完之后同一二进制两张、旧版对新版三组比对全部是 0。
- **预期会变**的那份逐条核对，四条登记的差异全部兑现，其中两条是修复：
  `before ![小图](x) after` 里图片此前**盖住了 `after`**（盒子比替代文字
  宽，而它是排完之后另贴上去的），表格里的图此前溢出单元格、把那一列压成
  一条窄缝。第四条就是上面那个空框。

#### 第八刀：表格的每一格自己排一次

第六刀登记的「窄到放不下的表格，列会重叠」是这一刀的入口。根因不是列宽算
错，是**单元格没有自己的布局**：整块先排成一条线性文字流（按整块宽度断行），
再把排好的簇按源码区间分派进格子、逐个改 x。压缩过的列里内容不会重排，于是
后一列的内容压在前一列上——`| long header | x |` 排在 12pt 宽里，第二列的
content_x 是 11.75，而第一列的内容一路铺到 24。

现在每一格按**自己那一列的最终宽度**排一次 [`BlockLayout`]（零基视觉空间），
断行、bidi、widget 都是同一套代码。列宽仍然分两步：先量每格不断行的自然宽度
取列最大值，总宽超了整体压缩；**再**按压缩后的宽度给每格排一次。行高是那一
行各格高度的最大值，不再是常数。

**两处手写的规则因此消失。** 表格此前有自己的一份命中测试（按 x 从左往右扫、
过了中点算下一个）和自己的一份 caret 定位——那是第六刀从文字流那一路删掉的
规则，表格这边留了一份。两份规则会分叉，而分叉的表现是「光标画在一处、点击
落在另一处」。现在两边都是 `BlockLayout::hit` / `BlockLayout::caret`，只是作
用在格子的布局上。

##### 为什么表格不是 `Decoration::Widget`

第 3 节的对照表把表格列在「一个 block widget」那一格。**它不能是**，至少不能
是图片那一种：非空 range 的 `Decoration::Widget` 会隐藏它覆盖的 source
（`Decoration::hides_source`），而整张表的单元格内容一旦从视觉字节流里消失，
光标就进不了任何一格——不变量 A2 说编辑走的是源码，而源码位置要靠视觉偏移找
回来。图片可以，因为图片没有「内部位置」；表格有，而且那正是它要编辑的东西。

对照表真正要解决的是 §2.1 那条泄漏（一种语法一条全链路），**而那件事已经做完
了**：`yu-scene` 里没有 `TablePrimitive`（网格现在是渲染中立的
`OrnamentPrimitive`），FFI 里没有表格几何，`yu-layout` 里没有 `table.rs`。
剩下的是几何，而几何要的是内部布局，不是换一种装饰变体。所以
`BlockOrnament::Table` 留着，第 3 节那一格的说法在这里按「文档与代码冲突时改
文档并说明理由」处理。

##### 这一刀的实证

- **`BlockCluster` 多了一个 `y`。** 一条 `BlockLine` 在表格里是**一个网格
  行**，不是一条文字行——同一条文字行跨越几个格子时，它在视觉字节流里不是
  连续的一段，而 `BlockLine::visual` 必须是。格内换行体现为这一行更高，簇各
  自带着自己的 y。少了它，格内第二行的光标与选中高亮会画在第一行上。
- **「落在哪一侧」在表格里有三级规则。** widget 的哪一沿最具体；其次是**格子
  的边界**——相邻两格的内容在视觉字节流里紧挨着（中间的竖线与空白全被隐藏
  了），「上一格的末尾」与「下一格的开头」是同一个偏移，只有 bias 分得开；
  最后才是格**内**的软换行（不变量 H5）。第一版少了第三级，一律按边界处理，
  于是点在格内第二行光标会画到第一行末尾去。
- **`CaretBox` 因此多了 `line_affinity`。** 软换行边界上的一个偏移在两行各有
  一个位置，`caret` 靠 affinity 选，而 `hit` 已经按 y 决定了是哪一行——把这个
  值带出来，调用方才不用再猜一遍。这与第七刀的 `widget_affinity` 是同一种
  东西：**`hit` 知道而调用方看不出来的那部分，要显式带出来**。
- **widget 的归属是 source 的事，不是视觉偏移的事。** 一张图塌成一个点，
  而表格里相邻两格的视觉区间**首尾相接**——同一个点既是上一格的末尾、又是
  下一格的开头，中间还可能夹着一个空格子的起止。第一版按「锚点落在这一格
  的视觉区间里」分派，于是一张图被三个格子同时认领，画三遍。`BlockLayoutInput`
  因此多留一份 `widget_sources`（与锚点同序）：布局层不要它（E1 说那一层
  不认识源码坐标），**分派 widget 的人要**。这个 bug 是「格子里的图要排出
  一个落在那一格里的盒子」那条守护测试第一次跑就抓到的，而它本身是变异
  验证逼出来的——活下来的那条变异（「切片不带 widget」）说明这条断言当时
  根本不存在。
- **网格的横线要按每一行自己的上沿画。** 行高不再是常数，而场景层原来按
  `行号 × 常数行高` 排横线——越往下错得越开。守护测试立起来时又踩到两件事：
  语料**要三行**（只有两行时第一行的高度恰好等于到第二行的间距，两种画法
  画出来一模一样，变异照样全绿），以及**判据要来自场景走的那条路**（场景按
  shaper 排，`block_layout` 按 `MonospaceMetrics` 排，两者断行不同、行高也
  不同，拿后者当参照会得到一条假红）。
- **两条性质从「跳过表格」变成真的压住表格。** 「点在字的左半边」此前靠「一行
  里的簇 x 递增」跳过窄表格——而列重叠正是这一刀修的东西，跳过等于不压；现在
  探针的 y 取簇自己那一行。「最近的 caret」那条仍然跳过表格，但换了理由：网格
  行上的位置分处几个格子几个 y，欧氏最近不是对的判据（点第二列不该跑到第一列
  去）。表格换成按**格**算的同一句话，严格程度一样。

##### 真实窗口比对

- **预期一处都不该变**的那份（不含表格的全语法文档）在内容区**逐字节相同**。
- **预期会变**的那份四张表里**只有第四张变了**（y 898..1041），前三张与所有
  非表格块逐字节相同——第四张正是那张必须压缩的宽表：此前
  `…放不进一列的宽度` 直接压在 `第二格也不短` 上、第五列越过表格右边缘，
  现在每一格在自己的列里换行，行高跟着长，网格线跟着走。
- **macOS 的自动深色模式会让前后比对整个作废。** 两次截图之间系统入夜切了
  外观，同一个二进制前后差了 47 万字节。判据是
  `defaults read -g AppleInterfaceStyle`；比对前后必须在**同一外观**下拍。
  隔离之后「旧(现在) vs 新(现在)」是 0。

#### 第九刀：GFM 的 TaskList 进 `yu-syntax`

第八刀留下的三个症状同一个根因——`block_sequence` 与语法树各有一套块结构。
这一刀解决前两个：task 的 `[ ]` 在树里**有节点了**，`- [x]` 里的 `[x]`
**不再**被解析成一个 shortcut `Link`。第三个（Setext 标题两边都不认）要块
身份两边一致，仍然开着。

`crates/yu-syntax` 多两个节点类型（`Task` 与 `TaskMarker`）与一个 leaf block
解析器，一共不到 90 行；`extension/task.rs` 的标记区间改成向树要。

##### 无条件开着，而不是先建 configure

上游把任务项做成一份可选的 `MarkdownConfig`，而 `yu-syntax` 有意没有移植
`configure`（第 6.3 节）。这一刀没有为它建：**`yu-syntax` 只有一个消费者，
而它永远要任务项。**一套唯一取值的配置机制不是抽象，是一层没有第二个值的
间接。GFM 表格哪天也进来，那才是重新问这个问题的时候。

代价是这一层解析的语法成了 CommonMark 的超集。它现在不与不变量 C7 打架，
理由是**可以验的**而不是推断的：652 条规范用例里一条任务项都没有，
`commonmark_spec.rs::tasklist_syntax_is_absent_from_the_spec` 钉住这条事实。
规范哪天加进一条 `- [ ] foo`，红的是那条断言，而不是 643 的棘轮悄悄掉一格
——**这正是「静默地做错事」在文档层面的形状**：提交里只看得见「基线从 643
调到 642」。

##### 两处离开上游，跟着 cmark-gfm 走

移植到现在为止的每一条差异都不改变解析结果，这两条改变：

- **`]` 后面可以什么都没有。** 上游要求 `/^\[[ xX]\][ \t]/`。按它，`- [x]`
  单独一行不是任务项——人在敲出 `- [ ]` 与随后那个空格之间会看见复选框闪
  一下。cmark-gfm 认，Yu 自己 v1 的 `parse_task_marker` 也认。
- **必须是列表项的第一个内容块。** 上游只问「最内层容器是不是 `ListItem`」，
  于是 `- foo\n\n  [ ] bar` 的第二个段落也成了任务项。cmark-gfm 不认，
  `block_sequence` 也不认（它按列表项第一行找标记）。

两处都是**两个互相独立的参照一起说上游放宽了**才动的手。第二条尤其不只是
「与别人不一样」：复选框是按块画的，而那种段落成不了任务块，于是 `[ ]` 被
藏起来、复选框一个都不画，画面上凭空少三个字符。按窄的那一边判。

##### comrak 成了任务项的 oracle

`tests/differential.rs` 的对照方开了 `extension.tasklist`，语料生成器多一条
任务项分支（2,000 份文档里 628 份含任务项），另加 20 条手写用例专挑上面那
两处偏差、松散列表、制表符与容器嵌套。参照渲染器（`tests/support/html.rs`）
因此要照抄 cmark-gfm 的两个习惯：复选框后固定跟一个空格而标记后原有的第一个
空白被吃掉，松散列表里复选框放在 `<p>` **外面**。它是参照渲染器，判据只能
来自对照方。

**这一步不是可选的。** 不开对照方的扩展，`- [x] a` 的差分比的是「开了扩展的
Yu」与「没开扩展的 comrak」，永远不一致，于是只能把整类输入从语料里排除
——那等于新加的语法一条 oracle 都没有。

##### `[x]` 曾经**恰好**不出事

`[x]` 此前会被行内解析器认成一个 shortcut `Link`，于是 link 与 task 两个互不
感知的 extension 盖在同一段 source 上，隐藏区间靠取并集**恰好**收敛到整个
`[x]`：`(2,3) ∪ (2,5) ∪ (4,5) = (2,5)`。画面上没有任何症状。

「恰好」是关键词：并集收敛不是一条被保证的性质，link 的定界符规则、task 的
标记范围、合并时的取并集任何一个动一下它就散架，而散架的样子是多出或少掉
两个方括号，不 panic 不报错。原来那条用例断言的是**并集**，那个数在修好前后
一模一样——现在断言换成**逐条**装饰，只有逐条才看得出重叠回来了没有。

##### 两份判断还在，用差分锁着

`BlockKind::TaskListItem` 与树的 `Task` 是同一个问题的两个实现，这一刀只让
`extension/task.rs` 的**区间**改问树，**块的身份**仍然问 `BlockKind`——与其
余十个 extension 同一个形状。为什么不一起换过去：`block_sequence` 不下降到
容器里，`> - [x] q` 在它眼里是一个 `BlockQuote`。定义域改按树取的话，那种块
会被藏掉 `[x]` 而复选框一个都不画（复选框走的是 `block_sequence`）——画面上
凭空少三个字符。这个洞是新加的差分第一次跑就抓到的。

`crates/yu-markdown/tests/task_identity.rs` 把两条路锁在一起，三条断言：
`block_sequence` 说是任务项时树必须给出同一段标记且它真的被藏起来；树认而
`block_sequence` 不认时**一条装饰都不许产**（并且断言这种块在语料里真的
出现过，否则这条用例什么都没验到）；勾选状态两边一致。两条路没有共用函数
——一条判在剥掉容器标记的 leaf block 上，一条判在块的第一行上——差分因此不是
自证的。两份判断合并之后这个文件就该删掉。

##### 这一刀的实证

- **`Task` 是块级节点，`is_block()` 是一次区间比较。** 上游用 `NodeProp` 的
  `block: true` 标记，追加的节点接在末尾；这份移植没有 prop 系统，所以追加
  的块级节点必须接在**块那一段的末尾**，标记节点接在标记那一段的末尾。接错
  的后果落在 `FragmentCursor::take_nodes` 上：一个块级节点不被认成块边界，
  增量复用少复用一段——不报错，只是慢，而且 C3 的差分照样全绿。
- **`NodeKind` 的编号只有一处依赖。** 模块文档说算法里有几处依赖「id 落在
  某个区间」，实际上这份移植把 `atx_heading` 写成了显式匹配，只剩 `is_block`
  一条。追加节点前先确认了这一点，而不是照着注释绕开。
- **643/652 一格没动。** 任务项无条件开着而通过率不变，是因为规范用例里没有
  任务项——这不是运气好，是被断言钉住的事实。
- **16 条变异，15 条被抓，活下来的 1 条是等价变异。** 活下来的是「把 Task
  注册在 SetextHeading 之前」：两个分派点各自都不看顺序（`leaf_next_line` 里
  任务项一律返回 false，`leaf_finish` 里 Setext 一律返回 false），十一份
  「任务项遇上 Setext 下划线」的输入两种顺序逐字节相同，验过。理由写在
  `block.rs` 那一行上，**没有为它补一条测试**——那会是一条断言实现细节而不是
  行为的用例。
- **真实窗口比对：三组全是 0。** 语料是一份含任务项的文档（普通/勾上/大写、
  有序、嵌套、引用块里、混着强调与链接，外加一个普通列表项作对照）。旧版
  （`4d0bb5b`）同一二进制两张、新版同一二进制两张、旧对新一张——**整窗**
  最大通道差都是 0，这一轮连窗口 chrome 都没有噪声，不需要切。这一刀预期一处
  都不该变：隐藏区间前后都是同一段，只是从三条重叠装饰变成一条。
- **新加的差分第一次跑就抓到一个洞。** `extension/task.rs` 的第一版把定义域也
  改成问树（`cx.block_node(Task)`）。`> - [x] q` 在 `block_sequence` 眼里是一个
  `BlockQuote`，于是 `[x]` 被藏起来而复选框一个都不画——画面上凭空少三个字符，
  不 panic、不报错。这与第八刀那次是同一个实例：**活下来的变异说明缺一条断言，
  而补上的断言可能抓出更严重的东西**；这一次是「补上的差分抓出了正在写的那一刀
  自己的 bug」。

#### 第十刀：复选框成为 widget

第九刀的截图抓到的那个既有缺陷：`- [ ] 写完第九刀` 画出来是 `- ☐写`，方框压在
「写」上。根因一句话——**`[x]` 是 `Decoration::Replace`，它的视觉宽度是零，
而复选框有宽度**。塌成一个点的位置上没有宽度可用，方框只能事后贴上去，贴哪
都会压到别人。这与第七刀「widget 有宽度，所以同一个视觉偏移在它两侧是两个
x」是同一句话的两个方向：**有宽度的东西必须在排版里占位。**

`BlockWidget` 因此多一个变体 `Checkbox(CheckboxSpan)`，`extension/task.rs` 从
产 `Replace` 改成产 `Widget`。第 3 节对照表里那一格（`Decoration::Widget(Checkbox)`）
本来就是这么写的。

##### 它能是 widget，而表格不能

判据是**有没有内部位置**。非空 range 的 `Decoration::Widget` 会隐藏它覆盖的
source，于是：

- 表格**有**内部位置——单元格内容一旦从视觉字节流里消失，光标就进不了任何
  一格，而那正是要编辑的东西。所以 `BlockOrnament::Table` 留着（第八刀）。
- 图片与复选框**没有**。光标不需要停在 `![替代](目标)` 或 `[x]` 中间，编辑
  它们走的是整段替换（不变量 B6）；VoiceOver 也是按块 press。

**隐藏区间一个字节都没变**：`Replace` 与非空 range 的 `Widget` 同样
`hides_source`，藏的还是那三个字节。变的只是它在行里占不占位——所以这一刀
不碰视觉字节流，也不碰任何一条 source↔visual 映射。

##### 几何从两份变成一份

此前场景层自己算：拿标记起点的 caret 当左上角、行高乘 0.68 当边长。那个
0.68 现在只在 `yu-editor::widget` 里出现一次，场景层照着排出来的
`CheckboxPlacement::bounds` 画。这是不变量 E6「视觉坐标只有一套实现」的小号
版本——同一个盒子两个地方各算一遍，迟早分叉。

顺带少一条错误路径：`ViewportSceneError::InvalidTaskMarker`（「块类型说这是
任务项，而标记问不出来」）没有调用点了。画的人不再问「这个块是不是任务项」，
它只问「排出来有没有盒子」。

##### 复选框不走图片那条命中快路

`BlockView::hit_test` 有一道快路：先看点落不落在某个已排好的盒子里。第一版
把复选框也串了进去，**变异验证说那是死代码**——去掉之后全部用例照绿，因为
`BlockLayout::hit` 本来就带着 `widget_affinity`（第七刀），落在哪一沿两条路
答案一样。

图片需要那一道，是因为它的盒子**可以比行高**，`line_for_y` 会把落在图片下半
部的点算到下一行去；复选框只有 0.68 个行高，撑不出这种情况。

而串进去不只是多余：`BlockHit::image()` 会因此把一次复选框点击报成「点在一张
图上」，FFI 那一层照着它给平台一个图片区间。**多余的代码顺手说了个谎**——这是
留着它更坏的那一半。删掉之后补了一条断言（点复选框时 `image()` 必须是
`None`），反向验证过。

##### 这一刀的实证

- **13 条变异，10 条被抓，3 条活下来，全是真缺口或死代码**，没有一条是等价
  变异：
  - 「边长常数变一倍」活下来——常数没有断言就等于没有约定。图片的
    placeholder 宽度早就钉着，复选框漏了，补上。
  - 「命中只认图片」活下来——那是上面那段死代码，删掉。
  - 「块平移时复选框不跟着走」活下来——真缺口。漏平移不会 panic：盒子还在、
    几何还自洽，只是它指着**平移前**的三个字节，点一下改的是别处的源码。
    补了一条用例。
- **复选框永远是 `Ready`，一次都不进 `pending_widgets`。** 报成 `Placeholder`
  不会画错，只会让 `LayoutCache` 永远认为「还欠着一个资源」，于是每一帧重排
  一次这个块——不报错，只是慢。有断言压着。
- **真实窗口比对：只有任务项那几行变了。** 七个任务项 → 七段差异
  （y 554..589 / 629..664 / 705..740 / 780..815 / 931..966 / 1082..1117 /
  1157..1192）；同一份语料里的普通列表项、普通有序项、标题、段落逐字节相同。
  同一二进制两张的噪声基线是「最大通道差 2、超阈值 0 字节」，旧对新是
  「111,576 字节超阈值」——信噪比不用解释。

#### 块结构合并：调查结论

第十刀之后剩下的就是这一件。它一直被写成「一刀大的」，先按方法论把
「谁在用块的什么」量出来，再决定切几刀。**结论是它比登记里说的小，而且
登记里有一条说法是错的。**

##### 两套块结构差在哪（实测）

`block_sequence` 是一个**按行走的扁平扫描器**：块铺满整篇源码
（`has_lossless_coverage`），一个块带着行尾的换行符，空行也是块。语法树是
嵌套的，不铺满，节点不带行尾换行符。逐条比对之后，差异分两类：

| 输入 | `block_sequence` | 树 | 性质 |
| --- | --- | --- | --- |
| `标题\n===` | 一个 `Paragraph` | `SetextHeading1` | **只是 kind 判错** |
| `    code` | `Paragraph` | `CodeBlock` | 只是 kind |
| `---` | `Paragraph` | `HorizontalRule` | 只是 kind |
| `<div>…` | `Paragraph` | `HTMLBlock` | 只是 kind |
| `- [x] a` | `TaskListItem` | `ListItem` + `Task` | 只是 kind |
| `- 外\n  - 内` | 两个 `ListItem` | 一个外层 `ListItem` | **粒度不同** |
| `- a\n\n  第二段` | `ListItem` + `BlankLine` + `Paragraph` | 一个 `ListItem` | 粒度不同 |
| `> - [x] a` | 一个 `BlockQuote` | 降到里面的 `Task` | 粒度不同 |
| 空行 | `BlankLine` 块 | 没有节点 | 铺满 vs 不铺满 |

**登记里那条「Setext 在块序列里是两个块」是错的**：实测 `标题\n===\n` 就是
一个块，0..11，边界与树的 `SetextHeading1` 只差行尾那个换行符。Setext 的症状
因此**不是边界问题，是 kind 判错**——这把它从「要动块身份」降级成「要换一个
分类器」。

##### 树能不能唯一地说出每一块是什么：能

对每个块的 range 问树「落在块**之内**的第一个块级节点」（跳过 `Document` 与
`BulletList` / `OrderedList` 这两种容器），全部语料上都得到唯一答案，包括上表
里 `block_sequence` 判错的那四类。取不到节点的块**就是**空行块。这套查法
`BlockContext::block_node` 已经在用，不需要新机制。

##### 于是切成三刀，第二刀可能不做

1. **块的 kind 改由树给**（下一刀）。边界、铺满、增量保留全部不动，只把
   `BlockParser` 里那几个行扫描判断（`atx_heading_level` / `opening_fence` /
   `is_reference_definition_line` / `parse_task_marker`）换成一次查树。
   `BlockKind` 的**变体集合不变**——Setext 与 ATX 都映射到同一个「几级标题」，
   拼法不进这一层；缩进代码 / 分隔线 / HTML 块继续映射到 `Paragraph`，与今天
   逐字节相同（它们本来就走「未支持的语法按普通段落源码绘制」，不变量 I5）。
   变体不变意味着 `viewport_tag` 不变，FFI 不动。`AtxHeading` 该改名叫
   `Heading { level }`。这一刀一次收掉：Setext 判成段落、任务项的两份判断、
   以及 `block_sequence` 里剩下的全部行扫描分类。
2. **块的边界也由树定**（可能不做）。它换来的是「引用块里的任务项也有复选框」
   「引用块里的标题也放大」，代价是布局与缓存的粒度跟着树走——改嵌套列表里的
   一个字要重排整个外层项。**要有人真的抱怨那两件事，才值得付这个代价。**
3. **删掉行扫描器**。第一刀之后它只剩「按行切 + 算 source hash + 增量保留」。
   那部分与 Markdown 语法无关，未必该删——到时候再看。

##### 第一刀的前置：树归谁

`DecorationCache` 的模块文档明写过「树不放 `MarkdownDocument`」，两条理由：
它是每次重建的值类型装不下上一版的树；只要块序列的调用方（`yu-export`）会
被迫付解析的钱。

**第一条经不起复查**：`parse_incremental(previous, …)` 手上就有上一版
`MarkdownDocument`，上一棵树跟着它走即可，与现在从 `DecorationCache` 里取
fragment 是同一件事。第二条仍然成立，但对 `yu-export` 这种一次性调用无所谓。

真正的代价是**树的解析从惰性变成每次编辑都做**：块序列现在每次编辑都重建，
树只在有人要装饰时才建。量过了（`tools/yu-bench --size-mib 1`，1 MiB / 约两万行）：

| 每次编辑 | 中位耗时 |
| --- | --- |
| 块序列增量重建（搬之前） | 250 µs |
| 语法树增量解析（要提前做的那件） | 331 µs |
| 块序列增量重建（搬之后，实测） | **598 µs** |

**在交互路径上这笔钱早就付了。** 编辑之后要出画面，出画面就要装饰，装饰就要
树——今天它只是晚几微秒在同一帧里发生。真正多付的只有「编辑了但没人渲染」的
调用方（脚本批量改、离屏文档），那条路上 250 µs 变 580 µs，仍然远在一帧以内。

这个数把「树归谁」从一次取舍降成一次搬家。**搬完之后复量，预测兑现**：
598 µs 就是 250 + 331 加上一点噪声。

**没预测到的那一项：全量那一路涨得比两件事之和多。** 全量块扫描从 3.7 ms
变成 21.8 ms，而全量树解析单独量是 11.8 ms——3.7 + 11.8 = 15.5，差着 6 ms。
它只发生在打开文档那一次（1 MiB / 两万行），没有到要停下来的程度，但**记在
这里而不是抹掉**：预测对了两项里的一项，另一项还没有解释。

#### 第十一刀 b：块的 kind 由树给

调查结论说的第一刀。`BlockParser` 仍然按行走、仍然定块的边界，但它不再回答
「这个块是什么」——`BlockKind` 由 `crate::classify` 问语法树要。

**一个查询，两个调用方。** `extension::block_node(tree, source, range)` 是
「这个块对应树里的哪个节点」的唯一实现：解析时 `classify` 用它定块的身份，
装饰时 `BlockContext::syntax` 用它定各家 extension 的起点。两处各写一遍的话，
同一个块在两层眼里可以是两个节点——那正是这一刀要收掉的那种重复。

它现在**按位置二分下降**（`TreeCursor::child_ending_after`），不再逐个子节点
扫。每个块都要问一次，而且一篇文档要问两遍：逐个扫是 O(块数²)，两万行的文档
上唯一的症状是慢。

**树给变体，行扫描器给负载。** `BlockKind` 的每一个变体都由节点类型决定；
变体上挂的字段（列表标记是 `-` 还是 `*`、序号从几起、围栏闭没闭合）不是分类，
是从源码字节里读出来的负载，行扫描器扫边界时顺手就读到了，而树反而说不出
「这个围栏没有收尾」。判据是「这一句话是不是 Markdown 语法的分类」。

`AtxHeading { level }` 因此改名 `Heading { level }`：ATX 与 Setext 落在同一个
变体上，**拼法不进块的身份**——只有隐藏区间需要知道它是哪一种，而那是
`extension/heading.rs` 问树的事。变体集合没变，`viewport_tag` 没变，FFI 没动。

##### 一个块只是叶子节点的一个片段时，谁也不是

行扫描器的边界与树的块边界不保证对齐，于是一个树节点可能横跨两个块。这时要
分两种情况，判据是 `NodeKind::is_block_context`：

- **容器节点**（`Blockquote` / `ListItem`）横跨是正常的：块就是容器里的一组
  行，说 `  - 内` 那一块是一个列表项没有错。
- **叶子节点**横跨说明这个块只拿到了它的一半。`foo\n-` 是一个二级 Setext
  标题（`-` 也是合法的下划线），而行扫描器在 `-` 那一行另起了一块——它看上去
  像一个列表标记。两块都认领这个标题的话，画面上会出现两个放大的行，第二个
  只有一个 `-`。这种块退回 `Paragraph`。

**这条规则是探针抓到的，不是想出来的。** 第一版没有它，`foo\n-` 直接产出两个
`Heading { level: 2 }`，全部自动化断言都绿——因为语料里没有这个形状。写完
`classify` 先拿十四个边界输入跑一遍打印，是这一刀唯一一次「先看再断言」。

##### 行首的缩进也要修剪，顺手修好一个既有缺陷

块带着自己的缩进（`  - 内` 的块从两个空格开始），而内层的 `BulletList` 从 `-`
开始。不修剪行首，「最深的包含者」就停在**外层**的 `ListItem` 上。

这件事此前就在错，只是没人看得见：`  - [x] b` 问不到自己的 `Task` 子节点
（复选框一个都不画），而 `  - 内` 的标记装饰指着**外层那一项的 `-`**——一条
指向块外的区间，同时它自己的 `  - ` 原样留在正文里。修剪行首之后两件事一起
好了。

补的守护测试是「装饰**指向**的 source 区间也不许越出这个块」。原来那条
「装饰不越块」查的是装饰盖在哪一段，查不到负载里带的区间（`MarkerOrnament::
source`、`ImageSpan` 的四段、`CheckboxSpan` 的三个字节）——**这条断言第一次跑
就在旧代码上红了**，反向验证过。

##### 登记的行为变化

三处，方向都是「行扫描器错，树对」：

| 输入 | 此前 | 现在 |
| --- | --- | --- |
| `标题\n===` | 一个普通段落 | `Heading { level: 1 }`，下划线不进视觉文本 |
| `foo\n[a]: /x` | 第二行是一条引用定义 | 段落的延续，不进引用表 |
| `foo\n2. bar` | 第二行是一个有序列表项 | 段落的延续（序号不是 1 的列表打断不了段落） |

`> foo\n> ===` 仍然是一个 `BlockQuote`，引用块里的任务项仍然没有复选框——那
两件事要块的**边界**也由树定，是第二刀的事。

导出与可访问性此前各有一份「标题的正文在哪」，两份都只认 ATX，于是 Setext
标题在大纲里会带着一行 `===`。它们现在都调
`yu_markdown::heading_content_range`——那一层才认识 Markdown 语法。

`tests/task_identity.rs` 删掉了：它锁的是 `BlockKind::TaskListItem` 与树的
`Task` 这两份判断，两份并成一份之后它锁的是自己。

##### 这一刀的实证

- **23 条变异，第一轮 18 条被抓，5 条活下来。** 复查之后 4 条是真缺口，各补了
  一条用例并反向验证会红；只有 1 条是等价变异：
  - 「认不出的节点退回行扫描器的形状」活下来——**`_` 那一条兜底里混着两句
    不同的话**：「树说不出这个块是什么」（节点退到 `Document`）与「形状与节点
    对不上」。前一句的用例一条都没有。拆成两条分支，补了
    `- a\n<div>\nx`。
  - 「下降不要求完整包含」活下来——`classify` 里的片段规则把它挡住了，
    **两道门挡的是同一件事，于是里面那道没有自己的用例**。它真正管的是
    `BlockContext::syntax`：退不到能装下整块的节点，块后半段的行内语法一条
    装饰都产不出来。补了 `一\n===\n*斜体*` 的断言。
  - 「标题装饰不看块的身份」活下来——同一份语料，另一个方向：按「块里找得到
    标题节点」判定的话，`一\n===\n*斜体*` 会被整块放大。
  - 「ATX 与引用定义那一条边界判断去掉」活下来——**边界的用例缺一半**。去掉
    之后 `# 标题\n段落` 变成一个块，而那个块横跨两个树块，于是标题不再是标题：
    块的身份没错，是边界把它吃掉了。原来那条引用定义的用例恰好压不住它——
    语料里两条定义挨着写，段落循环的 break 条件里还留着同一个判断。
  - 活下来的那 1 条等价变异是「行首修剪不夹到 `trimmed_end`」：交叉之后
    `TextRange::new` 给 `None`，`unwrap_or` 退回未修剪的整块 range，而全空白的
    块在 `classify` 之前就被 `is_blank` 拦掉了。理由写在那一行上，没有为它补
    测试。
- **代价量过了**（`tools/yu-bench --size-mib 1`，1 MiB / 约两万行，同一台机器
  上前后各跑一次）：

  | | 第十一刀 a 之后 | 这一刀之后 |
  | --- | --- | --- |
  | 全量块扫描（打开文档那一次） | 22.6 ms | **33.7 ms** |
  | 增量重建（每次编辑） | 567～625 µs | 591～611 µs |

  **交互路径上是噪声**（±25 µs，三个位置各有正负）；多付的 11 ms 全落在打开
  文档那一次，两万个块每个多问一次树，摊下来约 0.5 µs 一个块。二分下降是这个
  数成立的前提——逐个子节点扫的话这里是 O(块数²)。
- **真实窗口比对：两份语料，改的都是该改的那几行。** 同一二进制两张的噪声
  基线是「最大通道差 0～1、超阈值 0 字节」。
  - Setext 语料：差异从 y=408 起（第一个 Setext 标题）往下全部重排——标题变大、
    下划线整行消失，后面的内容跟着上移。**y<408 的 ATX 标题逐字节相同**，那是
    对照。
  - 嵌套语料：**只有一段差异**，y∈[243,273)——`  - 嵌套的列表项` 那一行。同一
    份语料里的外层项、嵌套任务项、顶层任务项、引用块、有序项全部逐字节相同。
- **截图又抓到一个既有缺陷（没有在这一刀里修）**：嵌套的**任务项**不缩进。
  缩进是 `MarkerOrnament::indent` 带上去的，而任务项按设计不产标记装饰
  （`- ` 原样留着，见 `extension/task.rs`），于是没有任何东西携带它的缩进量。
  前后逐字节相同，所以不是这一刀带进来的——但这一刀把它旁边那一行修好了，
  它因此**变得显眼**：同一层的列表项缩进了，任务项没有。**第十二刀修掉了**，
  见下一节——修法与当时登记的猜想相反，不是给任务项补一条标记装饰，而是把缩进
  从标记装饰里拆出去。

##### 树不在的时候

`MarkdownDocument::tree` 只有一种成因会是 `None`：源码超过 4 GiB。那种文档一条
装饰都产不出来，所以 `classify` 退化成行扫描器自己的结构形状——`# 标题` 会变成
一个普通段落。**这是登记在案的降级**，不是兜底：那份文档本来就是按纯文本画的。

#### 第十二刀：缩进独立成一条装饰

第十一刀 b 的截图抓到的那个既有缺陷：嵌套的任务项贴着左边缘，而同一层的普通
列表项缩进了。根因一句话——**缩进挂在标记装饰上，而任务项按设计不产标记**。

`BlockOrnament::Marker` 此前兼着两件事：「行首那个 `-` 画成什么」与「这一块
整体让多少列」。它们不是同一件事，只是恰好都由列表项产出：

- 普通列表项两件都要（`•` 替代 `-`，正文往右让）；
- **任务项只要后一件**——它的 `- ` 原样留在正文里（这样任务项画成
  `- ☐ 待办`、普通项画成 `• 项目`，见 `extension/task.rs`），所以它产不出标记
  装饰，于是也就一列都让不出来。

拆成 `BlockOrnament::Indent { columns }`，`MarkerOrnament` 不再带 `indent`。
`list.rs` 产两条，`task.rs` 产一条。**两个 extension 仍然不相交**（不变量 D6）
——它们各自说自己那一块让多少，谁也不需要知道对方存在。这比「让 `list.rs` 也
认任务项」好的地方就在这里：那条路要 `list.rs` 知道任务项的存在，只为了决定
标记文本画不画。

##### 「一个块缩进多少」只剩一个算法

新增 `BlockContext::indent_columns()`：从第一行的起点起跳过空格与制表符。
`list.rs` 此前按 `ListMark` 的起点减去行首算同一个数——那要先拿到标记节点，
而任务项要缩进却不要标记，两处各算一遍就会分叉。

`yu-editor` 那边的几何因此变成三段相加，各说一件事：

```text
indent = 引用竖条让出多少 + 源码里缩进了几列 + 行首标记本身占多宽（含它与正文之间那一列）
```

普通列表项逐字节不变（`columns` 就是原来的 `marker.indent`），任务项从
「一列都不让」变成「让 `columns` 列」。

##### 这一刀的实证

- **10 条变异，8 条被抓，2 条活下来，两条都不是等价变异：**
  - 「缩进列数从块起点算，不从第一行起点算」活下来——**那说明
    `BlockContext::first_line_start()` 是一层没有用的间接**：块序列铺满源码、
    不在行中间切，所以块的起点**就是**第一行的起点。它把整块切成行再取第一条
    的起点，同一个答案，代价是 O(块长度)。删掉了。这是「活下来的变异是死代码」
    的一个实例。
  - 「标记与正文之间不留一列」活下来——**常数没有断言就等于没有约定**（第十刀
    学到的那一条，这次落在另一个常数上）。拿掉那一列 `•项目` 会挤在一起，而
    一条用例都不红。补了断言，反向验证过。
- **真实窗口比对：只有嵌套任务项那一行变了。** 差异一段，y∈[318,349)；同一份
  语料里的嵌套列表项、顶层任务项、顶层列表项、引用块、有序项、段落全部逐字节
  相同。噪声基线（同一二进制两张）是「最大通道差 0、超阈值 0 字节」；删掉
  `first_line_start()` 之后重拍一次，内容区（切掉 chrome，y∈[37,1270)）与删之前
  逐字节相同——那次删改确实是等价的。
- **第一次比对拍到的是旧二进制。** 前后差异是 0，差点得出「改了等于没改」的
  结论。原因是陷阱 23：两份 app 的 bundle id 相同，`run-app.sh` 只 `pkill` 它
  **自己那条路径**，而当时在跑的是另一个工作树里的 Yu.app，于是 `open` 只是把
  旧实例激活到前台。判据是 `pgrep -lf` 看进程的**完整路径**，不是看窗口。

#### 第十三刀：候选引用要查表才算数

不变量 C6 规定 parser 只产出**候选**引用：`[文字][标签]` 在树里是一个 `Link`
节点，无论 `标签` 有没有被定义过，成立与否由装饰阶段判定。**装饰阶段此前不
查表。** `link.rs` 的模块文档还把这件事写成了一条设计：「引用式链接不需要
definition 索引」——那句话与 C6 直接冲突，代价是 `[文字][没定义]` 画成一个
哪儿也去不了的链接。

这一刀把表接进装饰阶段：`ExtensionSet::decorate` 多收一个
`ReferenceDefinitionIndex`，`BlockContext::resolves(label)` 回答「这个标签查得
到吗」，`link.rs` 与 `image.rs` 在产装饰之前先问一句。查不到的候选按 CommonMark
**根本不是**链接或图片，整段按源码画（不变量 I5）。

##### 顺着摸出来的那个更严重的缺陷：`LinkLabel` 带着方括号

节点覆盖的是 `[标签]` 而引用表里存的是 `标签`。原样交出去的话每一条**完整
引用**都查不中——`![替代][标签]` 无论定义在不在都解析不出目标，画面上永远是
一个空框。shortcut 那一路恰好不出事：它没有 `LinkLabel` 节点，落到正文上，
而正文本来就不带方括号。

这条 bug 一直躲在第七刀那条登记后面（「目标解析不出来的图片画一个空框」），
因为当时的语料只有**没定义**的引用——那种确实该画不出来。真实窗口比对里两行
并排放着才看出来：定义写对了的那一行，图也是同一个空框。

`DelimitedSpan::reference_label` 现在是「这一段是不是引用式、标签在哪」的唯一
实现，链接与图片共用。此前 `image.rs` 自己算一份、`link.rs` 根本不算。

##### 装饰第一次依赖了块外的事实

`DecorationCache` 是按块留的：range 与 kind 对得上就复用。而「这条引用成不
成立」取决于**文档全局**的引用表——补一条定义、删一条定义，用到它的那个块一个
字节都没变，缓存照样命中。症状是一条写对了的引用画成普通文字，或者一条已经
失效的引用还画成链接，都不报错。

所以缓存多一道：记住产出时那份引用表的**内容指纹**，不一样就整个清掉。指纹折
的是各条定义的标签与目标的**内容哈希**（`ReferenceDefinitionIndex::fingerprint`
本来就有），不是它们的位置——折位置的话每敲一个字都要把整篇文档的装饰重算
一遍。两条路（编辑后的 `retain_blocks`、第一次产装饰的 `get_or_build_block`）
共用同一个私有方法：只放一条路上，另一条就会攒下按旧表算的条目。

代价是**每一次 definition 的内容编辑都清一遍装饰缓存**，它不区分「哪些块用到
了这条定义」。definition 不常改，先按整清算；真成为热点再按标签建反向索引。

##### F3 选了哪一种归一化

CommonMark 要求 Unicode **full case folding**，标准库不提供，要引入一个依赖。
`yu-markdown` 现在一个外部依赖都没有，为几个字符（`ẞ`→`ss`、`ﬁ`→`fi`）扩大
依赖面不划算（第 6 节的依赖取舍）。**选 `str::to_lowercase`（simple
lowercase）**：从「只认 ASCII」走到「认 Unicode 的绝大多数」——`[Ä]` 与 `[ä]`
此前折不到一起，那是一条写对了的引用被画成普通文字。

剩下的那几个字符登记在 F3 上，规范用例 540 仍然红，棘轮仍然是 643。

##### 这一刀的实证

- **10 条变异，9 条被抓，1 条活下来，是真缺口**：把归一化里「折空白」那一步
  整个去掉，一条用例都不红——语料里没有一个标签带空白。CommonMark 的归一化有
  三条（去首尾、折内部、折大小写），此前只有大小写那条有断言。补了三种写法
  （多个空格、两端空白、跨行标签），反向验证过。
- **真实窗口比对：两行并排才看出来。** 语料把「没定义的引用」与「定义写对了的
  引用」放在相邻两行：
  - 旧版两行画得**一模一样**——都是「链接的方括号被藏掉」加「一个灰盒子」。
    定义写对了的那一行也解析不出目标，那正是方括号 bug 的样子。
  - 新版：没定义的那一行整段按源码画；定义那一行的链接成立，图片换成正常的
    pending placeholder。
  - 三段差异，两段是那两行；第三段是**光标闪烁**，同一二进制的两张里也有
    （第一张与第 2/3 张差在同样的位置，2 和 3 互相为 0）。**语料只有一行的话
    这个 bug 看不出来**，因为没有对照。

##### 第七刀那条登记的行为变化，方向反了

当时写的是「此前画替代文字，现在画一个空 placeholder 盒子……保住旧行为要先
回答 F3 的归一化」。**旧行为不该保住**：CommonMark 说解析不出来的引用根本不是
图片，它是一段普通文字，连替代文字都不该单独画出来——`![替代][没定义]` 就画成
`![替代][没定义]`。这是「文档与代码冲突时改文档并说明理由」的一次。

场景层那个「查不到 `ImageKey` 就画空框」的兜底留着，但它现在够不着：装饰阶段
自己查表，缓存又按指纹失效，编辑器自己那条路走不到那一支。它守的是别的调用方
拿一份对不上的表来渲染，**没有用例**，理由写在那一行上。

#### 还没做的
- ~~**`block_sequence` 与语法树的块结构还没合并。**~~ **块的身份合并完了**
  （第十一刀 b）：Setext 标题、引用定义、任务项的分类现在都只有一份实现。
  **剩下边界**——`block_sequence` 不下降到容器里，于是引用块里的任务项没有
  复选框、引用块里的标题也不放大（v1 起就是这样）。那是调查结论里的第二刀，
  它换来的那两件事要有人真的抱怨才值得付「改嵌套列表里的一个字要重排整个外层
  项」的代价。
- ~~**同一个「这是不是任务项」还有两个实现。**~~ **并成一份了**，
  `tests/task_identity.rs` 随之删掉。
- ~~**新发现（截图抓到的，与这一刀无关）：复选框盖住了正文的第一个字。**~~
  **第十刀修掉了**，见上。原文保留在这里，因为它是「截图抓到自动化测试抓不到
  的洞」的又一个实例：
  `- [ ] 写完第九刀` 画出来是 `- ☐写`，方框压在「写」上。`[ ]` 藏掉之后塌成
  一个点，复选框是画在那个点上的一个覆盖物，而它有宽度——**与第七刀
  「widget 有宽度，所以同一个视觉偏移在它两侧是两个 x」是同一件事**，只是
  复选框还没有走 widget 那条路（它由 `yu-workspace` 按
  `BlockKind::TaskListItem` 直接画）。前后截图逐字节相同，所以这是**既有**
  缺陷，不是这一刀带进来的；十个 self-check 与全部自动化断言都是绿的——
  盒子在、几何自洽、id 查得到，只有画面上看得见。与第七刀那个「画不出来的
  图一个像素都没有」是同一类。

  **登记时的判断有一处错了，记在这里：** 当时写的是「修它要先解决非空 range
  的 widget 会隐藏它覆盖的 source，与表格那一格是同一个问题」。不是同一个
  问题——隐藏 `[x]` **正是**想要的（`Replace` 本来就藏着它），而表格不行是
  因为单元格**有内部位置**。判据是「有没有内部位置」，不是「会不会隐藏」。
  按错的判断，这一刀会被推到表格那件事后面；按对的判断，它就是一个小刀。
- ~~**新发现（第十一刀 b 的截图抓到的）：嵌套的任务项不缩进。**~~ **第十二刀
  修掉了**，见上。当时登记的修法猜错了一半：写的是「让 `task.rs` 也产一条只带
  `indent` 的标记装饰」，那会让两个 extension 开始互相知道对方存在。对的做法是
  反过来——**把缩进从标记装饰里拆出去**，两边各产自己那一条，谁也不用知道对方。
- **新发现（与任务项无关，属于上面那条 Setext 线）：Setext 下划线那一行的
  容器标记在树里没有节点。** `> foo\n> ===` 解析成
  `Blockquote(QuoteMark, SetextHeading1(HeaderMark))`——只有一个 `QuoteMark`，
  第二行的 `>` 落在标题正文的 gap 里。产品上的表现是引用块里的 Setext 标题
  会露出一个 `>`。上游 `@lezer/markdown` 同样如此（`parseLeafBlock` 把
  `line.markers` 追加在调用 leaf 解析器**之后**），规范用例里没有这个形状，
  所以一直没被抓到。第九刀的探针撞上的，**第十一刀 b 也没有修**：块的身份
  合并之后 `> foo\n> ===` 仍然整块是一个 `BlockQuote`（`classify` 拿到的是
  容器节点），露出来的那个 `>` 要块的**边界**下降到容器里才碰得到——第二刀
  的事。

### S7 · 产品面

搜索、大纲、多光标、代码块高亮（tree-sitter 上场）、导出（comrak 上场）、
跨平台第二端。

#### 第一刀：大纲

##### 先回答的那个问题：大纲挂在哪

`AccessibilitySemanticSnapshot` 已经在建一棵文档级的语义树，里面就有 Heading
节点与 label 区间。所以要先说清楚：**大纲是它的第二个消费者，还是另一份派生
视图？**

**选了后者：并列的两份派生视图。** 理由不是「语义树里东西太多」，而是三者
共享的那份「唯一实现」（D4）本来就不在语义树里，在更下面一层——「哪些块是
标题、几级、正文在哪」由 `BlockKind::Heading` 与
`yu_markdown::heading_content_range` 定义。语义树自己是它的**第一个**消费
者，`yu-export` 是第二个，大纲是第三个。三份视图共用同一份定义，D4 要的唯一
性已经满足；把大纲再叠在语义树上，唯一性一点没多，耦合多了一层。

而两者的定义域是**结构性**地不一样，不只是「语义树多了行内节点」：

| | `AccessibilitySemanticSnapshot` | `OutlineSnapshot` |
| --- | --- | --- |
| 形状 | **扁平**：每个块节点的 parent 都是 Document(0) | **层级**：`##` 挂在它上面最近的 `#` 下 |
| 坐标 | UTF-16，服务 `NSTextInputClient` / AX 的 ABI | 源码坐标，UTF-16 只在 FFI 边界出现一次 |
| 代价 | 每个非代码块跑一次 `parse_inline_with_definitions` | 只扫块序列，不解析行内 |
| 导航 | 不需要 | 要块索引 |

把大纲建在语义树上，就得往语义树里塞进块索引与标题嵌套两样只有大纲要的东
西，还要改掉 `parent` 字段对 VoiceOver 的含义——那是一次 ABI 变更，换来的
只是大纲少写一个循环。

`OutlineSnapshot` 住在 `yu-editor`，与 `accessibility.rs` 并排：两者是同一
层的兄弟，不是上下游。它是 `(TextSnapshot, MarkdownDocument)` 的纯函数，
**一个 `EditorDocument` 的字段都没用到缓存那三个**——所以这一刀没有给
「`EditorState` 该不该抽」提供新证据，那三个字段照旧不碍事。

##### 顺带收掉的一个分叉：标题的正文有过两个答案

写大纲的探针时（十八个边界输入先打印再断言）抓到 `## a ##` 的正文报成
`"a ##"`。查下去是一条**三个消费者里两个错**的分叉：

- `extension/heading.rs` 问语法树，`yu-syntax` 把收尾的 `#` 串单独建成一个
  `HeaderMark` 节点，所以编辑器里 `## a ##` 显示 `a`——**对的**；
- `heading_content_range` 自己扫行，认得 ATX 前缀与 Setext 下划线，**不认得
  收尾串**——那条规则（前面要有空格、后面只能是空白、整行都是 `#` 不算）只
  写在 `yu-syntax` 里。于是导出成 `<h2>a ##</h2>`，VoiceOver 读作「a ##」。

三条都不报错、不 panic，全部自动化断言都绿——`yu-syntax` 的 652 条规范用例
压不住它，因为那条棘轮走的是 `yu-syntax` 自己的 HTML 渲染器，不经过
`heading_content_range`。

修法不是给 `heading_content_range` 补一条收尾串的扫描（那会变成第三份实
现），而是让它**问树**：`extension/heading.rs::anatomy` 一次算出「藏哪几段」
与「正文在哪」，**正文就是节点范围减去藏的那几段**，两者由构造保证不可能分
叉。为此 `heading_content_range` 的签名从 `(&TextSnapshot, Block)` 变成
`(&MarkdownDocument, Block)`——树跟着 `MarkdownDocument` 走（S6 第十一刀），
拿不到文档就拿不到树。`BlockContext::for_block` 是给装饰之外的消费者用的
入口：同一个上下文，只是没有焦点。

守护测试三处（`yu-markdown` 的正文区间、`yu-export` 的 `<hN>`、`yu-editor`
的大纲标签），变异验证过：把正文的终点改回 `node.range().end()`，三条同时红。

##### 导航不另开 FFI

`yu_storage_session_outline_items` 只回报视图。跳转由平台侧组合已有的两个
入口完成：拿 `label_start_utf16` 调 `set_selection_endpoints`，再调
`macos_shaped_caret_scroll_request`。滚动仍然走 `yu-editor::viewport` 那条
路，场景层不自己算 y。

##### 已登记：面板上的标题带着行内标记，第三刀再剥

`OutlineItem::label_range` 是**源码**区间，所以 `## **粗** 标题` 在面板上会
显示成 `**粗** 标题`。

**剥掉行内标记的唯一实现已经存在，而且不在 `yu-markdown`**：它是
`DecorationSet` 的 `hides_source`（D1「视觉表现的唯一来源是 DecorationSet」），
`emphasis` extension 已经在藏那两个 `**`。在 Rust 里新写一个
`strip_inline_markup` 会是第四份答案，必定与装饰分叉——正是这一刀刚修掉的
那个形状。

但走 DecorationSet 这条路要付一笔新钱。**FFI 上没有任何入口把视觉文本交给
Swift**（`projection_caret` / `projection_source_selection` /
`projection_hit_test` 全是坐标映射，`composition_projection` 是 IME），因为
文档由 Rust 渲染，Swift 从来不需要那些字节，它手上只有 canonical source
镜像。而大纲面板是 AppKit **自己画字**的——这是第一次有 UI 需要「字」。要
剥就得新开一个 FFI，而且只能是**回报区间**的那种（「这一段里哪几段被藏
了」，Swift 拿自己的镜像减掉），不能是把文本拷过去的那种——后者破 C4
「parser 不复制正文」与整套 range-backed 设计。

**现在只有一个消费者，所以不建。** 触发条件写死在这里：**搜索面板有完全
一样的需求**（结果那一行也要显示不带语法的文本）。两个消费者到齐时再开那个
区间 FFI，一次给两个面板用。这与「`EditorState` 什么时候抽」是同一条规矩。

显示源码区间不是一个错的答案——它显示的是源码，是一件真事，没有引入第二份
定义；而绝大多数标题根本没有行内标记。

##### UI 还没做，这是一个待决的取舍

派生视图与 FFI 都有测试压着；**大纲面板没有做**。S1 的未达成项「Swift 产品
代码 < 2,000 行」当前是 4,810 行（`SelfChecks.swift` 的 905 行不计），一个
带层级、可点击、跟着 Revision 刷新的 outline view 大概再加 200–300 行，把
差距从 2.4 倍推到 2.6 倍。这笔钱值不值得付，是产品决定，不是顺手做完就算的
事——所以这一刀停在 FFI，把决定留给下一刀。

#### 第二刀：大纲面板

决定已经拍了：**面板要做**，代价是上面那条未达成项继续往外走。实际数字比
预估的高一点：**Swift 产品代码 4,810 → 5,276 行**（`SelfChecks.swift` 从
905 涨到 1,068，同样不计），从 2.4 倍推到 **2.64 倍**。分布是
`OutlinePanel.swift` 275、`DocumentWindow.swift` +106、`StorageBridge.swift`
+66、`DocumentTextView.swift` +15、`main.swift` +4。`DocumentWindow` 那 106
里有约 25 行是真实窗口自检（与 `runFrameSchedulingSelfCheck` 同住，那一份
按老规矩留在产品文件里）。

##### 面板上只有三件事是真正的逻辑

其余都是 AppKit 样板。三件事各自对应一条守护断言：

1. **平表 → 树**。FFI 给的是带 `parent` 下标的平表，`NSOutlineView` 要
   parent→children。挂错父亲**不报错、不 panic**，面板照样画得出来，只是
   层级是错的——这是这个项目最危险的那种失败。
2. **跨刷新的身份**。`NSOutlineView` 按对象身份记展开状态，而每次刷新都会
   重建全部节点。身份是**从根到自己的 label 链**（同名兄弟按出现次序区
   分），不是 `index` 也不是 `block`：在文档最前面插一条标题会把后面每一条
   的 index 与 block 一起推后，按下标记的话展开状态会整体错位。
3. **label 的折行**。见下面「显示源码区间」。

##### 导航不另开 FFI

`DocumentTextView::navigateToOutlineItem` 把光标放到 `label_range` 的起点，
走的是 `setSelectedRange` 那条**已有**的路（落到
`yu_storage_session_set_selection_endpoints`）；滚动由随之而来的
`onCaretChange` 交给 `macosShapedCaretScrollRequest`，也就是
`yu-editor::viewport` 那条路。**面板不自己算 y**——它手上只有 UTF-16 偏移，
算 y 就要在平台侧复制一份排版，那正是 v2 要拆掉的东西。

##### 判据不能来自被测的那条路

「面板的条数与 FFI 一致」是自证的：面板本来就是照着那个数组画的。
`--outline-panel-self-check`（headless，`NSOutlineView` 与
`DocumentTextView` 一样不需要窗口也不需要 run loop）断的是另外四件事：

1. **树的形状**反过来核对平表——每个孩子的 `parent` 等于父节点的 `index`、
   根节点的 `parent` 必须是 `UInt32.max`、前序遍历恰好给出 0..n-1。「挂错
   父亲」与「静默地把孩子提成根」都在这条下面。
2. 点第 N 行之后**光标落在第 N 条标题的正文起点**，判据来自
   `bridge.selection`，与面板走的是两条路。
3. 那之后**滚动请求指向那一条的块**。这是两份派生视图的交叉核对：大纲报的
   `block` 与 viewport 那条路报的 `block_index` 必须是同一个答案。
4. 在**文档最前面**插一条标题（把每一条的 index 与 block 一起推后）之后
   刷新，**展开状态与选中行不丢**。在末尾追加字符压不住这一条——那种编辑
   谁都活得下来。

headless 压不住的只有一条：**滚动真的发生了**。那里没有 scroll view，
`revealCaretIfNeeded` 一进门就返回。它挂在
`--launch-window-self-check`（`Fixtures/outline.md`）的第 6 步上，实测
`0 → 867`。

反向验证做了八个变异，全部变红：挂到上一条而不查 `parent`（1）、
`displayLabel` 不折行（label 断言）、identity 改回按 index（4）、刷新时不
恢复展开状态（4）、不恢复选中行（4）、导航落在块首而不是正文起点（2）、
`OutlineItem::block` 加一（3，隔离出交叉核对那一条）、
`revealCaretIfNeeded` 忽略 `needsScroll`（真实窗口那一条，选区仍然正确、
只有滚动变红）。

##### 显示源码区间：这一刀不剥，只折行

`## **粗** 标题` 在面板上显示成 `**粗** 标题`，理由与触发条件见上面「已登
记：面板上的标题带着行内标记，第三刀再剥」，这一刀没有改变那个判断。唯一
的纯呈现例外是 **Setext 多行标题**：`多行\n标题\n===` 的 label 是
`"多行\n标题"`，一行放不下两行字，Swift 侧折成一行。折行只动空白，不动
任何标记。

##### 顺带记下：约束给不出分栏的初始宽度

`NSSplitView` 给 subview 0 加的 holding priority 压过 `.defaultLow` 的首选
宽度约束，光靠约束面板会缩到 min。初始位置只能在 `viewDidAppear` 里显式
`setPosition` 一次；min/max 仍由约束兜住，用户拖动照常。这条是真实窗口截图
抓出来的——headless 与自动化断言全绿，面板只是「窄了 70 点」。

##### 已登记的闸门没有动

`> # 引用里`、`- # 列表里` 仍然产出 0 条大纲。`Fixtures/outline.md` 里留了
一条 `> # 容器里的标题`，**没有**给它写断言——写了就等于把这个偏差焊死，
而它是「块的边界还没合并」那道闸门后面的事。人工验收清单 D3 记着它。

#### 第三刀：搜索

三个设计问题在开工前各复核了一遍，两处证据是自己回代码里查的。

##### 一、搜索高亮不是装饰，与选区同形状

上一刀留下的预判是「搜索与第十三刀（引用表接进装饰阶段）是同一个形状，
缓存要按查询失效」。**查下去不成立**，两处证据：

1. `yu_core::TextAttrs` 只有 `style` 与 `size_scale`，**没有背景色**。三张表
   里没有一张能表达「一段文字底下画一块颜色」。
2. 选区早就有答案，而且**不在装饰里**：`yu-workspace` 从一个源码区间加一份
   `BlockLayout` 直接产出 `EditorDecorationPrimitive{ role: Selection }`，
   场景层画矩形，`DecorationCache` 与 `DecorationSet` 全程不参与。

所以加了 `EditorDecorationPrimitiveRole::{SearchMatch, SearchCurrent}`，走选区
那条路，**`DecorationCache` 一个字节都不用清**，第十三刀那道引用表指纹不用抄
第二遍。「哪里一样、哪里不一样」的答案是：引用表改变的是**块的语义**
（`[a][b]` 到底是不是链接、要不要藏定界符），所以必须清装饰；查询改变的只是
**画在文字底下的矩形**，不改任何块的语义、不藏任何 source。

这条边界此前没有人写下来过——选区是它的第一个占位者，在只有一个的时候它看
上去像一个没解释的例外。现在写进了 `invariants.md` 的 D1 下面：**D1 管的是
文字自己**，判据是「文档的字节流变了它会不会变」。

**顺带记一笔证据**：`EditorDocument` 那堆「视图与缓存」的字段从 3 个变成 4 个
（`search`）。**这一刀仍然不抽 `EditorState`**——它与那三个一样是缓存，抽不
抽都跟着走。真正逼出它的是多光标那一刀。

##### 复核时补上的一项：帧身份

三条建议里没有提到 `MacosFrameKey`。它的文档写死了一句话——「新增一种不推进
Revision 的可视状态时必须同时加进来，否则它的变化会被静默跳过」。**查询正是
这样一种状态**：换查询不推进 Revision、不改几何、不改选区。不加进去的表现是
**在搜索框里打字，画面一动不动**——不报错、不 panic。

于是 `EditorDocument` 多了一个 `search_generation`（换一次查询加一），帧身份
带上它，守护断言照着「光标移动必须让帧身份改变」那一条的形状写。

「当前命中换了一个」**不用单列一项**：它是从选区推出来的，而选区已经在帧身份
里了。

##### 「当前命中」不存下标

一个搜索状态最自然的写法是 `{ query, matches, current: usize }`。这里没有
`current`：`SearchState::current(selection)` 由选区推出来，要求选区**恰好等于**
那一段。

理由是只留一份真相。存一个下标，它与选区就是两个可以对不上的答案：用户在文档
里点一下、撤销一次编辑，下标都不会跟着变，于是「当前」停在别处——不报错，只是
指错了地方。而「跳到下一个」本来就要走已有的选区入口（导航只能有一个实现），
选区因此**必然**是被更新的那一份。

##### 二、回报区间的 FFI：按块提问，只回报区间

第二刀登记的那条闸门（「面板上的标题带着行内标记，触发条件是搜索面板」）
这一刀结掉了。`yu_storage_session_block_hidden_spans` 交出的是**区间**，不是
文本——后者会破 C4「parser 不复制正文」与整套 range-backed 设计，而平台手上
本来就有 canonical 镜像，减一下就行。

三个容易搞错的点都核实过：

1. **`active` 不参与。** 走 `EditorDocument::block_decorations`（无光标露出的
   那条缓存路径），不是 `block_decorations_for_visual_state`。否则光标停进某条
   标题的 `**` 里，面板上那一行会跟着长出 `**`。
2. **按块提问。** 请求区间必须整个落在这个块里，否则拒绝。跨块入口会逼这一层
   去回答「块边界在哪」，那是上一层的事；而放行的后果是**静默地少藏**。
   搜索结果那一行可能跨块，所以 `YuStorageSearchMatch` 带上了块的区间，让调用
   方自己把行裁进块里。
3. **唯一实现比建议说的还上面一层。** 建议写的是 `DecorationSet::hides_source`，
   实际上 `DecorationSet::hidden_spans()` **已经存在**，而且已经合并成升序、
   不重叠、不相邻（映射索引就建在它上面）。FFI 只做裁剪与 UTF-16 换算，连合并
   都不用抄。

**判据的分工**：「藏对了没有」不在这一刀证——那是 `yu-decoration/src/hidden.rs`
的线性参照与 `extension_decorations.rs` 那 45 条压住的事。这一刀真正的新逻辑是
Swift 侧「拿镜像减区间」，它的判据是**性质**：结果长度等于原长减去藏掉的总长、
结果是原串的子序列、区间有序且不重叠且不越界；外加一组手造的畸形输入（逆序、
重叠、越界、负长度），它们必须整段原样返回——显示源码是一件真事，按一组自相
矛盾的区间去减不是。

减法住在 `PanelLabel`，**两个面板共用**：各写一份必定分叉，表现是同一段文字在
两个面板上不一样。也**没有**拿「两个面板显示同一个字符串」当判据——两边都从这
一份定义来，那是自证的。

##### 三、结果面板与大纲面板各写各的，只共用导航

数过 `OutlinePanel.swift` 的构成：8 个 `NSOutlineViewDataSource/Delegate` 方法
`NSTableView` 一个都用不上；`reload` 有一半是展开状态恢复，而搜索结果没有展开
状态；平表→树与跨刷新的 identity 链完全不需要——结果是平的一列，每换一次查询
整体重建，条目的身份就是「第几个匹配」。

所以各写各的。**第二个消费者到了，但它要的不是同一样东西。**

共用的是两样，各自只能有一个实现：

- **导航**：`DocumentTextView.navigateToOutlineItem` 泛化成
  `navigate(toSource:)`，收一个 UTF-16 区间。大纲那个入口退化成一行转发，
  搜索走 `navigateToSearchMatch`（**选中**那一段，因为「当前命中」要求选区恰好
  等于它）。滚动仍然由 `onCaretChange` 交给 viewport 那条路。
- **`PanelLabel`**：上面说的减法。

##### 区分大小写：这一版就是区分的，理由与 F3 同形

字面、区分大小写的子串匹配。不区分大小写要么改变偏移（`to_lowercase` 会让某些
字符变长），要么逐字符折叠——那是这个仓库里的**第二份 case folding**，与已登记
的 F3（引用标签只做 simple lowercase）同一个形状。**登记，等 F3 接受那个外部
依赖时一起做。**

##### 判据落在哪

- `yu-editor::search` 的单元用例：匹配不重叠、字节偏移在多字节文本里对得上、
  区分大小写、「当前」要求恰好相等（全选不点亮每一个）。
- `yu-workspace` 的场景用例：矩形**真的进了场景**，当前命中与其余分得开，收掉
  搜索一起消失，**IME 组字期间整个跳过**（那时视觉字节流带着 preedit 覆盖层，
  匹配是按 canonical 源码算的）。判据来自场景，不来自 `SearchState`。
- `yu-storage-ffi`：UTF-16 换算、块下标与块区间、编辑之后重扫、Revision 校验、
  两遍协议；以及帧身份那三条。
- `--search-panel-self-check`（headless，`Fixtures/search.md`）：剥标记、上下文
  裁进块、点第 N 行选区落在第 N 处命中、环回、选区离开命中后高亮消失、编辑之后
  重扫、0 结果不是错误。
- `--launch-window-self-check` 第 7–9 步：**搜索高亮真的进了屏幕上那一帧**
  （headless 没有 surface，场景根本不提交），收掉之后消失；外加人工验收 D4 里
  能自动化的两条（菜单项的勾、收起面板后焦点不留在面板上）。实测 `search=1→0`。

##### 反向验证

Rust 侧 20 个变异、Swift 侧 14 个，全部变红。三个活下来过：

- **「越界的块下标不拒绝」**活了一次——因为那条用例拿 `0..0` 去问，而那个区间
  对任何块都跨界，于是外面那道「请求要落在块里」的门先挡下来。**两道门挡同一
  件事时，里面那道往往没有自己的用例。** 改成拿最后一个块自己的区间去问才隔离
  得出来。
- **「拿不到区间时返回空串而不是源码」**活了一次——self-check 里 Revision 从不
  失配，那条退路没有用例。补了一条直接断在 `hidden: nil` 上的。
- **「上下文不裁进块」**活了一次，而它**不是**死代码。块的边界今天按行划，
  所以语料造不出「行超出块」的情形；但 AppKit 的 `lineRange(for:)` 认的是
  Unicode 行边界（`\u{2028}` 也算）而块扫描器只认 `\n`，而且「块的边界还没
  合并」那道闸门一开这件事就成立了。**没有当等价变异放过**：用一条手造输入
  （块比行窄）单独压住，理由写在 `SearchResults.contextRange` 上。

##### 顺带记一笔：变异脚本被超时中断之后，进程还活着

前台命令被 harness 超时中断，报的是「命令超时」，但那个 `cargo test` 的父进程
没有死。后果有三层：它继续按自己的节奏改同一批文件，于是后开的那一轮报出莫名
的 `SKIP`；两个 `cargo test` 互相等全局构建锁，看上去像卡死；最后一轮
`finally` 写回的是一份**带着别人变异的快照**，在 `yu-workspace/src/lib.rs`
末尾留下了一对多余的花括号。判据是 `pgrep -lf mutate` 与
`ps aux | grep "[c]argo test"`——**中断之后一定要看一眼，不要只看 harness 的
报错**，而且被污染的那一轮结果全部作废、要重跑。

##### 截图抓出来的四件事

自动化断言全绿、真实窗口 self-check 也全绿之后，人工验收的截图仍然抓出四个
缺陷。前两个是**渲染**，不是面板样式：

1. **底色的高度用错了行高。** `line_height()` 是**基准**行高；v2 的行高住在
   行盒里（`caret_line_height` 的文档早就写了这件事，caret 一直是对的）。
   于是标题那一处的底色又矮又靠上，压根没盖在字上。**选区从 S3 起就是这么
   画的，而没有任何断言压着它**——这一刀顺手修掉，并补上「矩形高度等于那一
   行的高度」的断言（语料里标题行必须比基准行高更高，否则那条断言压不住任何
   东西）。
2. **「当前命中」画在选区之下会变成一块脏灰。** 当前命中按定义就是选区那一
   段，把橙色垫在半透明的蓝下面，合成出来是 `(158,160,150)`。改成**三明治**：
   其余命中在选区之下，当前命中在选区之上、caret 之下。这个次序归
   `append_editor_decorations` 一家排——caret 是它末尾才发出的一层，把次序拆
   到调用方去排就会把 caret 盖掉。断言落在图元的先后上。
   顺带定下 alpha：底色画在字形**之上**，太不透明会把文字盖住（235 不行，
   140 可以）。
3. **搜索面板没有背景。** 裸 `NSView` 是透明的，查询框像浮在侧栏中间。换成
   `NSBox`（`fillColor` 收动态 `NSColor`，深浅色切换时自己重绘；写死一个
   layer 背景色不会）加一条顶边分隔线。
4. **侧栏里两个面板按固有宽度居中，没有撑满。** 竖直 `NSStackView` 的默认
   `alignment` 是 `.centerX`，而 `.width` 是按「最宽的那个」对齐——
   `NSScrollView` 根本没有固有宽度，两种都让查询框和结果列表缩成窄窄一条，
   结果那一行被直接裁断、连省略号都看不见。宽度只能显式钉在侧栏上。
   **这与陷阱 26（约束给不出 `NSSplitView` 的初始分栏位置）是同一类**：布局
   容器的默认策略与你以为的不是一回事，而它不报错。

四条都不报错、不 panic，全部自动化断言都绿。这一节存在的理由就是它们。

##### 代价

Swift 产品代码 **5,276 → 6,061 行**（2.64 → **3.03 倍**），分布是
`SearchPanel.swift` 302、`PanelLabel.swift` 75、`DocumentWindow.swift` +247、
`StorageBridge.swift` +127、`DocumentTextView.swift` +21、`OutlinePanel.swift`
+9、`main.swift` +4。`SelfChecks.swift` 1,068 → 1,368（不计）。
S1 那条未达成项继续往外走，这笔钱是显式拍过的。

#### 第四刀：多光标

四个设计问题在开工前各查了一遍代码。**这一刀没有预写的建议方案**——前三刀那些
是上一个会话查过代码之后写的，凭印象编出来的建议会让人把未经验证的判断当成
已验证的。

##### 先说查代码推翻的一个预期：事务层已经是多光标就绪的

`yu-text` 的 `Transaction::new` 收的就是**一组** `Edit`；`prepare` 先排序、
再 `validate_edits`、产出**一份** inverse。于是「三个光标同时打一个字」是一个
Transaction、一次 `history.record`，连续输入照样并进同一个 group——**`history.rs`
一行都没改**。原本预期要给它加「一次命令 N 条 edit」的概念，不用。

`source_change_from_applied` 也已经取全部 change 的**并集**，Swift 镜像的增量
同步对 N 条边本来就是对的（偏保守，不会漏）。

但同一处给出一条硬约束：`validate_edits` 不但拒绝重叠，还拒绝两个**空** edit
落在同一个偏移。两个相邻光标各按一次退格就会撞到同一点。**「不重叠」不是洁癖，
是一条不满足就直接报 `OverlappingEdits` 的前置条件。**

##### 一、一组选区：`Selections` 新类型，不是裸 `Vec + primary`

`Selections { revision, ranges: Vec<EditorSelection>, primary }` 住在 `yu-state`，
元素仍是 `EditorSelection`（那 130 行一个字没动，`Copy`/`Hash` 都还在）。三条
理由：

1. **不变式要有一个执法点。** 裸 `Vec` 会把「谁负责合并」散到 `EditorDocument`
   里那几十处赋值上，每一处都要记得合并一次。
2. **`revision` 不能有 N 份。** `EditorSelection` 自带 revision，`Vec` 就是 N 份
   必须相等的值 = N−1 个可以对不上的机会。这里只存一份。
3. **映射之后必须合并，而合并只能有一个地方。** `ChangeSet::map_anchor` 逐个偏移
   独立映射，两个不同偏移**可以映射到同一个**（删掉它们之间的文字）。
   `Selections::map_through` 是全仓唯一的收敛点。

不变式：至少一个 / 同一 revision / 有序 / 互不重叠 / primary 在界内。「互不重叠」
比 `validate_edits` 严一点：相邻两条 `prev.end() <= next.start()`，等号成立时两条
都必须非空。**这半句是这条规则的全部重量**——`aa` 在 `aaaa` 里的两处匹配就是
`0..2` 与 `2..4`，并掉等于把「选中全部匹配」变成「全选」；而一个停在选区边界上
的**空**光标要并进去，否则打字会把同一个位置插两次。

`preferred_x` 不进 `yu-state`：它是 f32 视觉列，而那个 crate 的模块文档把边界
画在「一个布局或投影类型都没有」。代价见下面第三条。

##### 二、`EditorState` 仍然不抽，但理由必须换掉

前三刀的理由是「那几个缓存字段跟着走」。这一刀动的是 `selections`，那是真的文档
状态，旧理由不成立了。

新理由是**这一刀没有给它带来第二个消费者，而拆开会让借用变难而不是变容易**：
`selection_reveal_block_index` / `selection_reveal_range` /
`block_decorations_with_selection_reveal` 三个函数同时要读 `selections` 和
`&mut decorations`，拆成两半之后它们只能改成收两个参数的自由函数，或者仍然挂在
`EditorDocument` 上——后者等于没拆。真正让人想抽的是「几十处 `self.selection = …`
的重复」，而那个重复的解药是 `Selections`，不是把结构体切两半。

**而且这一刀真正的风险是把两件事叠在一起**：那几十处赋值正是本刀要逐条重写的
地方，同时搬家会让变异验证分不清「红是因为多光标改错了，还是因为搬家搬错了」。

**触发条件改写**：原条件是「那三个缓存字段碍事了」。新条件是——**出现第二个需要
「一份完整可复制的编辑状态」的消费者时**（协作编辑的快照，或第七刀跨平台第二端
要在两个 host 之间传状态）。缓存字段的多少不再是判据，第三刀与第四刀都证明了它
跟着走不碍事。

##### 三、哪些一次全改，哪些留 primary 降级

判据：**这条路上「多个」是有意义的吗？** 没有意义的是 primary，有意义但这一刀
不做的是欠账。

**一次全改**（做不到就是静默出错）：编辑之后的映射、insert/delete/newline 的 N 条
edit、场景层的 N 块选区底色与 N 根 caret、`MacosFrameKey`、
`accessibilitySelectedTextRanges`、Swift 的 `setSelectedRanges`。

**primary 降级，且不是欠账**（平台 ABI 或问题本身就是单数）：
`AXSelectedTextRange`（复数是另一个属性，不是降级）、
`macos_shaped_caret_scroll_request`（滚到刚动过的那一根）、`SearchState::current`
的入参、行内语法露出（`visual_text_with_reveal` 收的就是一个 `Option<TextRange>`，
N 个块同时露出是装饰层的接口变更，与多光标无关）、表格单元格导航。

**primary 降级，是欠账，写在代码上**：

- **IME**：`CompositionOverlay` 是一个 preedit 覆盖一个区间，`NSTextInputClient`
  也只给一个 marked range。`begin_composition` 塌回 primary。**塌是必须的**——留着
  N 条而只有一条在组字，屏幕上会有几根不动的假光标，提交之后还会被映射到莫名其妙
  的位置。还债条件写在 `begin_composition` 上：要让 commit 把已确定的文字在其余
  每一处也插一遍，H1/H2/H6 三条不变量都要重过一遍。
- **纵向移动的粘滞列**：N>1 时每个光标用自己**当前**的 x，不吃粘滞列。代价是连按
  ↓ 穿过一行短行会左漂；N=1 时行为完全不变。还债条件：做「⌥⌘↑ 在上方加光标」时
  本来就要按光标存列，那时建 `Cursor { selection, preferred_x }`。
- **列表类编辑**（Enter 的续行与空项退出、空列表项退格、缩进/反缩进）：N>1 时
  Enter 只插普通换行，缩进只作用在 primary 那一行。理由不是省事——**这四条改的
  都是「整行」，而两个光标可以停在同一行上**，「空项退出列表」删的是
  `line.content_range()`，两个光标同一行就产出一对完全重叠的 edit，整条
  Transaction 被拒，表现是「按一下 Enter 什么都没发生」。理由与还债条件写在
  `insert_plain_newlines` 上。

##### 四、入口：两个，各自压住对方压不住的东西

- **`⌘⇧L` 选中全部匹配**：`SearchState::matches()` 给的就是有序、互不重叠的一组，
  恰好是 `Selections` 要的形状。**但它压不住合并**——这条路产出的选区必然合法。
- **⌥ 点加一根光标**：走 `setSelectedRanges` 那条已有的路。**它才压得住合并**，
  也是人工验收里唯一能手动构造多光标的形状。

导航泛化成 `navigate(toSources:primary:)`，单数那个退化成一行转发。

##### 落点是算出来的，不是映射出来的——这一刀查出来的真缺口

第一版让编辑之后的落点走 `map_through`。在 `aaaa` 里选中两处 `aa` 再打一个字，
两根光标并成了一根：`map_anchor` 在两条 edit 首尾相接时，把前一条的**终点**
（选区终点按 `Affinity::After` 映射）一路推到后一条替换之后，于是两条选区映射
之后重叠，`Selections` 合并掉。**源码是对的，屏幕上却只剩一根光标，不报错。**
而「选中全部匹配再替换」正是这一刀最主要的用法。

修法不是改 `map_anchor` 的端点语义（那会动到所有 anchor 的映射），而是
`apply_selection_edits` 里**根本不必去猜**：这些 edit 是这条命令自己造的，第 i 条
之前的累计位移直接加得出来。`map_through` 仍然跑，它服务的是外来的 Transaction
（undo/redo、缩进、任务勾选）。

##### 帧身份：整组比较，不要摘要

`MacosFrameKey.selection` → `selections: Vec<EditorSelection>`，类型从 `Copy` 退成
`Clone`。把 N 条哈希成一个 u64 能保住 `Copy`，代价是碰撞——而碰撞的表现正是这个
类型的文档明令要防的那件事：**静默跳过一帧**。一次 `Vec` 分配换掉一个不报错的
漏画，这笔账不用算。

守护断言分两条：**位置变**（原有的光标移动）与**根数变**（新增）。只比 primary
的话，从一根变三根会被判为等价——按下「选中全部匹配」，画面一动不动。

##### 第三刀留下的四处

- **`SearchState::current` 签名不改**，场景层改传 primary 的区间。模块文档那条
  「不存下标」的理由**不推翻，反而更强**：全部选中之后每一条选区都恰好等于一处
  命中，按「有没有某条选区等于它」判会让**每一处**都变成当前命中——正是那份文档
  担心的「全选点亮每一个」换了个形状回来。primary 不是第二个可以对不上的下标，
  它是 `Selections` 自己的一部分。
- **场景层两处**：选区与 caret 都改成遍历，caret 收集成 `Vec` 在末尾统一发——
  次序（其余命中 → 选区 → 当前命中 → caret）仍是这一层的唯一权威。
- **`SearchResults.next(after:)`** 游标改成 primary，与上面同源。
- **`DocumentWindow` 四处** `bridge.selection.range` 全部按 primary，并补了一条
  「面板导航之后选区只有一条」的断言。

##### 顺带记下：`NSTextView` 对不连续选区没有「主」的概念

第一版让 `navigate(toSources:primary:)` 把 primary 摆进 AppKit 镜像、再由
`setSelectedRanges` 从 `selectedRange()` 认回来。**认不回来**：AppKit 转手一次
之后 primary 一律退回第 0 条，⌥ 加的那根不会成为主光标，滚动与「当前命中」跟着
错。选区的权威在 Rust（不变量 I6），primary 直接送过去，镜像跟着走。

##### 判据落在哪

- `Selections` 的单元用例：性质（有序、不重叠、非空、primary 跟着合并走）+ 手造
  畸形输入（逆序、重叠、同偏移两个空选区、越界）。
- `yu-editor/tests/multi_cursor.rs`：**「N 条 edit 真的都生效了」的判据是 canonical
  source**，不是选区；「另外几个光标也动了」的判据是全部选区，不是主选区。
- `yu-workspace`：**判据来自场景里的图元条数**，不是 `document.selections()`。
- `yu-storage-ffi`：两遍协议、边界上的归一化、方向保留、帧身份两条。
- `--multi-cursor-self-check`（headless，`Fixtures/multi-cursor.md`）。
- `--launch-window-self-check` 第 10 步：⌥ 点的**坐标→源码**那一步（headless 没有
  已发布的 viewport 几何）、N 根 caret 真的进了屏幕上那一帧。

##### 反向验证

Rust 侧 25 个变异、Swift 侧 7 个，全部变红。**五个活下来过，逐个分类**：

- **「不滤掉空 edit」活了一次，它是缺口。** 一条零效果的 edit（`[0,0)` 换成 `""`）
  **照样推进 Revision**——`TextBuffer::apply` 不认「这条什么都没改」。不滤的后果是
  在文首按一次退格，文档变脏、压进一条什么都不做的 undo，而源码一个字节没变。
  这个契约在多光标之前就有（`delete_range` 开头那句 `if range.is_empty()`），
  **一直没有断言**。补了一条直接断 Revision 的。
- **「当前命中按任意一条选区判」活了两次。** 第一次是判据没落在被改的那条路上
  （用例只有一条选区，「任意」与「primary」是同一条）。补上多选区之后**又活了
  一次**——因为 `current` 是一个 `Option<usize>`，改法只是让它指错一处，条数仍然
  是 1。第三版把判据换成**那一处的 source 区间**才压住。「只数条数」与「数对了
  哪一个」是两回事。
- **「`set_selections` 接受空集合」活下来，它是死代码。** 那道门与
  `Selections::new` 挡的是同一件事，而里面那道先挡住了。按规矩删掉——保留就是
  给同一条规则留第二个答案，而且它永远不会被单独证伪。
- **Swift 的「`setSelectedRanges` 只送第一条」活下来，是缺口。**
  `navigate(toSources:primary:)` 改成直接送 Rust 之后，那个 override 的复数分支
  就没人走了；它真正的消费者是 **AppKit 与 AX**（给 `AXSelectedTextRanges` 赋值）。
  补了一条直接调它的 self-check 步骤。
- **Swift 的「⌥ 点不经投影」活下来，是缺口。** 断言只要求「多了一根、位置不同」，
  而把坐标→源码换成常数 0 也满足——⌥ 点到哪里光标都跑到文首。改成断**落点离
  点击处不超过 1 个 UTF-16 单位**；精确的边界归属归
  `--shaped-projection-hit-test-self-check`，不在这一步重证。

##### 截图抓出来的一件事：被选区盖住的命中变成脏灰绿

自动化断言全绿、真实窗口 self-check 也全绿之后，人工验收的截图抓出一个缺陷——
**与第三刀抓到的是同一族颜色**：

「选中全部匹配」之后，每一处命中同时也是一段选区。第三刀把「其余命中」排在选区
**之下**，理由是「选区是半透明的蓝，压上去两者都还看得见」——**那句话的前提是
选区与那些命中不是同一段**。多光标之后前提没了：黄底垫在半透明蓝之下合成出一块
脏灰绿，与第三刀在当前命中上实测到的 `(158,160,150)` 同一族。屏幕上两处选中的
文字一处褐橙、一处灰绿，没有一处看着像「选中」。

修法不是再排一次次序，而是**被选区完全盖住的普通命中干脆不画**：选区已经说明了
「这一段是命中，而且被选中了」。当前命中不受影响——它排在选区之上，而且按定义
就等于选区那一段，所以「哪一根是主光标」仍然看得出来。

判据落在场景图元的条数上（`SearchMatch` 必须为 0），而查这一条要**两条选区都
恰好落在命中上**，单选区的用例造不出这个形状。

顺带确认了人工验收新加的 C12：三根光标分别落在标题块、段落块与列表项里时，
三块底色与三根 caret 的高度**各自跟着自己那一行的行盒**，不是同一个基准行高。

##### 代价

Swift 产品代码 **6,061 → 6,356 行**（3.03 → **3.18 倍**），分布是
`DocumentTextView.swift` +117、`DocumentWindow.swift` +109、
`StorageBridge.swift` +65、`main.swift` +4。`SelfChecks.swift` 1,368 → 1,511
（不计）。Rust 侧新增 `yu-state/src/selections.rs`（一个类型加它的性质用例）与
`yu-editor/tests/multi_cursor.rs`。

比前两刀便宜得多，原因写在上面：**事务层与 history 一行都没改**，Swift 侧也没有
新面板——多光标是既有那几条路各自从「一个」变成「一组」，不是一块新 UI。

---

#### 第五刀：代码块高亮（tree-sitter）

五个设计问题在开工前各查了一遍代码。**交接稿给的四条「已核实事实」里有三条要
修正**，先说这三条——它们各自省掉或改变了一整块工作。

##### 查代码推翻的三个前提

**一、语言标签已经在了。** 交接稿说「`BlockKind::FencedCodeBlock { marker,
closed }` 不带语言标签」，对；但由此推出的两个候选答案（补进 `BlockKind` /
另开一条查询）**都不需要**。`extension/fenced_code.rs` 早就算出了语言名与正文
的区间，并且作为 `BlockOrnament::FencedCode { info, content }` 发出来了
（`extension/mod.rs:331-337`），消费者是 `yu-storage-ffi::fenced_code_of`——
KaTeX / Mermaid 那条路按语言名决定这个块渲染成什么。着色是同一个块上的第三个
消费者，不是第一个。

`BlockKind` 保持不动另有一条独立理由：它是 `Copy + Hash`，而且是
`DecorationCache` 复用条目的键之一（`decorations.rs:177`）。

**二、`tools/check-deps.py` 不检查外部依赖。** 它的 docstring 第 12–13 行写着
「外部 crate（ropey、comrak…）不在此列，那是选型问题，不是分层问题」，
`parse()` 的正则只收 `path = "` 且名字以 `yu-` 开头的行。全仓也没有第二处外部
依赖门禁。白名单最后确实改了，但**是因为多了一个 crate，不是因为多了一个外部
crate**。

**三、tree-sitter 不是「这条线上的第一个外部依赖」。** `yu-layout` 有三个
（unicode-segmentation / linebreak / bidi），`yu-editor` 有 unicode-segmentation，
`yu-text` 有 ropey，`yu-syntax` 有 unicode-general-category。**零外部依赖的只有
`yu-markdown` 一个**——F3 那条欠账说的正是它。

##### 一、颜色从哪一层进来：D1 的判据自己指着装饰这条路

交接稿最贵的那条事实成立：从装饰到场景批次，**没有任何一层带着「这个字什么
颜色」**，一帧只有一个正文色，由 `SceneBuilder::append_viewport` 的 `color`
参数一次性发给所有字形（`yu-scene/src/lib.rs:894`）。

但「像第三刀那样绕开装饰」这个选项**被 D1 自己的判据排除了**。第三刀刚给 D1
划的那条边界原文是：

> 判据是「文档的字节流变了它会不会变」：藏 `##` 会跟着源码变，所以是装饰；
> 一块选区底色只跟着选区变，所以不是。

代码高亮跟着字节流变，改的是文字自己的视觉表现。**它在 D1 里面。** 第三刀划出去
的是「非文档状态驱动的矩形」，不是「改颜色」——这一刀是那条边界第一次被反方向
用到。

「另开一张表」也被排除了，理由在代码里：`yu-editor::marks::winner_over` 是
**最窄的 Mark 赢，而且只赢一个**（`marks.rs:87-97`），Mark 不叠加。代码块整段
有一条 `Code` 的 Mark，token 的 Mark 更窄会把它整个盖掉——所以 token 那份属性
必须**同时**带着字型与颜色，也就是必须是同一张表的同一个值。第二张表解决不了
这件事，只会多一份「谁盖谁」的规则。

选法：**`TextAttrs` 加一个 `role: TextRole`，不是 `Rgba8`。**

- 产品选色不下沉到 `yu-markdown`。仓库已有的规矩写在
  `yu-workspace/src/lib.rs:97`：「产品选色住在这一层，不住在场景层」，
  `EditorDecorationStyle` 的搜索底色也是从平台传进来的。写死颜色等于把主题焊进
  解析层。
- `yu-layout` 拿到的仍然是「等宽、1.0 倍、Keyword 角色」，拿不到「这是 Rust 的
  `fn`」——不变量 E1 在这一层的落法不变。

**这一刀便宜的原因是样式身份本来就没丢**：`GlyphBox` 一直带着 `StyleId`
（`yu-layout/src/block.rs:576`），`BlockGlyph` 带着已经解释过的 `TextStyle` 与
自己的 source（`blockview.rs:114-123`）。丢掉它的只有 `yu-workspace` 转
`SceneGlyph` 的那一步。所以颜色不需要新开一条通路，只要**不丢**：
`BlockGlyph` 多一个 `role`，`SceneGlyph` 多一个 `Option<Rgba8>`，
`collect_glyphs` 里 `placement.color().unwrap_or(color)`。

`Option` 不是「每个字形都带一份颜色」：**没有覆盖**与**覆盖成正文色**是两件不同
的事，前者跟着主题走；而 `None` 让「上一层忘了传颜色」退化成现状，不是一片黑。

##### 二、两棵树怎么共存：**不共存——第二棵树活不过一次 `decorate()`**

`yu_highlight::Highlighter::spans` 里建树、遍历、丢掉。跨调用留下来的只有结果。
于是「谁拥有第二棵树、跟着谁失效」这个问题没有内容：它不存在到下一次。

**tree-sitter 自己的增量解析没有用上**，这是有意的：两套增量要两份「什么变了」
的答案，而块级那一份已经在 `DecorationCache` 里了（按 range + kind 复用、
`shift_through` 平移）。

但**留一条 memo 是必须的，而且这件事是量出来的**。加了一个探针数
`DecorationCacheStats::parses`：光标停在代码块里时，`block_layout_for_visual_state`
每个可见块都问一次 `selection_reveal_block_index()`，而它走的是未缓存的
`DecorationCache::decorate`（`document.rs:308`）。实测 5 个可见块的稳态一帧要跑
**5 次**未缓存产出，真实窗口二三十个可见块就是二三十次。而着色的代价（M1 Max，
release）：

| 代码块 | parse + highlight |
| --- | --- |
| ~12 行 / 260 B | 59 µs |
| ~100 行 / 2.2 KB | 428 µs |
| ~1000 行 / 21 KB | 3.4 ms |

外加 **query 编译 19–28 ms/语言**——那个必须只做一次（`OnceLock`），做在调用里
就是把这条路的代价整个颠倒过来。

所以 `yu-highlight` 带一条 memo：`(语言, 代码文本)` 精确比较。**它不是
`DecorationCache` 的第三道失效门。** 那两道（Revision + range/kind、引用表指纹）
是**正确性**的门，漏掉一次就画出一份对不上源码的东西；这一条是**代价**的门，
键就是内容本身，所以陈旧条目在定义上不存在——没有任何「什么时候该清」要回答。

**只有一条**，因为一帧里被反复问的只有焦点块那一个；其余块走 `DecorationCache`
自己的条目，各问一次。第二条要等「一帧里有两个块反复被问」真的出现。

**已登记的欠账**：单个代码块大到一次全量 parse 在编辑时看得见（按上表 ~1000 行
起，因为编辑推进 Revision，memo 必然落空）。那时才谈得上保留树、用 tree-sitter
自己的增量——那是第三份「什么变了」，要有人真的抱怨才值得付。

##### 三、语言标签：不动 `BlockKind`，也不另开 extension

着色写进 `fenced_code.rs` **自己**，因为**不变量 D6 不许 extension 互相感知**：
另开一个 extension 读不到这里的 `BlockOrnament`，就得把 info 与 content 两段区间
再算一遍。两份实现会在下一次改动时分叉，分叉的表现是颜色盖到围栏上。同一个文件
里它们是两个局部变量。

`Language::from_info` 只取 info string 的第一个词：CommonMark 允许
```` ```rust,ignore ````、```` ```js title=a.js ````。大小写只在 ASCII 上折叠
——语言名都是 ASCII，用不着仓库里的第二份 case folding（F3 那条欠账正是那件事）。

##### 四、外部依赖与分层：新建 `yu-highlight`，与 F3/comrak **不是**同一件事

交接稿把这条写成一个要验证的问题而不是判断，验证结果是**不同形状**：

| | F3（引用标签的 case folding） | 这一刀 |
| --- | --- | --- |
| 要动的逻辑在哪 | `yu-markdown` 自己的标签比较里 | 一个与 Markdown 无关的独立问题 |
| 能不能装进一个 crate | 不能，它必然是 `yu-markdown` 的直接依赖 | 能 |
| 对 `yu-markdown` 的外部依赖数 | 0 → 1 | 仍然 0 |

`yu-highlight` 住在 1 层（只依赖 `yu-core`），是全仓唯一认识 tree-sitter 的地方，
对外只有一句话：`(语言名, 代码文本) → Vec<RoleSpan>`。`yu-markdown → yu-highlight`
是一条 **workspace 边**，`check-deps.py` 管得着。

`RoleSpan` 有意**不用 `TextRange`** 装它的两个偏移：那是文档坐标的类型，拿它装
局部偏移正是 `ShapingProvider::shape` 那条已登记糊涂账的形状（它的 range 参数是
零基局部空间，看类型看不出来）。用裸 `usize` 逼调用方把「加上正文起点」这一次
换算显式写出来。

**着色用 `tree-sitter-highlight`，不自己拿 `Query` 拼。** 它的
`HighlightEvent` 是一个栈，栈顶就是最里面那一层 capture——按查询文件里的先后
再判一遍优先级会是第二份实现，而它正是这件事的参照实现（Helix / Neovim 生态用
的就是它）。`#match?` / `#eq?` 这类文本谓词也归它：忽略谓词的后果是
`((identifier) @constant (#match? "^[A-Z]"))` 把**所有**标识符染成常数。

新增的构建要求要登记：grammar crate 走 `cc` 编译 C，CI 三个平台
（`ci.yml:11` 含 windows-latest）都要有 C 编译器。另外
`build-rust-ffi.sh` 现在显式 `export MACOSX_DEPLOYMENT_TARGET=14.0`——`cc` 默认
按主机 SDK 的部署目标编译，而 `Package.swift` 声明的是 `.macOS(.v14)`，不对齐
的话每次链接刷十几行 "built for newer 'macOS' version"，真正的警告会淹在里面。

##### 五、帧身份：**不加**，这是三刀里第一次答案是「不加」

`MacosFrameKey` 的规则是「新增一种**不推进 Revision** 却改变画面的状态」。高亮是
`(源码字节, 语言)` 的纯函数，没有第二个输入：没有开关、没有主题切换、没有异步。
源码变了 Revision 就变了。`search_generation`（换查询）与选区**条数**漏掉，是
因为它们真的能脱离 Revision 变化；高亮不能。

反过来说清楚什么时候会变：如果以后高亮变成异步（大文件后台着色），它就会长出
`resource_refresh_pending` 那种形状（`SurfaceHost.swift:768` 的有界轮询），
那时才需要一个 generation。

##### 这一刀查出来的真缺口：`assemble` 把角色归零

`yu-markdown` 那一侧的断言全绿（装饰产出是对的），`yu-editor` 的字形上一个角色
都没有。查下去是 `blockinput.rs::DecorationDraft::assemble`：它给标题算字号倍率
时**从头重建**每一份 `TextAttrs`——

```rust
TextAttrs::new(style).with_size_scale(font_scale)
```

——新造的那份自然没有角色。表现是**代码块里一个字都不着色，而且不报错**：装饰对、
布局对、场景对，就是颜色没了。

这个缺口只有**跨层**的用例抓得住：`yu-markdown` 的用例断在装饰上（绿），
`yu-workspace` 的用例断在场景上（红，但它离缺口三层远）。抓住它的是
`block_view_properties.rs::code_highlight_roles_reach_the_glyphs`——判据是
**字形自己的 source 区间**切出来的文本等于 `fn` / `u32`，不是「有几个字形带
角色」。那一行现在写着为什么它是重建而不是修改。

##### 判据落在哪

四层，每一层的判据都来自它下游的产出，不来自被测的那条路：

- `yu-highlight/tests/languages.rs`：**每一条断言都把 `code[span]` 切出来跟一个
  字面量比**。每一种登记的语言各带一条 `(角色, 文本)` 期望，而不是共用一条
  「有注释就算过」——**JSON 没有注释**，共用判据会逼语料去迁就判据。外加结构
  性质（有序、不重叠、非空、不越界、落在字符边界上）、三条降级（认不出语言 /
  空 / 语法错的代码）、以及两条 memo 的用例：**用过的着色器与全新的必须给出同一
  个答案**，**同一段文本换一种语言必须换一个答案**。
- `yu-markdown/tests/code_highlight.rs`：区间搬到源码坐标之后指对了没有。
  **每一条高亮 Mark 都必须带着 `TextStyle::Code`**（`winner_over` 那条陷阱的
  断言）；高亮不许碰围栏那两行，判据是同一个 extension 的另一样产出
  （`BlockOrnament::FencedCode` 的正文区间）；光标停在块里不改变任何东西。
- `yu-editor/tests/block_view_properties.rs`：角色到了字形上，判据是字形的
  source；**外加一条反面**——没有高亮的块每一个字形都是 `Plain`，否则一个把所有
  字形都标成 `Keyword` 的实现也能过前一条。
- `yu-workspace`：判据是**场景图元的颜色**，一次都没问过 `TextRole`。主判据是
  一个差分：同一段代码一份带语言名一份不带，两份文档除此之外一个字节都不差。
- `--code-highlight-self-check`（headless，`Fixtures/render-code.md`）：同一个
  差分，但走**真的 CoreText**——字形的数量与分段都不同，颜色是不是按 run 正确
  分配只有这条路看得见。
- `--launch-window-self-check` 第 11 步：**真实 Metal surface 提交的那一帧**里
  数出的高亮字形数（实测 `highlightedGlyphs=8`）。上限那半句
  （`highlighted < commandCount`）压的是「所有字形都被刷成同一种颜色」——那种
  错法只断「大于零」是过得去的。

**「在代码块里打字，颜色跟着改」原本写在人工验收清单里，收尾时挪成了自动化**：
`--code-highlight-self-check` 把 `let` 改成 `lets`，断**高亮字形正好少 3 个**
（实测 7→4）。理由有两条：真人按键盘那条路把光标准确放进代码块不可靠（方向键
会滚动视口，合成点击驱动不了 AppKit）；而「少 3 个」这个数比「变了」强——一个
每次编辑都把整块颜色清掉的实现能过后者，过不了前者。

##### 反向验证

Rust 侧 20 个变异、Swift/FFI 侧 4 个，全部变红。**三个活下来过，逐个分类，
三个都是缺口，没有一个是等价变异**：

- **「取 capture 栈的**栈底**而不是栈顶」活了一次。** 判据落在了被改的那条路
  上，但**语料压不住它**：那几份代码里的 capture 几乎都只有一层，栈底与栈顶
  是同一个值。补了一条专门造两层结构的用例（模板串 / f-string / `"$HOME"`：
  外层是整段字符串，里面套着定界符与被插进去的名字），并且先断「同一段代码里
  确实存在外层的字符串角色」——否则语料退化成一层，用例自己就废了。
- **「memo 的键里没有代码文本」活了一次，也是语料的问题。** 第一版的两份 Rust
  语料是 `fn a() { let x = 1; }` 与 `fn b() { let y = 2; }`——标识符都是一个
  字符，两份的 `RoleSpan` **逐个字节完全相同**，拿上一份的答案回报比出来一样。
  换成长度与角色都不同的三份之后变红。**这与第四刀「当前命中」活两次是同一类
  错**：判据在路上，但输入造不出差别。
- **「给列表标记的字形也编一个角色」活了一次。** 那是全场唯一一条不查样式表、
  直接写死角色的路（标记的 `•` 不在 source 里，没有任何 Mark 盖着它），而反面
  用例的语料里只有**任务项**——任务项按设计不换替代标记，所以根本没有标记字形。
  加一条普通列表项之后变红。
- **「调色板给所有角色同一种颜色」活了一次。** 场景层的判据是「颜色种类 > 1」，
  而正文色也算一种：全部刷成红色时是「黑 + 红」两种，照样过。改成**数正文色
  之外还有几种**（≥ 2）之后变红。

**一条一般式留着**：`macos_highlighted_glyph_count` 拿 `config.color()` 比而不是
写死黑色——当前配置里那两个值相同，任何用例都分不开。留着它并把理由写在那一行
上：配置换了颜色而这里没跟着换，表现是「整篇文档都被算成高亮」，那是一个不报错
的假绿。

##### 已登记

- **深色模式不在这一刀里。** 调色板是一份写死的浅色，与
  `viewport_block_background` 挑同一块底。整个编辑区现在就是浅色的：
  `macos_render_host_config` 把背景写死成 `Rgba8::white()`，第四刀的人工验收
  记录里 D2 已经登记了这条（深色外观下面板变深而文档区仍是白底）。这一刀跟着它
  走——单独给高亮开一条深色路会造出「深色的代码配白色的底」。
- **每个 `StyledRun` 单独 shape**（`yu-layout/src/block.rs:1710`），token 化把
  一个代码块的 1 次 shaping 变成 N 次。连字不跨 run 边界，等宽编程字体里
  `->`/`!=` 这类连字如果被 token 边界劈开会画成两个字形。实测语料里没有出现，
  因为这些符号本来就是一个 token。
- **内嵌语言不下钻**（Rust 文档注释里的 ```` ```rust ````、JS 模板串里的 SQL）：
  `injection_callback` 一律给 `None`。少的是内嵌那一段的颜色。
- **括号与分号不着色**：全部着上之后代码看着像圣诞树，而它们本来就靠形状区分。
- **五种语言**：bash / javascript / json / python / rust，含常见别名。加一种是
  三处（枚举变体、别名、grammar），`tests/languages.rs` 要求每一个变体都真的着
  上色，所以漏掉后两处的任何一处都会红。

##### 代价

Rust 侧新增 `crates/yu-highlight`（一个 crate，397 行含文档）加两份用例
（288 + 258 行）。
Swift 产品代码 **6,356 → 6,415 行**（3.18 → **3.21 倍**），分布是
`DocumentWindow.swift` +53（第 11 步）、`StorageBridge.swift` +6（两个镜像字段）。
`SelfChecks.swift` 1,511 → 1,690（不计）。**这一刀是 S7 里 Swift 侧最便宜的
一刀**——高亮没有新 UI，它是既有那条「装饰 → 布局 → 场景」的路上多带了一样
东西。

真正的代价在构建：tree-sitter 加五个 grammar 的冷编译约 **2 分 34 秒**，静态库
里多出十来个 C 目标文件。

---

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
| ropey | Rope 实现，直接依赖（`2.0.0-beta.1`，全字节索引） | MIT OR Apache-2.0 |
| comrak | CommonMark 正确性 oracle 与 HTML 导出，直接依赖 | BSD-2-Clause |
| tree-sitter | 代码块内部高亮，直接依赖 | MIT |
| Zed | SumTree、坐标系统思想（**仅思想，代码不可参考**） | GPL-3.0 |
| xi-editor | Rope Science、editor construction kit 思想 | Apache-2.0 |
| ProseMirror / Milkdown | 文档模型与插件化思想（不采用其 AST 往返模型） | MIT |
