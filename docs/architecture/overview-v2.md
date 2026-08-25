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
`yu-state` 收敛编辑状态。**进行中。**

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

已完成的两块：

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

**还没做的：** `EditorState`（见上，S5）、`Facet`（见上，S6）。

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
| ropey | Rope 实现，直接依赖（`2.0.0-beta.1`，全字节索引） | MIT OR Apache-2.0 |
| comrak | CommonMark 正确性 oracle 与 HTML 导出，直接依赖 | BSD-2-Clause |
| tree-sitter | 代码块内部高亮，直接依赖 | MIT |
| Zed | SumTree、坐标系统思想（**仅思想，代码不可参考**） | GPL-3.0 |
| xi-editor | Rope Science、editor construction kit 思想 | Apache-2.0 |
| ProseMirror / Milkdown | 文档模型与插件化思想（不采用其 AST 往返模型） | MIT |
