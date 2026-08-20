# ADR 0039：Markdown block CST v1

## 状态

已接受（Phase 1）

## 背景

现有 `yu-markdown` 已有跨 Piece/rope chunk 的 lossless line scanner、持久化 `BlockSequence` 和
增量收敛，但只区分空行、段落、ATX heading 与 fenced code。下一步需要让 projection/layout 能
识别 blockquote 和 list item，同时不能把 Markdown 源码复制成第二份文本或破坏现有 `(range, kind)`
缓存契约。

## 决策

- `Block` 作为 root-level、source-backed 的 flat CST node；它只持有 `TextRange` 和 `BlockKind`，
  不持有 `String`、HTML 或可编辑富文本对象。
- 新增 `BlockQuote { depth }` 与 `ListItem { ordered, depth, marker, start }`。连续同深度 quote 行归入
  一个 block；list item 的 lazy/indented continuation 行归入当前 item，新的 marker 开始新的
  block；嵌套 marker 先以更大 depth 的独立 block 表示。
- line analysis 只识别 ASCII container marker 和有限 metadata；UTF-8 正文仍通过 `ChunkCursor`
  流式扫描，CRLF 和所有源码 byte range 原样保留。
- 增量 parser 继续使用现有 `BlockRecord` 的 range/kind/start-state/end-state/hash/byte
  equality 收敛条件。attached marker、fence 和 container boundary 任何变化都必须通过 full parse
  differential test 校验。
- 不创建通用 `yu-syntax` crate；等第二个真实语法消费者出现后再抽取 child arena/green-red tree。

## 结果

- `BlockProjection` 和后续 layout 可以区分 quote/list block，而不改变 canonical source 或 cache
  key。
- flat block CST 为 inline parser 预留稳定输入范围；嵌套 child identity、list tightness、task
  checkbox 和完整 CommonMark container stack 仍可在后续阶段增加。
- 现有 Piece Tree、Rope、Snapshot retention 和 incremental model test 继续复用同一 parser。

## 验证

- block range 重建必须等于原始 source；范围不能有 gap 或 overlap。
- `-attached`、`1.attached` 等 attached marker 必须保持 paragraph；合法 quote/list marker 必须
  产生对应 kind/depth。
- 随机 Unicode/Markdown edits 下 incremental document 必须与 full parse 完全相等。
