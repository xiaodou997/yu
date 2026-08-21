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

---

## D. Decoration

**D1.** **视觉表现的唯一来源是 DecorationSet。** 任何「隐藏语法字符」
「替换为控件」「改变样式」的效果都必须表达为 Decoration，
不得通过 layout 或 scene 的特殊分支实现。

**D2.** DecorationSet 不可变，与 Revision 绑定，可安全并发读取。

**D3.** `map(&ChangeSet)` 必须使 DecorationSet 随 Transaction 正确迁移，
边界 bias 显式声明。此性质由 property-based 测试守护。

**D4.** DecorationSet 必须支持 O(log n) 的 source offset ↔ visual offset
双向映射，且 round-trip 无损。这是投影映射链的唯一实现。

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

**E2.** crate 依赖图必须是严格 DAG，方向为：

```text
yu-core → yu-text → yu-syntax → yu-markdown → yu-state → yu-decoration
        → yu-layout → yu-scene → yu-render → yu-font → platform
```

反向依赖是 CI 失败，不是待办事项。`yu-font` 只依赖 `yu-core`。

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

---

## F. 已登记的规范偏差

本节为空表示当前实现与 CommonMark 无已知有意偏差。
每条偏差必须包含：偏差内容、原因、影响范围、是否计划修复。

| 编号 | 偏差 | 原因 | 计划 |
| --- | --- | --- | --- |
| — | — | — | — |

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

---

## J. 性能

**J1.** 编辑只重解析受影响范围，只重建变化的 decoration，
只 layout 变化的 block，只提交 damage region。

**J2.** 不 layout 整份文档。按 block height index 以 O(log n)
定位可见范围，只处理 viewport + overscan。

**J3.** 高成本资源（图片、Math）只对当前 viewport 调度，
具备 LRU、内存预算、异步加载、失败退避与离屏淘汰。

**J4.** 上述每一条都必须有 bench 守护。没有 bench 的性能不变量等于没有。
