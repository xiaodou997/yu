# 核心不变量 v2

这些约束优先于具体数据结构和第三方库选择。任何实现、任何优化、任何平台适配
都不得违反本文。与本文冲突的代码是 bug，与本文冲突的 ADR 无效。

> 本文取代 v1 不变量。v1 版本保留在分支 `archive/v1-source-projection`
> 与 tag `v1-final`（commit `e8140be`）。v1 中约束 TextKit fallback、
> capability mask 与 count/fill FFI 的条目随该机制一同废止。
>
> 架构依据见 [`docs/architecture/overview-v2.md`](../architecture/overview-v2.md)。

---

## A. Source

**A1.** Markdown 源码是唯一持久化真源。不存在第二份文档数据。

**A2.** 任何视觉结构不得拥有源码副本，只能引用 source range，
或显式声明自己是 transient 替代物。

**A3.** 未被编辑的源码不得因解析、装饰、布局或保存而被重新序列化。
`__hello__` 保存后仍是 `__hello__`。

**A4.** `DocumentSession` 只接受有效 UTF-8。UTF-8 BOM 属于文件元数据，
加载/保存时保留，但不得进入 canonical source 坐标或 parser range。

**A5.** 保存使用同目录临时文件加原子 rename。外部文件指纹变化或目标消失时
必须拒绝覆盖；reload 只能在 clean session 执行。软链接保留用户可见路径，
但读写、指纹与原子替换都指向 canonical target。

---

## B. Editing

**B1.** 所有永久修改必须经过 Transaction。任何组件不得直接修改文本。

**B2.** 一个 Transaction 内的所有 range 属于同一个 base Revision。

**B3.** Transaction 原子提交。任一 edit 非法时文档保持完全不变。

**B4.** 成功提交产生严格递增的新 Revision 和可应用的 inverse Transaction。

**B5.** 跨编辑长期存在的位置使用 Anchor，不使用裸 `ByteOffset`。

**B6.** 语法相关的编辑行为（列表 Enter、空项 Backspace、缩进/反缩进、
task 状态切换、加粗、插入链接）本质都是对 source 的 replacement，
必须表达为普通 Transaction，不得建立富文本第二真源。

**B7.** History 只保存有界 inverse Transaction。Undo/Redo 回放不再写入 history；
新的永久 edit 清空 redo；selection 与 composition 边界断开当前 group。

**B8.** dirty 是当前 Revision 与 `saved_revision` 的比较。
Undo 回到相同字节也不能绕过显式保存边界。

**B9.** **选区是一组，不是一个。** `Selections` 恒满足：至少一条、全体同一
Revision、按起点升序、互不重叠（相邻两条 `prev.end() <= next.start()`，等号成立
时两条都必须非空）、`primary` 在界内。归一化（排序、合并、定位 primary）只有
`Selections` 一个实现，平台侧与命令层都不得自己先排一遍。

> 「互不重叠」比 `yu_text::validate_edits` 严一点，这是有意的。相邻的两段**非空**
> 选区必须留着——`aa` 在 `aaaa` 里的两处匹配就是 `0..2` 与 `2..4`，并掉等于把
> 「选中全部匹配」变成「全选」；而一个停在选区边界上的**空**光标要并进去，
> 否则打字会把同一个位置插两次。

**B10.** **一条命令一个 Transaction。** N 条选区产出 N 条 edit，一次提交、一份
inverse、一次 `history.record`。因此 undo 分组与光标有几根无关。

**B11.** **命令自己造的 edit，落点自己算。** 编辑后的选区位置由这条命令的累计
位移推出，不经 `map_anchor`——两条 edit 首尾相接时，前一条的终点会被
`Affinity::After` 推到后一条替换之后，两条选区因此重叠并被合并。`map_through`
只服务外来的 Transaction（undo/redo、缩进、任务勾选）。

---

## C. Parsing

**C1.** 解析结果的 source range 必须有序、有效，且能完整覆盖预期源码。

**C2.** **lossless 的定义**：解析树中任意两个相邻节点之间的 gap 必须可由
position 精确推导，且推导结果与原始字节完全一致。
不要求每个字节都拥有一个命名节点。
（此定义相对 v1 放宽，以匹配 lezer 风格的紧凑树；保真度要求不变。）

**C3.** `incremental_parse(edit(old))` 必须与 `full_parse(new)` 等价。
等价性由差分测试守护，不由人工推理保证。

**C4.** parser 不复制正文。节点通过 source range 引用 Snapshot。

**C5.** malformed 或未闭合的 Markdown 必须保留为可编辑源码，
不得导致内容丢失，不得凭空制造语义节点。

