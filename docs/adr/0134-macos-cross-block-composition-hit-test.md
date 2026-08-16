# ADR 0134：macOS 跨 block composition 的 transient hit-test

状态：Accepted

## 背景

跨 block marked-text 已经可以进入 Rust 的 transient layout 和 Metal publication，但旧的
`yu_storage_session_macos_projection_hit_test` 是 canonical projection、仅 Revision-bound。
它不能安全地把 transient block layout 的命中结果映射回包含 preedit 的 visual UTF-16 坐标，
尤其在 replacement 后续 block 命中时会丢失 preedit 引入的 visual 偏移。

## 决策

新增 `yu_storage_session_macos_composition_projection_hit_test`，要求调用方同时提供当前
canonical Revision 和 composition generation。Rust 完成以下步骤并只返回 owned scalar：

1. 用 composition-aware viewport snapshot 选择 document-space block；
2. 对受影响 block 使用未缓存的 transient CoreText-shaped layout；
3. 将 block-local 命中通过完整 transient projection 映射为 source/visual UTF-16；
4. 同时返回 block index、document-space caret point、visual selection 和 visual replacement；
5. Revision 或 generation 过期时清空输出并拒绝结果。

旧 canonical hit-test ABI 保持不变，供没有活动 composition 的普通 pointer 路径和回退使用。
Swift 只消费该协议，不复制跨 block source→visual 偏移或 Markdown 解析规则。

## 结果

- 跨段落、空行或列表的 preedit 命中与 caret/selection 使用同一个 transient projection；
- 后续 block 的命中坐标仍是 document-space，native host 不需要重建 HeightIndex 或 preedit 偏移；
- composition update/cancel 通过 generation guard 自动拒绝旧 hit，避免旧 geometry 覆盖新状态；
- 完整 visual renderer 迁移前，TextKit 仍可作为输入、Accessibility 和失败回退宿主。

## 验证

```bash
cargo test -p yu-storage-ffi ffi_macos_composition_hit_test_maps_cross_block_transient_coordinates
./experiments/macos-document-host/build-rust-ffi.sh
swift build --package-path experiments/macos-document-host
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --composition-hit-test-self-check \
  experiments/macos-document-host/Fixtures/composition-cross-block.md
```
