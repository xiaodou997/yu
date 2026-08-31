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

---

## F. 已登记的规范偏差

每条偏差必须包含：偏差内容、原因、影响范围、是否计划修复，以及它对应的
CommonMark 规范用例号。

**登记表必须是紧的。** `crates/yu-syntax/tests/commonmark_spec.rs` 双向校验：
未登记的失败用例让测试红，**已登记却通过了的用例同样让测试红**。后一条守的
是「偏差修好了但登记还留着」——那会让下一个人分不清哪些失败是有意的。

当前口径：CommonMark 0.31.2 的 652 条用例中 **643 条逐字节通过（98.62%）**，
其余 9 条分属下面三条偏差。

| 编号 | 偏差 | 原因 | 规范用例 | 计划 |
| --- | --- | --- | --- | --- |
| F1 | 引用式链接的**括号配对**不查 reference table。`[a [b]][ref]` 的分组与 CommonMark 不同 | 不变量 C6 的直接后果，见下 | 512, 523, 528, 569, 571 | 不修 |
| F2 | 制表符不展开。跨越「标记/内容」边界的制表符整个归标记 | 不变量 A1/A3 的直接后果，见下 | 5, 6, 7 | 不修 |
| F3 | 引用标签只做 simple lowercase，未做 Unicode full case folding。`[ẞ]` 匹配不上 `[SS]:` | 标准库不提供 full folding，为几个字符引入 `yu-markdown` 的第一个外部依赖不划算，见下 | 540 | 接受那个依赖时 |

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

### F3 为什么只修了一半

这一条初版写的是「S4 落地时决定」，S4 查下来发现那个说法把三件事混在了
一起。理清之后是这样：

1. **让 540 失败的不是 parser，是对照用的参考渲染。**
   `crates/yu-syntax/tests/support/html.rs` 的 `normalize_label` 用
   `char::to_lowercase`（Unicode simple lowercase），而 CommonMark 要求
   full case folding：`ẞ` 的 simple lowercase 是 `ß`，full fold 是 `ss`，
   所以匹配不上 `[SS]:`。
2. **`yu-syntax` 的产品链路里根本没有引用标签匹配。** 不变量 C6 规定 parser
   只产出候选引用，成立与否由装饰阶段判定。所以「引用标签怎么归一化」这个
   问题在 `yu-syntax` 里没有答案，也不该有。
3. **`yu-markdown/src/reference.rs` 里那个 `to_ascii_lowercase` 是 v1 扫描器
   自己的 reference table，与 540 无关。** 它随 v1 一起被 S6 取代。

于是能做的只有一件事：把参考渲染改成 full case folding。Rust 标准库没有
full case folding，要为一条规范用例给测试支撑代码引入一个依赖——**决定是
不引入**，第 6 节的依赖取舍在这里同样适用，为了让一条用例变绿而扩大依赖面
不划算。

真正要决定的事被推到 S6：v2 的 reference table 建在装饰阶段，那时才需要选
一种归一化。

**S6 第十三刀选了：`str::to_lowercase`（Unicode simple lowercase），不是 full
case folding。** 理由是同一条——标准库不提供 full folding，要为几个字符
（`ẞ`→`ss`、`ﬁ`→`fi`）给 `yu-markdown` 引入它的第一个外部依赖，不划算。

选完之后**这一条缩小了，没有关掉**：

- 已经解决的：`to_ascii_lowercase` 折不到一起的那一大片（`[Ä]` 与 `[ä]`）现在
  折得到了。引用表也真的接进了装饰阶段——候选引用查不到定义就不是链接、不是
  图片（C6 落地）。
- 还没解决的：full folding 的那几个字符。规范用例 540 仍然红，棘轮仍然是
  643。要动它，得先接受那个依赖，届时参考渲染
  （`crates/yu-syntax/tests/support/html.rs` 的 `normalize_label`）要一起改。

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

**I6.** 平台的文本 mirror（如 `NSTextInputClient` 所需）不是第二真源，
不拥有 history、dirty 或 selection 的最终权威，可随时丢弃重建。

> 这条在多光标上有一个具体后果：**`NSTextView` 对不连续选区没有「主」的概念**，
> `selectedRange()` 转手一次之后 primary 一律退回第 0 条。由 Yu 发起的多光标
> 必须把 `primary` 直接送给 Rust，不能让 AppKit 转手再认回来。

**I7.** **AX 的单数与复数是两个属性，不是一个的降级。**
`AXSelectedTextRange` 给 primary，`AXSelectedTextRanges` 给全部。后者不得从前者
推出来——屏幕上有五根光标而读屏只知道一根，不报错。

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