**C6.** **引用链接的成立与否不由 parser 决定。** parser 只产出候选引用；
是否解析为链接由同 Revision 的 reference table facet 在装饰阶段判定。
（此条修正了 lezer-markdown 已声明的偏差。）

**C7.** CommonMark 语义以官方 spec 用例为准，与 comrak 做差分测试。
任何有意偏差必须在本文 F 节逐条登记，未登记的偏差是 bug。

> **S7 第六刀之后 comrak 同时是产品实现**（`yu-export` 的 HTML 导出）与这里的
> 测试 oracle。**这没有让差分变成自证**：差分的「我方」是 `yu-syntax` 的树加
> `tests/support/html.rs`，那条路上一行 comrak 都没有。但两处的版本必须锁在
> 一起，否则「差分绿了」与「导出对了」不再是同一个 comrak 说的话——版本因此
> 走根 `Cargo.toml` 的 `[workspace.dependencies]`。

规范用例入库在 `third_party/commonmark/`（CC-BY-SA 4.0，与仓库其余部分的
许可不同，见该目录 README），由 `crates/yu-syntax/tests/commonmark_spec.rs`
逐条执行。该测试同时核对用例数与文件校验和——「差分测试通过了但其实用例
没跑到」是这条不变量最容易被架空的方式。

**`yu-syntax` 解析的是 CommonMark 的超集**：GFM 的任务项（`- [x] 交作业`）
无条件开着，理由见该 crate 的模块文档与架构总览的 S6 第九刀。它与本条不打架
的理由是**可以验的**，不是推断的：652 条用例里一条任务项都没有，由
`tasklist_syntax_is_absent_from_the_spec` 钉住。规范哪天加进一条，红的是那条
断言，而不是通过率的棘轮悄悄掉一格——后者在提交里只看得见一个被调小的数字。

超集部分自己带 oracle：`tests/differential.rs` 的对照方 comrak 开着同一个
扩展。**开对照方的扩展不是可选的**——不开的话，`- [x] a` 的差分比的是「开了
扩展的 Yu」与「没开扩展的 comrak」，永远不一致，于是只能把整类输入从语料里
排除，而那等于新语法一条 oracle 都没有。

---

## D. Decoration

**D1.** **视觉表现的唯一来源是 DecorationSet。** 任何「隐藏语法字符」
「替换为控件」「改变样式」的效果都必须表达为 Decoration，
不得通过 layout 或 scene 的特殊分支实现。

> **它的边界：D1 管的是文字自己。** 选区、光标、搜索命中的底色**不是**
> Decoration——它们是盖在文字上、由**非文档状态**（选区、查询）驱动的矩形，
> 不改任何字节的字型、字号或可见性，改的只是它下面那块颜色。装饰的三张表也
> 表达不了它们：`TextAttrs` 只有字型与字号倍率。它们住在场景层，由一段源码
> 区间加一份 `BlockLayout` 直接产出 `EditorDecorationPrimitive`。
>
> 判据是「文档的字节流变了它会不会变」：藏 `##` 会跟着源码变，所以是装饰；
> 一块选区底色只跟着选区变，所以不是。**哪一天搜索要改变文字本身**（隐藏不
> 匹配的行、折叠），那时它才落回 D1 里面。
>
> 选区是这条边界的第一个占位者，第三刀（搜索）是第二个——写下来是因为在
> 只有一个的时候，它看上去像一个没解释的例外。
>
> **第五刀（代码块高亮）是这条判据第一次被反方向用到。** 它给字形本身上色，
> 看着像「又一种颜色」，但按上面那句判据它跟着字节流变，所以**它在 D1 里面**，
> 走装饰而不是场景层。这条边界划的是「非文档状态驱动的矩形」，不是「改颜色」。

**D1 的一个落法：`TextAttrs` 只有一处重建，加属性必须在那里显式带过来。**
`yu-editor::blockinput::DecorationDraft::assemble` 给标题算字号倍率时用
`TextAttrs::new(style).with_size_scale(..)` **从头造**一份新的属性，装饰那边
新加的任何一样东西都会在这里被归零。第五刀的配色角色就是这么掉过一次的：
装饰产出是对的、布局是对的、场景是对的，**代码块里一个字都不着色，而且不报错**。
`yu-markdown` 那一侧的断言压不住它——判据必须落在**下游**（字形上的属性）。

**D2.** DecorationSet 不可变，与 Revision 绑定，可安全并发读取。

**D3.** `map(&ChangeSet)` 必须使 DecorationSet 随 Transaction 正确迁移，
边界 bias 显式声明。此性质由 property-based 测试守护。

