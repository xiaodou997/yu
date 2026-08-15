# ADR 0117：Revision-bound visual scene snapshot bridge

## 状态

已接受（Phase 3 Track C，native scene 诊断边界；不是生产渲染路径）。

## 背景

`yu-scene` 已经拥有 `ViewportSceneInput` 与 `SceneBuilder`，但 macOS 文档壳目前只能消费
shaped viewport 的 block geometry。若 Swift 自己把这些 geometry 拼成背景、文本区域或 damage，
就会再次复制 scene 的顺序、来源范围和 Revision 规则；若此时直接接入完整 glyph/Metal，又会把
生产 TextKit、GPU surface 和资源生命周期同时引入，难以定位边界错误。

## 决策

- `yu-storage-ffi` 新增 `yu_storage_session_macos_visual_scene` count/fill ABI。Rust 先按
  expected Revision 取得 CoreText-shaped `ViewportSnapshot`，构造 `ViewportBlockGeometry`，再由
  `ViewportSceneInput::new` 和 `SceneBuilder` 验证并生成最小 retained scene。
- 当前 diagnostic scene 为每个可见 block 生成两个 owned rectangle primitive：背景和文本 ink
  bounds。它们只返回 Revision、block index、source UTF-16 range、kind 与有限矩形；不跨边界暴露
  Markdown node、`LayoutSnapshot`、glyph atlas、GPU handle 或 AppKit object。
- snapshot header 携带 block range、content height、scroll/viewport/max-scroll 和 primitive count。
  count/fill 的容量不足不得写入部分 primitive；stale Revision 必须清空 header/写入计数并返回
  `YU_STORAGE_STALE_REVISION`。
- Swift 只消费 owned scalars，并在 `--visual-scene-self-check` 中验证 painter order、同一 block
  的 source range、document-space y/height、viewport bounds 和 stale 丢弃。生产文档 view 继续走
  source TextKit mirror；此桥不改变窗口、IME、复制粘贴或 Accessibility 的所有权。

## 结果

- Rust 的现有 scene 输入和 builder 得到一次真实 native handoff 验证，Swift 不再拥有第二套
  primitive 顺序或 block geometry 规则。
- primitive 是可丢弃的 Revision-bound snapshot；编辑后旧 scene 无法进入 native 数组，未来可在
  同一 ABI 形状上替换为真正的 glyph/image scene payload。
- 当前文本矩形只是协议探针，不宣称完成 heading/emphasis/code/link 的视觉渲染，也不连接生产
  Metal surface。下一步应在该边界上加入 glyph atlas/RenderPlan 的 owned publication，再做真实
  damage 和 GPU 提交。

## 验证

```bash
cargo test -p yu-storage-ffi ffi_macos_visual_scene_is_owned_count_fill_and_revision_bound
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-scene-self-check experiments/macos-document-host/Fixtures/block-projection.md
```
