# ADR 0112：macOS projection selection 与 point hit-test FFI

## 状态

已接受（Phase 3 Track A，诊断边界）。

## 背景

Yu 已能从同一 `DocumentEditorSession` 返回 source-backed visual projection、block snapshot
和 composition overlay，但 native host 仍不能把一个 source selection 或 mouse/view-local
point 映射回 projection。若 Swift 自己隐藏 `**`、`*`、link delimiter 或按 visual 文本计算
行宽，就会产生第二套坐标模型，并在 Unicode、line ending、Revision 变化时漂移。

## 决策

storage FFI 新增两个 Revision-bound 查询：

1. `yu_storage_session_projection_selection` 接收 source UTF-16 起止和 affinity，返回 visual
   UTF-16 selection 以及两端 source round-trip。非折叠 selection 使用 projection 的 Before/After
   外缘，collapsed range 保留 caller affinity；hidden delimiter 不进入 visual selection。
2. `yu_storage_session_projection_hit_test` 接收 layout-local point 与显式 metrics 配置，复用
   `DocumentEditorSession` 的 full-source projection 和 `yu-layout::LayoutSnapshot`，返回 snapped
   source/visual caret、line、projection-local point 与 affinity。

两种查询都先校验 expected Revision；任何 stale、surrogate split、未知 affinity、无效配置或
非有限 point 都拒绝并清空 output。返回值只包含 owned scalar，不返回 Rust pointer、Projection、
LayoutSnapshot 或 AppKit 坐标。

## 结果

- native selection/hit-test 可以共享 Rust 的 source↔visual↔point 语义，不需要在 Swift 解析
  Markdown 或猜 delimiter。
- layout point 明确是 full-projection-local metrics 坐标；screen/view 转换仍由平台层负责。
- 当前只增加 self-check 和 bridge，不替换生产 TextKit source mirror；真实鼠标拖选、viewport
  origin、CoreText shaped layout 和 visual mirror 留在 Track B。

## 验证

```bash
cargo test -p yu-storage-ffi ffi_projection_selection_and_hit_test_round_trip_visual_coordinates
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --projection-hit-test-self-check experiments/macos-document-host/Fixtures/projection.md
```
