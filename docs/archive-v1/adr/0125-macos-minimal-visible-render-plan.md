# ADR 0125：macOS product minimal visible RenderPlan projection

## 状态

已接受（Phase 3 Track C；这是完整 visual editor 之前的最小可见渲染切入点）。

## 背景

0124 已把 Rust-owned `MetalSurface` 接入真实 document-host `NSView` 生命周期，但 surface
仍位于 source TextKit mirror 后方，因此真实 RenderPlan 虽然提交到 drawable，用户看不到它。
下一步不能直接删除 TextKit：它仍然是当前稳定的键盘、IME、复制粘贴、caret、selection、
Accessibility 和渲染失败回退表面。需要先让既有 glyph RenderPlan 在产品窗口中可见，同时保持
source/selection/history 只有 Rust 一份。

## 决策

- `MacosSurfaceHostView` 作为 source TextKit mirror 的 sibling 放在其上方，但覆盖范围只同步
  到 `NSScrollView.contentView`，不覆盖原生 scroller。
- surface 只有在一次带当前 Revision、viewport、scale 的 Metal submit 成功后才显示；首次绑定、
  submit 异常、view 离开 window、detach 都隐藏 surface。隐藏 surface 不会改变 Rust document、
  TextKit source mirror 或 selection state。
- surface view 的 `hitTest` 固定返回 `nil`。鼠标、键盘、IME composition、复制粘贴、caret 和
  VoiceOver 继续落到 TextKit/Accessibility adapter；Rust surface 只提供视觉 glyph projection。
- CAMetalLayer 设置为透明。RenderPlan 的 glyph coverage 叠加在 source mirror 上，未绘制像素
  透出 TextKit 的背景、caret 和 selection。该阶段不引入第二份文本、DOM 或可视化演示模式。
- surface 失败是非模态诊断状态：status tooltip 记录错误，TextKit mirror 继续可编辑；后续
  resize/scroll/edit/窗口布局会再次尝试成功提交。

## 结果

产品窗口首次拥有可见的 Rust RenderPlan 路径，且仍可在 Metal 不可用或提交失败时安全回退。
`yu_storage_session_macos_render_host_surface_submit` 继续是唯一 surface/atlas/renderer 所有者；
Swift 只持有 `NSView` lifecycle 和 owned scalar snapshot。输入、IME、selection、Accessibility
和 canonical Markdown source 没有迁移到 overlay。

## 非目标

- 不在这一阶段删除 TextKit 或把它变成只读占位。
- 不实现最终 Markdown delimiter hide/reveal、鼠标 hit-test、caret/selection 绘制或表格 overlay。
- 不增加可视化演示模式、额外窗口或第二份 document model。

## 验证

```bash
swift build --package-path experiments/macos-document-host
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --macos-render-host-lifecycle-self-check \
  experiments/macos-document-host/Fixtures/block-projection.md
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --macos-render-host-surface-self-check \
  experiments/macos-document-host/Fixtures/block-projection.md
```