**D4.** DecorationSet 必须支持 O(log n) 的 source offset ↔ visual offset
双向映射，且 round-trip 无损。这是投影映射链的唯一实现。

**「唯一实现」允许上面有一层薄的换算，但那层不得自己数隐藏了多少字节。**
S6 换消费者之后 `yu-editor::VisualText` 就是那一层，它只做三件 DecorationSet
按定义做不了的事：**换原点**（装饰集合的视觉偏移是整篇文档的，而
`BlockLayout` 排的是一个块）、**拿出文本**（装饰集合不持有源码）、
**叠 composition**（preedit 是往视觉文本里插入一段不在 source 里的文字，
D 节的四个变体都表达不了它，H1 也说它是 transient overlay）。

判据是：**任何一处「这段被藏起来了吗」的判断都必须来自 DecorationSet。**
自己再遍历一遍隐藏区间去切可见片段，就是第二个实现——哪怕结果一样，它会
在下一次改动时分叉，而分叉的表现是光标与画面差几个字节，不报错。

**D5.** `Replace` decoration 使对应 source 的 visual width 为零，
但 source 长度不变、内容不变、可被光标穿越与选中。

**D6.** 多个 extension 产出的 DecorationSet 合并时顺序确定
（按 `from, side, priority`），extension 之间不得相互感知。

**D7.** Widget 的 intrinsic size 未就绪时必须返回 placeholder size，
layout 正常完成，不得阻塞、不得整帧失败。资源就绪后发布 Revision-bound
通知触发受影响范围重新 layout；失败必须有有界退避重试并保留可编辑的源码回退。

---

## E. 分层与依赖

**E1.** **Markdown 只存在于 `yu-markdown` 一个 crate。**
`yu-state`、`yu-decoration`、`yu-layout`、`yu-scene`、`yu-render`、`yu-font`
及所有 platform crate 中不得出现任何 Markdown 语法概念。
验证方式：`grep -ri "markdown\|heading\|emphasis\|blockquote\|codefence" <这些 crate>` 零命中。

**那条 grep 是必要条件，不是充分条件。** `table` / `image` / `task` 都不在
关键词里，而 `yu-layout::TableLayoutSnapshot` 与 `yu-scene::TablePrimitive`
与 `HeadingPresentation` 是同一种泄漏：一种语法一个类型、一条全链路
（架构总览第 2.1 节）。判据按第 3 节的对照表走，不按 grep 走——否则 grep
会绿而泄漏还在。S5 结束时这三样分别成了：表格与图片的几何搬进 `yu-editor`
（那一层允许认识 Markdown），场景层的三套语法 primitive 合并成一个渲染中立
的 `OrnamentPrimitive`。

**`yu-editor` 不在禁止清单里。** 它是允许认识 Markdown 的那一层，
`tools/check-deps.py` 登记了 `yu-editor → yu-markdown`。不透明 id
（`StyleId` / `LineStyleId` / `WidgetId`）的解释权就归它——布局层查表拿到
「斜体、1.6 倍字号」，拿不到「这是二级标题」。

**E2.** crate 依赖图必须是严格 DAG，方向为：

```text
yu-core ─┬─ yu-text ── yu-syntax ─┐
         ├─ yu-decoration ────────┴─ yu-markdown ── yu-state ─┐
         └─ yu-font                                           │
                    yu-layout ── yu-scene ── yu-render ── platform
```

反向依赖是 CI 失败，不是待办事项。`yu-font` 只依赖 `yu-core`。

**`yu-decoration` 在 `yu-markdown` 下方，不在上方。** 本条初版写的是
`yu-markdown → yu-state → yu-decoration`，与第 4.3 节自相矛盾：那里给
`yu-markdown` 的职责是「Markdown 语法定义与 **decoration 产出**」，而产出
decoration 就必须认识 `Decoration` 这个类型。

正确的方向由两条推出来：`yu-decoration` 的禁止项是「知道 Markdown」，一个
不知道上层任何事情的数据结构属于下层；而 `yu-state` 要聚合各 extension 的
产出，它在两者之上。于是 `yu-decoration` 是一个**原语**——像 `yu-text` 那样
——而不是一个中间层。

**E3.** `yu-render::RenderCommand` 的变体集合冻结为
`Glyph` / `FillRect` / `Texture` / `Quad`。
新增语法**不得**新增 RenderCommand 变体。

**E4.** `yu-text` 不得让 ropey 的类型或索引逃逸出 crate 边界；
对外只暴露 `ByteOffset`。

