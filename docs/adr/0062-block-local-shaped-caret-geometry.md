# ADR 0062：Block-local Shaped Caret Geometry

## 状态

已接受（Phase 1 诊断）

## 背景

ADR 0061 固定了 source caret 到 parser-owned block projection 的局部映射，但只返回
visual UTF-16。原生输入、命中测试和后续 reveal 还需要真实字体 shaping 后的 line、point 和
line height；如果平台重新构造一份 Markdown 文本或直接持有 Rust layout 对象，source projection
和 CoreText layout 就会出现两个正确性边界。

## 决策

- `yu-editor-ffi` 增加 `YuBlockShapedCaret` 和
  `yu_macos_composition_session_block_shaped_caret`。查询携带 expected Revision、source
  UTF-16、CaretAffinity、字体 size 和 block max width。
- macOS 实现使用 `CoreTextShaper::from_system_ui` 与 `EditorDocument::block_layout_with_shaper`。
  shaped layout 由 `EditorDocument` 的 revision-bound layout/projection cache 维护；平台不复制
  Markdown parser，也不接收 `Projection`、`LayoutSnapshot` 或 CoreText 句柄。
- 结果只返回 owned scalar：Revision、source/block-local visual UTF-16、round-trip source、line
  index、block-local x/y、零宽 caret 和 native line height。Before/After affinity 在 hidden
  delimiter 边界上保留不同的 source round-trip，但相同 visual point。
- 非 macOS 保留同名 ABI 并返回 `YU_FFI_CORE_TEXT_UNAVAILABLE`，避免跨平台 header 分叉；错误时
  先清空 output。stale Revision、surrogate split、未知 affinity、非法尺寸和布局失败不得修改
  document、selection、composition、history 或 Revision。

## 结果

- macOS 可以在不进入完整 GUI 的情况下验证从 source boundary 到真实 CoreText caret geometry 的
  闭环，并可将 block origin、scroll container 和 AppKit 对象留在平台层。
- shaped geometry 与 projection caret 是两个窄 ABI：前者消费后者的 block-local 坐标，但不把
  平台字体状态写入 canonical editor model。
- 当前接口仍是诊断/布局边界，不承诺 TextKit 的自然语言断行完全等价，也不实现完整 native
  viewport 或 renderer。
