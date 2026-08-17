# ADR 0148：图片请求计划与调度可观测性

## 状态

Accepted（Phase 3 Track C）

## 背景

上一阶段已经限制图片请求来自 viewport/overscan，并加入了 intrinsic metadata 与退避重试。
但 storage host 仍直接把 block 扫描结果交给 cache：同一个 destination 在多个 block 出现时，
调度顺序和重复请求数没有统一契约，surface snapshot 也无法区分“候选很多”和“去重后实际工作多”。

## 决策

1. `yu-assets::ImageRequestPlan` 接受带 `block_index` 和 `ImageRequestPriority` 的候选 occurrence，
   按 `ImageKey` destination 去重。Visible candidate 优先于 overscan candidate；相同优先级按
   block index 和 fingerprint 稳定排序。Plan 同时保存 candidate、unique、duplicate 以及
   visible/overscan 分布统计。
2. macOS CoreText builder 从同一 `ViewportSnapshot` 的 document-space block geometry 标记
   visible/overscan priority。storage FFI 只把 plan 的 unique requests 送入 `ImageCache`，避免
   worker 层重复排队；cache 的逻辑 retry tick 继续负责退避，state 记录实际 retry enqueue 次数。
3. `YuStorageMacosRenderHostSurfaceSnapshot` 增加 candidate、duplicate、visible candidate、
   overscan candidate 和 retry counters。它们是当前 batch 或当前 surface 生命周期的诊断 scalar，
   不改变 canonical source、RenderPlan resource ownership 或 image fallback 行为。
4. `yu-image-scheduling-bench` 在 viewport/overscan 查询后运行同一 `ImageRequestPlan`，输出
   unique scheduled requests 与 duplicate candidates，确保大文档调度 benchmark 覆盖真实去重
   契约而非只测 layout resolver。

## 结果

- 同一 viewport batch 内同 destination 只会产生一个 worker request，且当前可见图片不会排在
  overscan 预取之后。
- self-check 可以验证 candidate 数、去重数和 visible/overscan 分解守恒；失败重试可独立观察。
- 调度诊断停留在 scalar ABI，不把 Markdown、路径字符串、decoded bytes 或 GPU 对象复制到 Swift。

## 验证

```text
cargo test -p yu-assets -p yu-storage-ffi -p yu-workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p yu-bench --bin yu-image-scheduling-bench --release -- 100000 20
swift run --package-path experiments/macos-document-host \
  YuMacDocumentHost --macos-render-host-surface-self-check \
  experiments/macos-document-host/Fixtures/render-images-plan.md
```