由 `tools/check-rope-leak.py` 强制，四条机械规则：只有 `yu-text` 可以依赖
ropey；只有 `crates/yu-text/src/storage/ropey_backend.rs` 可以引用 ropey 的
路径；该文件不得有无限定的 `pub`；不得开启 ropey 的 `metric_chars` feature。
后两条合起来使 ropey 的类型不可能进入 `yu-text` 的公开签名——要写进签名就得
先写出路径，而能写路径的文件什么都不导出。

本条初版写的是「char index 或 ropey 类型」。选定的 ropey 2.x 是全字节索引
的，char 相关 API 只在 `metric_chars` 下存在而 Yu 没有开，因此 char index
不是需要防的东西，而是不存在的东西——第四条规则守的就是它继续不存在。真正
要防的是类型与依赖的扩散，条文据此改写。

**E5.** 断行、bidi 与分词属于共享 Rust（UAX #14 / #9 / #29）；
shaping 与栅格化属于平台。平台后端不得决定断行位置。

**E6.** **视觉坐标只有一套实现，坐标空间在类型里。**
`Point` / `Size` / `Rect` 只定义在 `yu-core::geometry`，空间
（`Block` / `Document` / `Device`）作为类型参数存在。跨空间只能走
`translate_into`（平移原点）与 `scale` / `unscale`（换单位），
不得以裸 `f32` 直接搬运。任何组件不得自行摊开 `x/y/width/height: f32`。

由 `tools/check-geometry.py` 强制。例外只有跨 C ABI 的平铺结构体与非 f32 的
整数量（atlas 纹理坐标、图片自身像素尺寸），逐个登记并写明单位。

这一条守的是两次真实事故：绝对坐标被当成相对坐标，逻辑坐标上又乘了一次
backing scale。两次都不 panic、不报错，只是画错，都要靠真实窗口才发现——
和字节/字符索引混用属于同一类失败模式。

**E7.** **shaping 的产出契约写在 `yu_core::ShapingProvider` 上，由
`yu_core::shaping_conformance` 强制，每一个实现都要跑。**

十条条文见那个 trait 的文档。核心是两条合起来的那一条：**一簇一形**——一个 run
的全部 `Glyph::source` 必须首尾相接、不重叠、**非空**地铺满该 run。

- 「铺满」少了「非空」不成立：`from != cursor` 对空区间恒不成立，于是空区间
  过得了那道门，然后在 run 末尾让布局层**越界 panic**，落在中间则凭空多算
  一段 advance。这一条是 S7 第七刀 spike 抓出来的，此前整仓都活着。
- **后端做不到就返回 `Err`，不许伪造区间。** 一簇多形有三种凑法（重复起点 /
  空区间 / 并成一形），分别是重画、panic、丢字形，没有一种是对的。

**这一条为什么必须是可执行的**：`ShapingProvider` 今天有六个实现，五个是
mock，全都一 grapheme 一 glyph——**只有一类实现的接口等于还没被证明**。而契约
以前只存在于调用方（`yu-layout/src/block.rs` 的 tiling 门）里，类型上一个字
都看不出来；第二端照着类型写就会撞上。

**语料压不住的那半要靠故意违约的 mock。** 实测（S7 第七刀 spike，35 个语料）：
真实的 CoreText 从来没产出过两个字形同一个起点——会出现它的脚本全部先被
`CTRunStatus` 拒了。所以「一簇多形」这一条**没有任何真实输入能触发**，
反向验证只能靠合成输入（`yu-font-macos` 的 `cluster_spans` 用例、
`yu-layout` 的 `Ranges` mock、`yu-core` 自己那几条）。

**配套的一条：`FontFaceId` 由 shaper 铸、由 rasterizer 消费，两者必须共用
同一张 `yu_font::SharedFaceTable`。** 各铸各的不 panic、不报错，表现是**屏幕
上画出来的字全是别的字**。这条以前只存在于 macOS 的一个方法上
（`CoreTextShaper::rasterizer()`），现在住在类型里。

两步走到位的：刀 b 把「共用」做成默认路径（`SharedFaceTable` 是拿到一张表的
唯一方式，后端的 rasterizer 只能由它构造）；刀 c 补上「从哪儿要栅格化器」
——`yu_font::RasterizingShaper` 要求 shaper 自己交出与它配对的那一个，于是
**拿到 shaper 的人不需要、也没有第二条路去要**。它同时是把视口帧准备
（`yu_workspace::ViewportFrameBuilder`）泛型化的条件：那 462 行里唯一跟平台
有关的就是这一步。

---

## F. 已登记的规范偏差

