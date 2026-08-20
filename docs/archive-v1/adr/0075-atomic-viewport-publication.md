# ADR 0075：Viewport 发布的 staged builder 提交

## 状态

已接受（Phase 1 诊断）

## 背景

`ViewportFramePublisher::publish` 同时组装 scene、render plan、frame cache 和 publication
serial。`RenderPlanBuilder` 还会保留 atlas page fingerprint，用于跨 frame 去重上传。若直接
使用调用方传入的 builder，组装成功后但 serial overflow、stale cache 或其他发布校验失败时，
builder 可能已经提前推进 fingerprint 状态；下一次重试看到的状态就不再对应一次真正成功的
publication。

## 决策

- `publish` 先 clone 调用方的 `RenderPlanBuilder`，所有 scene/render-plan 组装都写入 staged
  builder。
- frame Revision、publication serial 和 cache handoff 全部通过后，才把 staged builder
  替换回调用方；任一错误都丢弃 staged state。
- cache、last publication、serial 和调用方 builder 共同构成一次逻辑 publication；失败不得
  部分提交其中任意一项。
- retry 使用同一个调用方 builder 时必须重新计算所需 atlas page upload，并最终与一次直接
  成功的 publication 等价。

## 结果

- serial overflow、stale frame/cache 或组装错误不会污染 page fingerprint 去重状态。
- 发布失败后可以用同一 document、atlas 和 builder 重试，失败前的 cache/publication 仍可被
  host 使用。
- staged clone 会复制轻量 fingerprint map，而不会复制 scene、source、layout 或 GPU 资源；
  在发布成功后只做一次 map state move。
- 无窗口测试覆盖 overflow 后 builder/cache/publication 不变，以及随后 retry 成功。
