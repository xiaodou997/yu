# ADR 0095：未查询 viewport 时保持惰性状态

## 状态

已接受（2026-08-14）

## 背景

`EditorDocument` 的编辑路径会把 `ChangeSet` 同步到 `ViewportLayout`。此前只要 layout
保存过当前 revision，即使没有调用过 `visible_blocks`，下一次编辑也会被视为已有 viewport
状态，并触发完整 block entries 的重建。大文档在纯文本编辑场景下因此承担了不必要的
`O(number-of-blocks)` 成本。

viewport entries 只服务于可视范围查询和测量；revision 是校验边界，不代表已经建立了可视
索引。因此二者必须区分。

## 决策

`ViewportLayout::map_through` 只在 `entries` 非空时执行已有 viewport 状态的映射、重建和
同步。如果 entries 为空，方法只更新 revision 并清空 height index；后续第一次
`visible_range`/`visible_blocks` 查询通过 `sync` 按当前 Markdown 文档建立完整条目。

## 结果

- 未请求 viewport 的纯编辑路径不会 materialize 全文 block entries。
- 已经请求过 viewport 的文档仍保留原有的 measured height 和 key-based remap 行为。
- 首次 viewport 查询的成本被推迟到真正需要布局时。
- revision mismatch 校验仍然有效；惰性只影响缓存物化，不改变 source 或 Markdown 语义。

回归测试为 `viewport_state_stays_lazy_until_first_query`，并由
`yu-session-bench --size-mib 1 --iterations 4 --random-edits 4` 的 `local`、`random` 和
`fence-propagation` 场景持续观测。