每条偏差必须包含：偏差内容、原因、影响范围、是否计划修复，以及它对应的
CommonMark 规范用例号。

**登记表必须是紧的。** `crates/yu-syntax/tests/commonmark_spec.rs` 双向校验：
未登记的失败用例让测试红，**已登记却通过了的用例同样让测试红**。后一条守的
是「偏差修好了但登记还留着」——那会让下一个人分不清哪些失败是有意的。

当前口径：CommonMark 0.31.2 的 652 条用例中 **644 条逐字节通过（98.77%）**，
其余 8 条分属下面两条偏差。**F3 已经关掉**（S7 第六刀），编号退休不再复用，
关掉它花了什么写在下面。

| 编号 | 偏差 | 原因 | 规范用例 | 计划 |
| --- | --- | --- | --- | --- |
| F1 | 引用式链接的**括号配对**不查 reference table。`[a [b]][ref]` 的分组与 CommonMark 不同 | 不变量 C6 的直接后果，见下 | 512, 523, 528, 569, 571 | 不修 |
| F2 | 制表符不展开。跨越「标记/内容」边界的制表符整个归标记 | 不变量 A1/A3 的直接后果，见下 | 5, 6, 7 | 不修 |

> **F1 与 F2 是解析的偏差，HTML 导出不随它们偏。** S7 第六刀把导出换成 comrak
> 之后，剪贴板里的 HTML 走的是 CommonMark 的语义，而编辑器画的是 Yu 自己的
> 解析。这几处「所见 ≠ 所拷」是**有意**的：剪贴板是给别的 app 的，别的 app 认
> CommonMark。同类还有 `***`、缩进代码块、HTML 块——`BlockKind` 没有这三种
> 变体，编辑器按 I5 画成普通段落源码，导出按 CommonMark 渲染。理由与代价写在
> `yu-export` 的模块文档与 overview 第 8 节 S7 第六刀。

> **剪贴板的 HTML 是一份写给陌生人的独立文档，不是一段片段**（S7 第七刀 c 的
> G 节验收补）。它必须自己声明编码：`public.html` / `text/html` 这个类型只承载
> 字节，收件方拿不到声明就只能猜，而系统的传统编码在中文环境下是 GBK——实测
> 不带声明时中文粘进 TextEdit 全是乱码，而 ASCII 完好。声明加在剪贴板那一层
> （`ClipboardPayload::html`），`export_html_fragment` 仍然是一段不带编码的片段。
>
> **反方向（导入）在同一次验收里翻了一次案，记在这里。** 原来的登记是「导出
> 照原样发原始 HTML，导入按白名单拒绝原始 HTML（那是别人的 HTML）」，由
> `raw_html_deliberately_does_not_round_trip` 钉住并写着「别当成缺口顺手补上」。
> G 节实测下来，这条理由在**信封**上不成立，只在**语义**上成立：
>
> - 每一个真实浏览器发的剪贴板 HTML 都带 `<div>` / `<span>` / `style` / 注释
>   （Chrome 对一个连一个 `div` 都没有的页面，也会把词间空白包成
>   `<span> </span>`、给每个块挂二十来条声明的 `style`）；
> - 于是「拒绝信封」在实践上等于**拒绝每一次浏览器粘贴**。
>
> 现在的规矩分三档：**语义标签**照常翻译；**信封与纯呈现**
> （`html`/`head`/`body`/`div`/`span`/注释/`<!doctype>`）穿透，`head` 连内容
> 一起丢；**其余标签继续拒**（`<b>`、`<article>`、`<script>`——「那是别人的
> HTML」这一半没有变）。属性同理**默认忽略**（输出是 Markdown，被忽略的属性
> 没有地方可去），只拒一小份「忽略了会让输出**静默出错**」的名单：
> `colspan`/`rowspan`（表格少画几列）、`reversed`/`type`（有序列表编号反了）、
> `hidden`（看不见的文字变成正文）。
>
> **代价是登记在案的**：用户自己写在 Markdown 里的 `<div>raw</div>` 经 HTML
> 这条路导入时被拍平成 `raw`。Yu 分不出「用户写的 div」与「浏览器的 div」，
> 而后者是唯一真实的输入来源。`presentational_containers_are_flattened_not_rejected`
> 钉住这个代价，`semantic_raw_html_deliberately_does_not_round_trip` 钉住没有
> 变的那一半。Yu → Yu 不走这条路（剪贴板上有 canonical 的 Markdown flavor）。
>
> **同一次验收还查出一件更根本的事**：平台侧取剪贴板的顺序原来是
> 「Markdown > 纯文本 > HTML」，而**任何真实剪贴板都带纯文本**——于是整条 HTML
> 导入路径在生产里**一次都没有被走到过**，白名单、用例、fixture 全都在为一条
> 不可达的分支服务。顺序改成 **Markdown > HTML > 纯文本**：纯文本按定义是同一
> 份内容丢掉结构之后的样子，两者都在时取纯文本等于每次都主动选那份更少的。
> 回退没有变（策略拒绝时返回 `nil` 落回纯文本），只是现在真的会走到。
>
> **一处已知的粗糙，不修**：Chrome 拷贝时会把词间的普通空格换成不换行空格
> （U+00A0），粘进来的段落因此夹着看不见的 NBSP，搜索匹配不上。不改写它——
> 导入器的职责是翻译结构不是重写文本，而悄悄换回空格会连着毁掉网页作者故意
> 写的那些。`a_real_browser_clipboard_payload_imports_to_markdown` 把它断言
> 出来，让它看得见。

