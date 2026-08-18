# ADR 0169：macOS table divider 的只读 Accessibility descriptor

## 状态

已接受（2026-08-18；可操作 splitter 由 ADR 0170 承接）

## 背景

ADR 0168 保留了完整 splitter Accessibility element，原因是 native 层还没有稳定的 divider
几何枚举协议。当前 CoreText-shaped table layout 已能在同一 Revision 下计算可见表格的列边界，
因此可以先固定一个不改变文档的 descriptor ABI，供 VoiceOver element、键盘增减动作和诊断工具
共同使用。

## 决策

1. storage FFI 新增 `yu_storage_session_macos_table_resize_accessibility_dividers` count/fill
   查询。输入使用与 retained surface 相同的字体、宽度、scroll 和 viewport；输出包含 Revision、
   block/column index、column count、document-space divider rect，以及 table source UTF-16 range。
2. 查询只枚举当前 viewport 内的内部 column divider。它不打开 `TableResizeGesture`，不修改
   source、selection、history、layout cache 或 surface submit state；active composition 时返回空集合，
   避免把 transient composition layout 暴露成稳定 AX target。
3. Swift 只保留 owned scalar descriptor，并通过 coordinator 转发查询；Swift 不扫描 Markdown
   pipe、不推断 block 高度，也不持有 Rust layout。可操作的 NSAccessibility splitter element、
   increment/decrement 步长、stale element 销毁通知和 source-neutral preview 语义在后续 ADR 0170
   中定义；本 ADR 仍是其几何 ABI 基础。

## 结果

- VoiceOver/native adapter 现在有一个可验证、Revision-bound 的 divider geometry 契约。
- descriptor 与 pointer hover/begin 使用同一 CoreText document-space 坐标，未来不需要第二套表格
  测量逻辑。
- 由于 descriptor ABI 本身是只读的，它不会伪造 source edit，也不会把像素宽度写入 Markdown；
  可操作 native splitter 的生命周期由 ADR 0170 另行负责。
- ADR 0170 在不改变本 descriptor ABI 的前提下，将其投影为临时 native splitter；本 ADR 的
  count/fill 与 Revision 约束保持不变。

## 验证

- Rust macOS FFI test 覆盖 count/fill、一致的 divider x、column count、document source range
  和 source 不变。
- macOS table resize coordinator self-check 消费同一 descriptor，验证 Revision、rect 和目标列
  与 pointer divider 一致。
