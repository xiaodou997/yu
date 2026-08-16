# ADR 0147：图片 intrinsic metadata、退避重试与 viewport 调度基准

## 状态

Accepted（Phase 3 Track C）

## 背景

0146 让 ready 图片真实高度进入 HeightIndex，并在 GPU atlas 中淘汰离屏 texture，但 decoded
CPU publication 也可能受容量限制被淘汰。如果尺寸与像素绑定，图片在下一帧会回到 placeholder
高度，导致滚动位置跳动。另一个问题是失败请求如果永远只记录一次，会把可恢复的文件/worker
错误永久冻结；如果每帧重试，又会造成 ImageIO 队列风暴。

## 决策

1. `yu-assets::ImageCache` 将 `ImageDimensions` 与 `DecodedImage` 分开保存。decoded entries
   继续使用显式容量的 LRU；intrinsic metadata 使用独立、有界的 metadata LRU（默认容量至少
   256），并通过 `ImageIntrinsicPublication` 按当前 Revision 重绑定。metadata 只包含宽高和
   ImageKey，不包含 source 文本、像素、worker 或 GPU 对象。
2. `ImageRetryPolicy` 使用逻辑 frame tick、最大尝试次数和有界指数退避。`MacosImageResourceState`
   在每次 viewport batch 开始推进 tick；退避窗口内不会重复排队，到期后允许重新进入 pending，
   达到上限后在当前 Revision 保持 fallback。新 Revision 自动清除旧失败。
3. workspace/core-text publisher 同时接受 ready `ImagePublication` 与 metadata-only
   `ImageIntrinsicPublication`。这样 image placement、block height、content height 和 max
   scroll 在像素被淘汰后仍使用相同 intrinsic 尺寸；RenderPlan 仍只有 ready publication 才能
   产生可采样 texture，未 ready 时继续使用 fallback rectangle。
4. 添加 `yu-image-scheduling-bench`，生成大规模 image block 文档并在 overscan 0/160/640px
   下重复 viewport 查询，记录 resolver candidate calls、measured blocks 与 elapsed time。
   该基准验证调度只依赖 viewport/overscan，而不是扫描整篇 Markdown；它不是产品可视化模式。

## 结果

- CPU/GPU 像素淘汰不会使已知图片的 intrinsic block 高度退回 placeholder。
- 可恢复的 decode/IO/worker 错误会在受控次数内自动重试，不会每帧重复提交。
- 图片请求仍是窗口化的；overscan 增大只线性增加候选窗口，不会把 100,000 个 block 全部排入队列。

## 验证

```text
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p yu-bench --bin yu-image-scheduling-bench --release -- 2000 100
```

`yu-assets` 单元测试覆盖 metadata 在 decoded LRU eviction 后仍可重绑定，以及退避窗口和最大
尝试次数；`yu-workspace` 覆盖 metadata-only scene；macOS surface self-check 继续覆盖 ready
intrinsic reflow、离屏 Metal atlas eviction 和 stale publication。
