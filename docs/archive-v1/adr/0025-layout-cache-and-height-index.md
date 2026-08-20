# ADR 0025：revision-bound layout cache 与 viewport height index

- 状态：Accepted
- 日期：2026-08-10

## 背景

`LayoutSnapshot` 已经可以独立完成 block-local wrapping、caret 和 hit-test，但如果每次
查询都重新从 `BlockProjection` 构建布局，大文档滚动和重复 hit-test 会重复做同一份工作。
同时，未来的 viewport virtualization 需要在不 layout 全文的情况下，根据累计行高定位可见
block。

## 决策

`EditorDocument` 增加独立的 `LayoutCache`，entry 的 key 是：

```text
(Markdown block source range, BlockKind, LayoutConfig.max_width bits,
 LayoutConfig.line_height bits)
```

- 同一 revision 和 config 的 `block_layout` 查询返回 cache-owned snapshot；
- 成功 Transaction 后，cache 先用 `LayoutSnapshot::map_through` 映射严格位于 changed range
  外的 entry，交集或边界变化直接丢弃；
- 增量 Markdown block sequence 更新后，再按新的 `(range, kind)` 做 retention，防止 block
  类型或边界变化错误复用旧布局；
- reset source 清空 cache；revision 不匹配的 entry 也不会被读取；
- `LayoutCacheStats` 暴露 entries、builds、hits、remapped、invalidated 计数，供 benchmark
  和回归测试使用。

`yu-layout::HeightIndex` 使用 Fenwick tree 保存视觉行高：prefix、point update 与
`find_line(y)` 均为 O(log n)，并且只依赖纯 Rust 数据结构。它是后续跨 block viewport
virtualization 的索引基础，不负责布局、窗口或 GPU。

## 结果

- 重复 layout 查询不会重新扫描 projection；
- 前缀插入等不影响 block 内容的 edit 可以保留 visual 坐标，只更新 source ranges 和 revision；
- block 结构变化是保守失效的，不会把段落布局当成 heading 布局；
- 高度索引可以独立 benchmark 和 fuzz，不把 GUI 生命周期提前引入核心 crate。

## 限制

当前 cache 的 retention 是 block 数量上的线性扫描，layout snapshot 仍是 block-local，
`HeightIndex` 默认由 snapshot 的统一 line height 构建；真实字体 shaping、变量行高、跨 block
虚拟化和 GPU scene 仍属于后续阶段。跨 block 的估计/实测状态和 viewport 迭代规则由 ADR 0026
定义。
