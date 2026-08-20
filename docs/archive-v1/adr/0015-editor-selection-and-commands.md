# ADR 0015：EditorDocument 统一拥有 selection 与基础 command

- 状态：Accepted
- 日期：2026-08-10

## 背景

`EditorDocument` 已经统一拥有 canonical source、Revision 和 composition overlay，但如果
selection/caret 仍由平台 view 私自保存，键盘编辑、IME commit 和外部 Transaction 会再次形成
多份状态。尤其在 Unicode 文本中，裸 `usize` 不能表达 revision、UTF-8 source offset 和
visual caret affinity 的边界。

## 决策

`yu-editor` 增加 `EditorSelection`：

```text
EditorSelection
├ revision
├ anchor: ByteOffset
├ focus: ByteOffset
└ CaretAffinity
```

anchor/focus 保留选区方向，`ordered_range()` 只在需要替换源码时生成有序范围。selection
只能从同一 `TextSnapshot` 构造或通过 `ChangeSet` 映射；旧 Revision 的 selection 不能设置到
当前文档。

`EditorDocument` 现在同时拥有：

```text
EditorDocument
├ TextBuffer
├ EditorSelection
└ Option<CompositionOverlay>
```

永久 Transaction 成功后先映射 selection；composition commit 再把 caret 放到提交文本之后。
`EditorCommand` 的第一批命令是插入、前后删除和左右移动。删除/移动按 Unicode extended
grapheme 边界执行，不把组合重音、ZWJ emoji 拆开。

macOS FFI 增加 selection 查询，返回 `(revision, UTF-16 start, UTF-16 end)`。Swift bridge 在
commit 后用该结果更新 AppKit selection，并验证它与 Rust canonical source 相同。

## 结果

- source、selection、composition 和 command 共享同一个 Revision 协议；
- 永久编辑不会留下平台私有的 source selection；
- Unicode 删除和移动有可重复的 Rust 行为测试；
- FFI 不暴露 Rust 内部结构，只返回 caller-owned 标量坐标；
- 后续 projection/layout 可以把 `EditorSelection` 映射到 visual caret，而不改变编辑核心。

## 限制

当前 macOS spike 仍保留 AppKit text storage 作为视图投影，正式编辑器不会把它作为
canonical source。grapheme command 已通过 chunk-aware `GraphemeCursor` 执行，不因一次
左右移动或删除物化整个 Piece Tree Snapshot。
