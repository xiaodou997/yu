# ADR 0064：Shaped Viewport Block Snapshot

## 状态

已接受（Phase 1 诊断）

## 背景

ADR 0026 已经在 Rust 内部定义了 `ViewportSnapshot`，ADR 0063 又让 macOS caret reveal 使用
CoreText shaped backend。下一步如果让 Swift 为 scene 或 document view 自己推导 block origin、
source range 和高度，就会重新产生第二套 Markdown/viewport 状态；如果直接暴露 Rust snapshot，
又会穿透 FFI 所有权边界。

## 决策

- 增加 `yu_macos_composition_session_shaped_viewport_blocks`，使用 count/fill ABI 返回当前
  Revision 的 shaped viewport metadata。
- header 返回 Revision、选中 block index range 和 content height；每个 owned block scalar
  返回 block index、source UTF-16 range、document-space `y`/`height`、measured 标记和稳定的
  `YU_VIEWPORT_BLOCK_*` kind tag。
- Rust 使用 `EditorDocument::visible_blocks_with_shaper`、`ViewportLayout` 和 `HeightIndex`。
  host 必须先发布匹配的 CoreText width/line metrics；stale Revision、非法参数、capacity 不足
  和布局失败都不能写入部分 block 数组。
- Swift 只把 metadata 转成临时值供 self-check/后续 scene 使用；`ViewportSnapshot`、Markdown
  block、Projection、LayoutSnapshot 和平台对象都不穿过 ABI。非 macOS 返回
  `YU_FFI_CORE_TEXT_UNAVAILABLE`。

## 结果

- 原生 host 获得了可直接交给 scene/document view 的 block origin 与 source range，不需要复制
  Markdown parser 或 block-height 算法。
- count/fill 可以在大文档中只申请可见窗口大小的数组；容量不足时 header/count 仍可用于重试，
  但 caller-owned block storage 保持不变。
- 当前 snapshot 只包含 block metadata，不包含 glyphs、images、scene primitives 或 scroll view；
  这些仍属于后续 viewport/scene/render 阶段。
