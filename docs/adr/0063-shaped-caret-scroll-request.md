# ADR 0063：macOS Shaped Caret Scroll Request

## 状态

已接受（Phase 1 诊断）

## 背景

ADR 0053/0055 已经证明 metrics-only `CaretScrollRequest` 可以穿过 Rust FFI 被
`NSScrollView` 消费，但它使用的是配置中的 metrics layout。上一阶段新增了 block-local CoreText
caret geometry；如果 macOS host 仍只调用 metrics-only scroll 查询，真实字体换行和 block 高度
就不会进入 caret reveal 路径。

## 决策

- 增加 `yu_macos_composition_session_shaped_caret_scroll_request`。它携带 expected Revision、
  CoreText System UI size/max width、当前 scroll/viewport/margin，返回与既有 ABI 相同的 owned
  absolute `YuEditorCaretScrollRequest`。
- 查询复用 `EditorDocument::caret_scroll_request_with_shaper`、当前 `ViewportConfig` 的
  estimated height/overscan 和 revision-bound `ViewportLayout`。调用前 host 必须通过
  `yu_composition_session_set_viewport_config` 发布与 size/max width 对应的 CoreText metrics；
  不匹配时返回 `YU_FFI_INVALID_VIEWPORT_CONFIG`，查询不会悄悄重置已有 height measurements。
- Rust 负责 shaped block layout、HeightIndex 前缀高度、document-space caret y 和 target clamp；
  Swift `YuNativeViewportAdapter` 仍只负责 Revision 检查、native content/clip height clamp 和
  `NSClipView` bounds。
- 非 macOS 保留稳定 ABI 并返回 `YU_FFI_CORE_TEXT_UNAVAILABLE`。stale Revision、非法尺寸、
  viewport 参数或布局失败必须先清空 output；查询不得修改 canonical source、selection、
  composition、history 或 Revision。

## 结果

- macOS spike 的真实 host 可以在 CoreText shaped backend 下走同一条 caret reveal 链，而无需在
  Swift 侧复制 Markdown/block height 计算。
- metrics-only 查询继续保留为 fallback 和对照测试；shaped 查询只增加 layout backend/cache
  工作，不把平台字体句柄或 AppKit 对象带入 Rust ABI。
- 当前仍是产品 GUI 前的风险验证；图片、嵌入 block 和最终 renderer 的 block origin 仍由后续
  viewport/scene 阶段接入。
