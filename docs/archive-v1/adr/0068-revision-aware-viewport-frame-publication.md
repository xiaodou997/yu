# ADR 0068：Revision-aware Viewport Frame Publication

## 状态

已接受（Phase 1 诊断）

## 背景

`yu-workspace` 已经能把一个 `EditorDocument` viewport 组装成 `Scene` 和 `RenderPlan`。真实
应用会在编辑线程之外准备字体、atlas、布局或 Metal command；这些工作完成时，文档可能已经
进入了更新的 Revision。若 host 直接替换当前 frame，旧 scene/render plan 就可能覆盖新文本。

## 决策

- `ViewportRenderFrame` 同时拥有 `ViewportSceneFrame` 与 `RenderPlan`，构造时要求二者 Revision
  相同。
- `assemble_viewport_render_frame` 在 scene 组装成功后生成同 Revision render plan；render-plan
  builder 只有成功后才提交自己的 atlas page fingerprint 更新。
- `ViewportFrameCache::publish_if_current(current_revision, frame)` 只接受 frame Revision 等于
  调用方当前 Revision 的结果；stale frame 被拒绝，缓存保持原值。
- 缓存不会让较旧 Revision 回退覆盖较新 Revision；编辑后 host 可以调用 `invalidate_stale`
  丢弃旧 frame，再发布新 frame。
- cache 只保存最新一份 frame，不持有 source、EditorDocument、HeightIndex、native object 或
  GPU handle；后台 worker 仍只能通过 owned frame 与平台层交接。

## 结果

- scene、render plan 和平台发布拥有统一的 Revision gate，旧后台任务可以安全完成后再被丢弃。
- frame replacement 是单入口、可测试的行为；macOS host 不需要自行复制 stale 检查或维护第二套
  viewport generation。
- 当前 cache 是单文档、单 viewport 的最小实现；多 tab、多 viewport 或 GPU retained target
  需要在相同 Revision gate 上扩展，而不是绕过它。
