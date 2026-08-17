# ADR 0166：macOS table resize gesture 使用 session-owned CoreText geometry

## 状态

已接受（2026-08-17）

## 背景

`yu-workspace` 已经能够把 `TableResizeCommit` 应用到 retained scene/render plan，但如果
macOS host 用 metrics-only table hit-test 捕获 divider，再把 delta 交给 CoreText frame，
不同 shaper 的自然列宽可能让 pointer anchor 与最终 scene 错位。此前 FFI 只有一次性的
hit-test 和 `layout_cells_with_resize`，没有 begin/update/finish/cancel 的生命周期契约。

## 决策

1. `YuStorageSession` 持有一个可选的 `TableResizeGesture` 和 session-only preview；native
   通过 `table_resize_begin/update/finish/cancel` 驱动它。gesture 始终绑定捕获 Revision，
   stale 或非有限 pointer 会拒绝并丢弃当前 gesture。
2. 通用 begin 保留 metrics-only 诊断语义；macOS retained host 使用
   `yu_storage_session_macos_table_resize_begin`，按传入 font size 创建 CoreText shaper，
   在相同 `LayoutConfig` 下命中 divider。
3. update 的 preview 和 finish 的 commit 都只描述 geometry，不产生 Markdown transaction。
   finish 保留最终 column preview 供后续 retained frame 使用；cancel 清除 preview。row
   gesture 可以完成生命周期诊断，但在 variable-row layout 完成前不进入 scene override。
4. macOS render-host 每次 frame 从 session preview 构造 `ViewportRenderConfig::with_table_resize`；
   因此普通 frame 和 surface submit 自动消费同一份 transient layout。source、selection、
   history、HeightIndex 和 canonical layout cache 不变。

## 结果

- pointer begin、drag frame、release/cancel 的 Revision 和 geometry 由 Rust 单一持有。
- macOS hit-test 与 CoreText scene 使用同一字体/shape 坐标，不再让 native 层自行换算 divider。
- finish 后的最终视觉宽度仍是 session-only；真正 Markdown 表格格式化/持久化另行设计。
- 旧的一次性 metrics-only FFI 保持兼容，便于非 macOS 诊断和 deterministic model test。

## 验证

- Rust FFI lifecycle test 覆盖 begin/update/finish/cancel、重复 begin、stale 和 source 不变。
- macOS FFI test 先从 retained CoreText frame 捕获 divider，再执行 shaped begin/update，确认
  transient border 在下一帧移动，finish/cancel 后恢复 canonical border。
- document-host Swift render-host self-check 覆盖同一生命周期并验证 frame serial 单调递增。
