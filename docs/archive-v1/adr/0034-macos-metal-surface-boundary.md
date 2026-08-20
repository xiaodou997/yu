# ADR 0034：macOS Metal surface 与 atlas upload 边界

## 状态

已接受（Phase 1）

## 背景

`yu-render` 已经生成 backend-neutral `RenderPlan`，但真实 macOS 绘制还需要创建
`MTLDevice`、配置 `CAMetalLayer`，并把 owned alpha atlas page 变成 `MTLTexture`。把这些对象
放进共享 renderer 会让 Objective-C 生命周期、窗口线程和 device loss 污染 editor/scene 契约；
直接引入 wgpu 又会在没有确定 surface 生命周期前扩大依赖和构建成本。

## 决策

增加 macOS-only crate `platform/macos/yu-render-macos`，边界分成两层：

### Rust 层

- `MetalDevice` 持有系统 device 的生命周期和只读 registry id；
- `MetalSurfaceConfig` 验证 logical size、scale，并计算 drawable pixel size；
- `MetalSurface` 持有未附着窗口的 `CAMetalLayer`，resize 成功后递增 generation；
- `MetalUploader` 实现共享 `yu-render::RenderUploader`，把 alpha `AtlasPageUpload` 上传为
  `MetalTexture`；
- native pointer 只存在于 platform crate 的私有类型，不能进入 `yu-scene`、`yu-layout` 或
  editor canonical state。

### Objective-C bridge

`native/metal_bridge.m` 是最薄的 Apple framework 调用层，只使用 Metal/QuartzCore/Foundation
完成：

- `MTLCreateSystemDefaultDevice`；
- `CAMetalLayer` 创建与 drawable size/scale 配置；
- `MTLTextureDescriptor(R8Unorm)` 创建和 `replaceRegion` alpha upload；
- retained native object 的 release。

当前不创建 NSWindow/NSView；clear-only drawable acquisition 和 present/commit 由 ADR 0035
单独定义，glyph/rect command encoding 仍不属于本阶段。这样真实窗口和完整 renderer pipeline
仍可独立演进。

## 测试策略

- 所有平台默认测试 surface config 的尺寸验证；
- macOS native device/surface/upload 测试保留为 `#[ignore]` opt-in，因为 CI 或无图形会话可能
  没有 Metal device；
- 在有 Metal-capable macOS session 时运行：

```text
cargo test -p yu-render-macos -- --ignored
```

## 结果

- shared render crate 没有新增 wgpu/Metal 依赖；
- Metal texture/device/layer 生命周期被隔离在 macOS crate；
- atlas upload 的真实硬件路径可以在不启动完整 GUI 的情况下验证；
- 后续接窗口时只需把已配置 layer 附着到平台 view，再扩展 glyph/rect pipeline。

## 限制

- 当前 bridge 使用小型 Objective-C C ABI，尚未接入 glyph pipeline、shader、clipping 或 damage
  scissor；clear-only frame 仍受 drawable/window 上下文限制；
- Metal device unavailable 时默认测试不会失败，必须显式执行 ignored test 才能完成硬件验证；
- 纹理上传目前是 shared `R8Unorm`，彩色 emoji、图片和 private-storage staging 属于后续阶段。
