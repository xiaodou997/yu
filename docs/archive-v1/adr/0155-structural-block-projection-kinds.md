# ADR 0155：结构化 block projection kind 与 parser-owned prefix

## 状态

Accepted（Phase 3 Track A；heading/blockquote/list visual projection）。

## 背景

此前所有非 fenced/task/reference block 都以 `Inline` projection 暴露，heading 的 `#` 与
blockquote 的 `>` 因而会被当作普通可见文本。若让 Swift 或 layout consumer 自行扫描这些
delimiter，会产生第二套 Markdown 语义和 source range。列表又不能简单删除 bullet，因为当前
scene 尚未有独立 list-marker primitive。

## 决策

1. Markdown parser 暴露 `block_syntax_hidden_ranges`，只返回结构前缀的 source ranges；它在
   `TextSnapshot` chunk 上扫描，不物化整份 source。
2. `BlockProjection` 新增 `Heading`、`BlockQuote`、`List`，并通过稳定 FFI tag 传播；普通
   list 保留 bullet 与任务文本，task 仍只隐藏 parser-owned `[ ]`/`[x]` marker。
3. heading 隐藏 ATX marker/分隔空白，blockquote 隐藏每个带 `>` 的行前缀；inline emphasis/link
   仍由 parser-owned `InlineSpan` 提供，fence 继续由独立 `CodeProjection` 处理。
4. GFM table 暂不伪装成新的 block kind。它继续由 `parse_table` 独立识别，待 cell layout、
   hit-test 和 source range ABI 一起定义后再升级。

## 结果

- native projection metadata 可以区分 heading、blockquote、list、task 和 fenced code，且
  Swift 不需要解析 Markdown delimiter。
- hidden prefix 的 source/visual mapping 与 Unicode/IME/Revision contract 继续复用同一
  `Projection`；列表 marker 不会在缺少视觉替代物时丢失。
- 下一步可以在 layout/scene 层添加 heading style、blockquote indent 和 list marker，而不
  改变 canonical source 或 projection kind。

## 验证

```bash
cargo test -p yu-markdown -p yu-projection -p yu-editor -p yu-storage-ffi
swift build --package-path experiments/macos-document-host
experiments/macos-document-host/.build/debug/YuMacDocumentHost \
  --block-projection-self-check experiments/macos-document-host/Fixtures/block-projection.md
```
