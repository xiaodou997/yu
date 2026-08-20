# ADR 0060：Revision-bound Projection Caret Query

## 状态

已接受（Phase 1 诊断）

## 背景

Projection-aware shaped layout 已经证明 Rust 可以返回 source line 与 visual line 的双坐标，
但原生输入层还需要一个更小的查询：给定当前 canonical Markdown source 的 UTF-16 caret，
平台必须知道 hidden syntax 之后的 visual UTF-16 位置，并且在 delimiter 边界保留
upstream/downstream 语义。若 Swift 自己遍历 projected text 或保存 delimiter 规则，就会产生
第二套映射实现；若查询不绑定 Revision，旧的 native caret 可能被应用到新 source。

## 决策

- `yu-editor-ffi` 提供 revision-bound 的
  `yu_composition_session_projection_caret`，输入为 `expected_revision`、source UTF-16
  boundary 和 `YU_CARET_AFFINITY_UPSTREAM/DOWNSTREAM`。
- ABI 只返回 owned scalar：

  ```c
  typedef struct YuProjectionCaret {
      uint64_t revision;
      uint64_t source_utf16;
      uint64_t visual_utf16;
      uint64_t round_trip_source_utf16;
      uint8_t affinity;
  } YuProjectionCaret;
  ```

- Rust 先校验 Revision、UTF-16 scalar boundary 和完整 source range，再消费
  `EditorDocument` 当前 revision 的 parser-owned projection。Upstream 映射到
  `ProjectionBias::Before`，downstream 映射到 `ProjectionBias::After`。
- `round_trip_source_utf16` 是同一 visual boundary 按相同 bias 反向映射的结果。它让平台
  可以验证 hidden delimiter 边界，而不需要拥有 Projection 或 Markdown parser。
- 查询只读，不推进 Revision，不修改 source、selection、composition 或 history。所有错误都
  通过既有 status code 返回；stale Revision 和 surrogate 中间位置必须在输出发布前拒绝。
- macOS spike 只负责把 `NSSelectionAffinity` 转成 FFI 常量并消费标量结果。self-check 临时
  使用 `before **羽🙂** after`，验证同一 source boundary 的 upstream/downstream 视觉位置相同，
  但反向 source boundary 分别停在 delimiter 前后，然后恢复窗口原文。

## 结果

- native caret mapping 与 Markdown hidden syntax 的语义由 shared Rust projection 唯一决定。
- 过期 AppKit/TextKit 位置不能穿透 FFI；UTF-16 surrogate split 也不会被静默取整。
- ABI 仍然没有返回行、点或 CoreText/TextKit 对象；真实 shaped line、viewport 和 block-local
  projection 的几何查询可以在此契约上继续扩展。
- 当前诊断对完整 source range 构建/查询 inline projection；后续产品化时应让 block-local
  layout 直接复用同一映射，避免把 caret query 误当成最终 GUI 布局 API。

