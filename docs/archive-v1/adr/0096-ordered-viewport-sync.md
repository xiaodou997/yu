# ADR 0096：Viewport entries 优先使用有序 merge

## 状态

已接受（2026-08-14）

## 背景

ADR 0094 用 `ViewportKey → entry` 哈希索引修复了完整 viewport entries 同步时的
`O(blocks²)` 问题。但在正常编辑中，entries 和 Markdown blocks 都按 source range 有序；每次
局部编辑只会移除受影响 block，随后用哈希表重建剩余条目仍会产生完整索引分配和扫描成本。

## 决策

`ViewportLayout::sync` 先检查旧 entries 的 source range 是否非递减。若有序，则用两个游标对
旧 entries 和新 blocks 做线性 merge：相同 `TextRange + BlockKind` 保留原 measured height，过期
或 kind 不同的旧 entry 计入 invalidation，新 block 使用 estimate。只有旧 entries 顺序异常时
才回退到 ADR 0094 的 `HashMap<ViewportKey, Vec<ViewportEntry>>` 路径。

## 结果

- 正常局部/结构编辑不再为每次同步分配完整 HashMap。
- measured height、backend 和 Revision 语义不变；结构变化仍能丢弃受影响 entry。
- fallback 保留重复 key 和异常顺序下的安全行为。
- 1 MiB、先 materialize viewport、4 次编辑的局部/随机场景从约 90 ms/次降至约 37 ms/次。

## 验证

- `cargo test -p yu-editor viewport_`
- `cargo test -p yu-storage unified_session_exposes_viewport_without_a_second_editor_handle`
- `cargo run -p yu-bench --bin yu-session-bench -- --size-mib 1 --iterations 2 --random-edits 4`
