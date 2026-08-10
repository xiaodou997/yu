# ADR 0021：在 EditorDocument 内维护 revision-bound ProjectionCache

- 状态：Accepted
- 日期：2026-08-10

## 背景

Projection 已经能从 parser-owned inline token ranges 建立 source-backed visual runs，但 GUI 或
平台适配层不能自行保存一份脱离 `EditorDocument` 的投影。编辑后如果继续使用旧 Revision 的
projection，源码范围、selection 和 visual caret 会失去一致性。

## 决策

`yu-editor::EditorDocument` 拥有一个 `ProjectionCache`。调用 `projection(range)` 时：

- 同一 Revision 和 source range 命中已有 entry；
- 首次查询通过 `yu-projection::Projection::inline` 构建 entry；
- 永久 Transaction 成功后，cache 使用 `Projection::map_through` 处理所有 entries；
- 严格位于 projection range 外的 edit 映射 source ranges 并复用 visual runs；
- 触及 source range 或任一边界的 edit 保守地丢弃 entry；
- `reset_source` 清空所有 entries；composition overlay 的更新不改变 cache，因为它不改变
  canonical source。

ProjectionCache 统计 builds、same-revision hits、remapped entries 和 invalidated entries，供
后续性能基线使用，但统计本身不参与编辑正确性。

## 结果

- projection 的 Revision 生命周期由 canonical EditorDocument 统一管理；
- prefix/suffix 编辑不会强制重新解析不受影响的 inline projection；
- Markdown delimiter 上下文的边界规则采用安全的失效策略，而不是猜测局部语义；
- GUI 尚未引入，cache 可以先通过纯 Rust 行为测试和 benchmark 验证。

## 限制

当前 cache 只缓存完整 source range 的 Projection，不缓存 block parser 或 layout。编辑触及
projection 边界时即使语义上可能不影响 delimiter，也会重新构建；后续接入 block sequence 后
再按 block 状态做更细粒度的失效传播。