### F1 为什么不修

CommonMark 的行内解析发生在**全部块解析结束之后**，因此它做 `]` 配对时已经
拥有完整的 reference table：`[link [foo [bar]]](/uri)` 里 `[bar]` 不成立，
所以外层括号才能配上。

不变量 C6 选了另一条路：parser 只产出候选引用，成立与否由装饰阶段判定。
这是为了增量——一旦行内解析依赖全文档的 reference table，在文档任何位置增删
一条 `[x]: /y` 就会让**所有**行内解析结果失效，而这正是编辑器里最常见的编辑。
`@lezer/markdown` 出于同样的理由做了同样的取舍。

代价就是这 5 条：括号分组在少数嵌套情形下与规范不同。取舍是清楚的——
增量性是产品性质，这几种嵌套是边角写法。

### F2 为什么不修

CommonMark 在块解析时把制表符展开成空格再计算缩进，于是一个制表符可以「一半
属于列表标记、一半属于内容」。Yu 不能这么做：不变量 A1 说 Markdown 源码是唯一
持久化真源，A3 说未被编辑的源码不得被重新序列化。一个制表符是一个字节，
节点的 range 切不到字节中间。

因此跨边界的制表符整个归标记，内容少掉那几列。受影响的只有「制表符恰好跨越
内容起始列」这一种写法。

> 展开制表符属于**呈现**，不属于解析。真要让这几列显示出来，是 S4 的装饰层
> 给制表符一个宽度，而不是让 parser 改写源码。

### F3 已经关掉（S7 第六刀），留下的是这条路怎么走完的

这一条活了三个阶段，每一阶段都缩小一点，最后在 S7 第六刀关掉。留着这段是
因为**它每一次没关掉的理由都是同一条**，而那条理由最后是被另一件事付掉的。

1. **S4 查清了让 540 失败的不是 parser，是对照用的参考渲染。**
   `crates/yu-syntax/tests/support/html.rs` 的 `normalize_label` 用
   `char::to_lowercase`（Unicode simple lowercase），而 CommonMark 要求
   full case folding：`ẞ` 的 simple lowercase 是 `ß`，full fold 是 `ss`，
   所以匹配不上 `[SS]:`。而 `yu-syntax` 的产品链路里根本没有引用标签匹配
   （不变量 C6），所以这个问题在那个 crate 里没有答案，也不该有。

2. **S6 第十三刀把 reference table 建进装饰阶段，选了 `str::to_lowercase`。**
   从「只认 ASCII」走到「认 Unicode 的绝大多数」——`[Ä]` 与 `[ä]` 此前折不到
   一起，那是一条写对了的引用被画成普通文字。剩下 `ẞ`/`ﬁ` 那几个字符。
   不做 full fold 的理由一直是同一条：标准库不提供，要给 `yu-markdown`
   引入它的**第一个**外部依赖。

3. **S7 第六刀付了那笔钱，但不是为 F3 付的。** 同一刀把 HTML 导出换成
   comrak，而 **comrak 自己用的就是 `caseless`**
   （`comrak::strings::normalize_label`），它已经在 `Cargo.lock` 里。于是
   `yu-markdown` 直接依赖 `caseless` 这件事，代价从「为几个字符扩大整个
   依赖面」变成「用一个已经进来的 crate」。

关掉它改了两处，**两处必须是同一个答案**，所以版本走 `[workspace.dependencies]`：

- `crates/yu-markdown/src/reference.rs::normalized_label`——产品链路。
- `crates/yu-syntax/tests/support/html.rs::normalize_label`——参考渲染，棘轮走它。

