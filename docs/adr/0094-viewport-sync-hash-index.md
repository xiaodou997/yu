# ADR 0094：Viewport block sync 使用 key 索引

## 状态

已接受（2026-08-14）

## 背景

`EditorDocument::apply_transaction_core` 会把成功的 source edit 映射到
`ViewportLayout`。当 viewport 已经建立了完整 block entry 列表时，旧实现对每个新 Markdown
block 从旧列表头部线性查找未使用的相同 key：

```text
new block 0 → scan old entries
new block 1 → scan old entries
...
```

大文档中的 7 万个 block 会把一次局部编辑放大成 O(blocks²)，掩盖真正的增量 parser 成本。

## 决策

`ViewportKey` 对 `TextRange + BlockKind` 实现 `Hash`，`ViewportLayout::sync` 先构建
`HashMap<ViewportKey, Vec<ViewportEntry>>`，再按新 block 顺序取出可复用 entry。使用 `Vec` 而
不是单一 value 保留重复 key 的安全语义；当前 parser 的 block range 通常已经唯一。

未匹配的旧 entry 继续计入 invalidation，匹配的 entry 保留原有 measured height 和 backend
状态，外部 API 与 Revision 规则不变。

## 结果

在 1 MiB / 约 7 万 blocks fixture 上，4 次随机编辑从约 54 秒降至约 223 ms；raw
`parse_incremental` 的毫秒级成本不再被 viewport cache sync 的二次复杂度吞掉。

## 验证

- `cargo test -p yu-editor -p yu-markdown`
- `cargo clippy -p yu-editor -p yu-markdown --all-targets -- -D warnings`
- `cargo run -p yu-bench --bin yu-session-bench -- --size-mib 1 --iterations 2 --random-edits 4`

