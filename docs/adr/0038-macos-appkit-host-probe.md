# ADR 0038：macOS AppKit host probe

## 状态

已接受（Phase 1）

## 背景

0037 已经固定 `NSView` ↔ `CAMetalLayer` 的 scoped attachment、surface generation 和 retained
target，但此前只能在无窗口单元测试中验证 Rust 状态转换。真正的 `CAMetalLayer nextDrawable`、AppKit
main-thread 要求和 view detach 生命周期必须在 macOS 图形 session 中至少跑一次。直接引入完整
产品窗口会把验证边界和应用 shell 混在一起，因此需要一个明确标记为测试用途的最小 host。

## 决策

- Objective-C bridge 提供 probe-only 的 `NSWindow`/`NSView` 创建、销毁和 main-thread callback
  helper；helper 不暴露给 shared crates，也不由生产 `MetalSurface` 创建窗口。
- ignored Rust test 在 callback 内创建 `MetalSurface`、附着临时 view、提交 retained render plan，
  resize surface 后再次附着和提交，再 drop attachment 并销毁 host。
- callback 的 context 只指向测试栈上的 probe state；native bridge 不保存 Rust context，主线程
  callback 返回后立即释放临时 autorelease pool。
- 没有 Metal device、AppKit session 或有效 drawable 时，该测试保持 ignored，不影响默认 workspace
  test 和无窗口 CI。

## 结果

- 在真实 macOS session 中可以验证 attachment、resize generation、drawable acquisition、retained
  target blit 和 scoped detach 的组合生命周期。
- 产品 backend 仍保持“只接收外部 `NSView`、不创建窗口”的所有权边界；probe 不成为 UI 或 demo 模式。
- 后续产品 shell 可以复用相同的 Rust attachment API，但必须自行拥有窗口、事件循环、IME、菜单和
  Accessibility 生命周期。

## 验证命令

默认无窗口检查：

```text
cargo test --workspace
```

有 Metal/AppKit session 时显式运行：

```text
cargo test -p yu-render-macos macos_appkit_attachment_resize_and_drawable_probe_are_live \
  -- --ignored --exact --test-threads=1
```

## 限制

该 probe 只验证 backend boundary 和 frame lifecycle，不验证产品菜单、文档编辑、IME、Accessibility
或最终窗口布局；它不能替代后续原生 shell 的集成测试。
