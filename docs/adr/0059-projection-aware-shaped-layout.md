# ADR 0059：Projection-aware Shaped Layout Diagnostic

## 状态

已接受（Phase 1 诊断）

## 背景

ADR 0058 只比较 canonical plain source 的 shaped line range。Yu 的产品模型不会把
Markdown 源码直接当作视觉文本：`**strong**` 的 delimiter、link destination 等
parser-owned syntax 必须保留在 source，却不能占用 visual width。若 macOS 只把原始
Markdown 交给 TextKit，比较结果会验证错误的对象；若 Swift 自己复制 Markdown parser，
又会产生第二套语义实现。

## 决策

- `yu-editor-ffi` 增加 `YuCoreTextProjectedLine`，同时返回 source UTF-16 range、visual
  UTF-16 range 和 shaped width。
- `yu_macos_core_text_projected_layout` 使用 shared `Projection::inline`、真实 CoreText
  system UI shaper 和 `LayoutSnapshot`，通过一次 count/fill ABI 返回 line records 以及
  parser 生成的 projected UTF-8 文本。FFI 不返回 `Projection`、`TextSnapshot` 或 CoreText
  句柄，也不修改任何 `EditorDocument` session。
- macOS spike 用 FFI 返回的 projected UTF-8 文本创建临时 TextKit storage，再把 Rust
  visual UTF-16 ranges 与 TextKit line fragments 逐条比较。Swift 不实现 Markdown 语法、
  delimiter 配对或 hidden-range 规则。
- Rust 额外保留的 zero-width trailing caret line 不计入 TextKit source-consuming line
  数量，但必须保持有序、source/visual range 合法且 width 为零。至少一条 source range
  必须比 visual range 更长，以证明 hidden syntax 被排除。
- 本阶段使用宽容器和显式换行样本，隔离 projection/source mapping；当前 shared layout
  的 grapheme/advance wrapping 与 TextKit 的自然语言 word-break 策略不作等价性承诺，
  后续单独定义 line-break policy comparison。

## 结果

- native 诊断现在验证的是 Yu 实际要绘制的 projected text，而不是错误的 Markdown 原文。
- source line `0..58` 可以对应 visual line `0..31`，hidden delimiter/link tail 不会污染
  visual UTF-16 坐标，同时 source range 仍可用于编辑、selection 和 undo。
- projected text 是 Rust parser 的唯一输出，避免 Swift mirror 发展成第二个 Markdown
  解析器；canonical source、Revision、selection 和 history 都保持不变。
- 该 probe 仍不覆盖完整 CommonMark、reference definition、fenced code、复杂 fallback、
  BiDi 或最终 viewport virtualization；这些需要各自的 projection/layout 契约和测试。
