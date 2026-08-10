# ADR 0026：纯 Rust 跨 block viewport layout

- 状态：Accepted
- 日期：2026-08-10

## 背景

`HeightIndex` 已经能对一组视觉高度做 O(log n) 查询，但编辑器仍缺少跨 Markdown block 的
窗口模型。如果为了得到滚动位置而先 layout 全文，大文档会失去增量和虚拟化的收益；如果
完全不知道不可见 block 的高度，又无法定位目标 block。

## 决策

`yu-editor::ViewportLayout` 保存每个当前 Markdown block 的轻量状态：

```text
(source range, BlockKind, height, measured)
```

- 未测量 block 使用 `ViewportConfig.estimated_block_height`；
- `visible_blocks(ViewportRect)` 先用 `HeightIndex` 加上 overscan 找到候选区，只调用
  `EditorDocument::block_layout` 测量候选 block；
- 实测高度按 `line_count * LayoutConfig.line_height` 写回 `HeightIndex`，最多迭代有限轮次
  重新定位窗口；
- 结果是 revision-bound `ViewportSnapshot`，只包含 block index、source range、kind、y、
  height 和 measured 标志，不创建 GUI/GPU 对象；
- 成功 Transaction 后，严格位于 changed range 外的 entry 通过 `ChangeSet` 映射，触碰范围或
  block kind/boundary 改变的 entry 丢弃；`reset_source` 清空状态；
- `ViewportStats` 暴露当前 entry/measured 数量及 remapped/invalidated 计数，方便 benchmark
  验证是否意外 layout 全文。

## 结果

- 首次查询只测量 viewport/overscan 内的 block，其余 block 不会因为滚动定位而构建 layout；
- 重复查询会命中既有 `LayoutCache`，只重新读取高度索引；
- 前缀插入不会让已测量的后缀 block 回到 estimate，source 坐标会随 Revision 更新；
- 该模型可以在没有窗口、字体 shaping 或 GPU 的环境中做正确性和性能测试。

## 限制

当前 block 状态的同步/retention 是轻量的线性扫描，测量窗口使用有限轮次收敛；估计高度在
首次滚动到远处时可能导致 y 位置短暂调整。真实字体 shaping、变量行高、anchor-preserving
scroll correction、异步 layout、scene virtualization 和平台滚动条仍属于后续阶段。