**产品链路那一侧靠棘轮抓不住。** 把 `reference.rs` 换回 `to_lowercase`，
`yu-markdown` 的 116 条用例一条都不红（实测），红的只有 `yu-syntax` 的棘轮
——那是另一份实现。判据不能靠另一条路代劳，所以
`extension_decorations.rs::reference_labels_use_full_case_folding_not_simple_lowercase`
在产品链路自己这一侧断，并反向验证过。

**没有一并解决的是搜索的「不区分大小写」**，尽管它一直挂在同一个闸门上。
两者只是碰巧都需要一份 case folding：F3 折出来的是一个查表键，从不映射回
源码偏移；搜索要回报 `TextRange`，折叠必须给得出对齐信息，而 `caseless`
给不出。那条登记的触发条件因此改了，见 `yu-editor::search` 的模块文档。

---

## G. Revision 与异步

**G1.** 后台任务只读取不可变 Snapshot。

**G2.** 每个异步结果携带其输入 Revision。

**G3.** 过期结果不得发布到当前编辑状态，必须整体拒绝而非部分采用。

**G4.** 取消只是优化，Revision 检查才是正确性边界。

**G5.** 缓存只能提高性能，不能改变文档语义。
任何缓存命中与未命中路径必须产生相同结果。

**G6.** 资源（图片、Math、字形）的 publication 必须验证尺寸与数据长度；
旧 Revision 的 publication 在进入 GPU 边界前拒绝。

---

## H. Input 与 IME

**H1.** IME marked / preedit 文本是 transient overlay。
不写入 canonical source，不推进 Revision，不进入 Undo，不污染 parser 缓存。

**H2.** 只有 IME commit 才生成永久 Transaction。

**H3.** OS 查询的 selection、surrounding text 与 caret rect
必须来自同一个一致的编辑状态。

**H4.** source anchor affinity 与 visual caret affinity 是两个独立语义，
不得互相替代。

**H5.** 软换行处的 hit test 必须保留 upstream / downstream，
不得只返回裸 offset。

**H6.** composition 的每次 update / commit / cancel 必须同时携带
expected Revision 与 composition generation；失配返回 stale，
不得触碰 canonical source。

**H7.** 光标按 grapheme cluster 移动。跨 chunk 边界的 grapheme 结果
必须与连续 UTF-8 文本一致，且不得为单次移动物化完整 Snapshot。

---

## I. Platform

**I1.** 平台层不解析 Markdown，不根据 delimiter 自行推导 source range。

**I2.** 渲染循环由 Rust 拥有。RenderPlan 不跨 C ABI。
平台提供的是 GPU surface 与窗口，不是绘制逻辑。

**I3.** 平台与 Rust 的 FFI 边界只承载：文件操作、输入事件、
selection 查询、Accessibility 查询、surface 生命周期。
新增视觉能力**不得**新增 FFI 函数。

**I4.** FFI 必须返回明确 status code。panic、非法 UTF-8、
surrogate 中间位置不得穿过 ABI。

**I5.** **不存在第二条渲染路径。** Rust 渲染器是唯一渲染器。
尚未支持的语法按普通段落源码文本绘制——永不白屏、永远可编辑。
不得引入 TextKit 或任何平台文本系统作为整页/局部回退绘制。

> **「尚未支持的语法」这句话不够**：S7 第七刀查出来的那个洞不在语法上，在
> **shaping** 上。CoreText 拒掉希伯来、阿拉伯、天城文（`CTRunStatus` 那一步，
> 实测连单独排一个字符也拒），而 `ShapingProvider` 的 `Err` 一路传成 `?`，
> 于是那一整屏发不出来——连源码文本都画不出来。
>
> 条文补一句：**后端排不出来的簇画替代字形（U+FFFD），不让整个块失败。**
> 降级是两级的：整段失败先逐簇重试（同一个 run 里排得出来的簇要保住自己的
> 字形），仍然失败的才替换。**降级必须看得见**——`BlockLayout::substituted_clusters`
> 报出替换了几个簇，正常语料必须是 0。
>
> **契约违约不走这条路。** 后端说「排好了」却给出不铺满或有空隙的字形是 bug
> 不是「排不了」，那条继续返回 `Err`（E7）。两者混在一起会让一个坏后端悄悄
> 退化成一屏替代字形。
>
> **这不等于支持了 RTL 与印度系文字。** 画出来的是一串替代字形，读不了，
> 但文档打得开、滚得动、选得中、复制得出原文。真正支持要让契约允许「run 内
> 字形按视觉序、source 逆序」，那会牵动 tiling 门、caret 映射与命中测试，
> 是另一件事，**登记在案**。

