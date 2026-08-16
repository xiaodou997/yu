# ADR 0132：macOS visual count/fill 绑定 composition generation

状态：Accepted

## 背景

`yu_storage_session_macos_visual_render_plan` 和
`yu_storage_session_macos_visual_scene_glyphs` 使用两次 FFI 调用完成 count/fill。普通编辑会
推进 canonical `Revision`，但 IME marked-text 的 update/cancel 只推进 transient composition
generation。若 native caller 在两次调用之间收到 IME 更新，旧 header 的容量可能被用于新的
glyph 数组；仅校验 Revision 无法发现这次错配。

## 决策

视觉 render-plan、retained glyph scene、persistent host 和 surface snapshot 都携带
`composition_generation`。Rust 在 fill capacity 非零时读取调用方保留的 count header，验证：

```text
prior Revision       == expected Revision
prior generation     == current session generation
```

验证失败时清空 header 与 written 计数，不写入任何数组，并分别返回 stale Revision 或
`YU_STORAGE_STALE_COMPOSITION`。首次 count 查询仍使用清零 header，不需要额外的 expected-generation
参数；这保持 C ABI 的调用形状不变。

Swift 只消费 Rust 返回的 generation，并把它纳入产品 Metal submit key；它不自行推断或缓存
composition 文本。host/surface 输出也回传同一 generation，便于 self-check 和后续真实 renderer
接管时继续保持同一 identity。

## 结果

- 两次 count/fill 之间的日文、emoji、dead-key preedit 更新不会污染旧数组；
- cancel 即使不改变 source Revision，也会使旧 transient header 失效；
- ABI 没有增加独立的 generation 参数，旧的 count/fill 使用方式保持不变；
- canonical scene/render-plan 仍由 source Revision 驱动，跨 block preedit 仍按现有安全回退策略处理。

## 验证

```bash
cargo test -p yu-storage-ffi --lib
./experiments/macos-document-host/build-rust-ffi.sh
swift build --package-path experiments/macos-document-host
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --macos-render-host-self-check experiments/macos-document-host/Fixtures/projection.md
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --macos-render-host-surface-self-check experiments/macos-document-host/Fixtures/projection.md
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-render-plan-self-check experiments/macos-document-host/Fixtures/projection.md
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-scene-glyph-self-check experiments/macos-document-host/Fixtures/projection.md
```
