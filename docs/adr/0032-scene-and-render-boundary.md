# ADR 0032：revision-bound scene 与 backend-neutral render plan

## 状态

已接受（Phase 1）

## 背景

CoreText 阶段已经能得到 owned glyph bitmap 和 CPU atlas，但还不能把这些数据直接交给窗口或
GPU。若 layout 直接调用 Metal/wgpu，窗口线程、device loss、texture handle 和 source revision
会被混在一起，后台 scene 也无法安全丢弃。

## 决策

增加两个纯 Rust crate：

### `yu-scene`

- `SceneBuilder` 生成绑定 `yu-core::Revision` 的 retained scene；
- primitive 只包含有限几何、`Rgba8` 和 `yu-font::AtlasEntry` placement，不保存 source text、
  bitmap pixels、native object 或 GPU handle；
- glyph 使用 baseline origin + atlas metrics 计算 bounds，fill rect 使用显式 bounds；
- `DamageSet` 合并相交/相邻矩形，并在超过预算时折叠为总 bounds；
- scene 的 primitive 顺序就是 painter order，viewport 作为 scene-owned geometry 一起传递。

### `yu-render`

- `RenderPlanBuilder` 消费一个 scene 和对应的 CPU `GlyphAtlas`，生成同 revision/viewport 的
  owned `RenderPlan`；
- 生成 backend-neutral `RenderCommand`，不创建 window、device、texture 或 command encoder；
- 通过 page bytes、尺寸和 page id 的 fingerprint 去重 `AtlasPageUpload`；atlas page 发生变化
  时再次上传；
- scene 引用的 atlas entry 必须与当前 atlas 完全一致，否则返回 stale/missing 错误；
- `RenderUploader` 只定义未来 backend 所需的 alpha-page upload 操作，返回的 texture handle
  由 backend 自己拥有。

当前不引入 `wgpu`、Metal 或 Vello 依赖。真实 GPU backend 需要先确定 macOS surface/device
生命周期和 device-loss 策略，再实现 `RenderUploader` 和实际 command encoding。

## 结果

- render preparation 可以在无窗口、无 GPU 的 CI 中测试和 benchmark；
- 旧 revision 的 scene 可以在提交前被丢弃，不会修改 canonical source；
- atlas 上传只按 page 变化发生，多个 glyph 共享同一 page 时不会重复复制；
- scene/render crate 的依赖方向不会把 platform shell 反向引入 editor/layout。

## 限制

- 当前 render plan 是全 scene command list，damage 只作为 backend 的 repaint hints；尚未实现
  GPU clip/scissor、batch sorting、texture eviction 或实际 partial command encoding；
- 只定义 alpha glyph page 和 solid rect，图片、彩色 emoji、paths、selection/caret overlay
  以及 accessibility tree 仍属于后续 renderer 阶段。