> **系统外观是平台知道而 Rust 不知道的一件事，与几何同类。** 跨 ABI 进来的是
> **一个事实**（现在是深还是浅），出去的是**一整套颜色**——「产品选色住在
> `yu-workspace`」这条不因为多了一种外观而松动。让平台送颜色等于把主题选择挪
> 进壳里，于是第二端要把同一套配色再挑一遍，而两端挑出来的**一定会漂开**。
>
> **它必须同时进 `FrameKey`。** 换外观既不推进 Revision 也不改变几何，漏掉的
> 表现是「切到深色，侧栏面板变深了而文档区一动不动」——面板走 AppKit 的语义色
> 自动跟，文档区是 Rust 画的一帧，而那一帧被判成了与屏幕上那一帧等价。
>
> **认不得的外观字节按浅色画，不拒整帧。** 为一个不认识的枚举值拒绝提交，
> 表现是一片空白——那正是 I5「永不白屏」要防的。这一条是反向验证时活下来的
> 一个变异逼出来的：把兜底改成深色之后全套用例照样绿。

**I6.** 平台的文本 mirror（如 `NSTextInputClient` 所需）不是第二真源，
不拥有 history、dirty 或 selection 的最终权威，可随时丢弃重建。

> 这条在多光标上有一个具体后果：**`NSTextView` 对不连续选区没有「主」的概念**，
> `selectedRange()` 转手一次之后 primary 一律退回第 0 条。由 Yu 发起的多光标
> 必须把 `primary` 直接送给 Rust，不能让 AppKit 转手再认回来。

**I7.** **AX 的单数与复数是两个属性，不是一个的降级。**
`AXSelectedTextRange` 给 primary，`AXSelectedTextRanges` 给全部。后者不得从前者
推出来——屏幕上有五根光标而读屏只知道一根，不报错。

**I8.** **C 头文件是无条件的，所以每个 `pub extern "C" fn` 也必须是无条件的。**
平台差异写在**函数体里**（`#[cfg(not(target_os = ...))]` 早退一个状态码），
不写在函数上。一个挂着 cfg 的 extern 函数在别的平台上根本没有符号，而头文件
仍然声明它——链接时 unresolved symbol。

由两条机制不同的检查强制，两条都要：

1. `tools/check-ffi-header.py` 第 4 条读**源码的属性**：便携，不用编译，在开发
   机上立刻红。它兜不住 `cfg_attr`、宏生成的 extern、被 cfg 掉的外层 mod。
2. `tools/check-ffi-symbols.py` 读**产物的符号表**：判断由 rustc 与归档器做出。
   它只覆盖**当前这个平台**（本仓交叉编译不了：tree-sitter 的 grammar 是 C，
   交叉要目标平台的 C 编译器），三个平台的覆盖来自 CI 的 rust 矩阵。

这一条是 S7 第七刀补的，起因是一次**全套门禁绿着的谎话**：两个 `macos_*` 函数
整个挂在 `#[cfg(target_os = "macos")]` 下，而 `cargo test --workspace` 在三个
平台都绿——因为从来没有人在非 macOS 上*链接*过那个 staticlib，只*编译*过。

> **反方向同样是撒谎，刀 c 用到了这一条：** 一个函数**无条件存在**，但它驱动的
> 状态整个是平台的（`macos_render_host_frame` 背后是 `MetalSurface` +
> `MetalViewportHostSession`），那么给它一个平台中立的名字就等于让 ABI 说了
> 一句它兑现不了的话——第二端调不了，而名字说它能。**名字要跟着「谁答得上来」
> 走，不跟着「参数里有没有原生指针」走。** 前缀该留的留着，Windows 另开一份。

---

## J. 性能

**J1.** 编辑只重解析受影响范围，只重建变化的 decoration，
只 layout 变化的 block，只提交 damage region。

「只重解析受影响范围」的**可断言量是重扫的字节数**，不是耗时：同样的输入
永远给同样的答案，退化时一定是真的退化。两层各有门禁——`yu_syntax::Parse`
的 `reparsed_bytes`（`crates/yu-syntax/tests/incremental.rs`），与
`DecorationCacheStats::reparsed_bytes`（`crates/yu-editor/src/decorations.rs`
与 `document.rs`，压的是产品链路上那条接线）。

**J2.** 不 layout 整份文档。按 block height index 以 O(log n)
定位可见范围，只处理 viewport + overscan。

**J3.** 高成本资源（图片、Math）只对当前 viewport 调度，
具备 LRU、内存预算、异步加载、失败退避与离屏淘汰。

**J4.** 上述每一条都必须有 bench 守护。没有 bench 的性能不变量等于没有。
