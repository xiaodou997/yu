# ADR 0024：block-local layout snapshot 与 source/visual hit-test

- 状态：Accepted
- 日期：2026-08-10

## 背景

Projection 已经定义 source ranges、hidden syntax、style 和 visual offsets，但还没有稳定的
布局边界。若编辑器直接在窗口或 GPU 层计算换行和 caret，会再次产生一份不可验证的坐标模型，
也无法在未来接入字体 shaping 时保持 source/visual contract。

## 决策

新增 `yu-layout` crate，输入一个 revision-bound `Projection`，输出 block-local
`LayoutSnapshot`：

- `VisualLine` 保存 source/visual range、局部 y、宽度和 cluster index range；
- `VisualCluster` 以 Unicode grapheme 为最小布局单位，并携带 source/visual range、style、x/width；
- `LayoutCaret` 将 source boundary 映射到 visual offset、line 和 point；
- `LayoutHit` 将 point 映射回 source boundary，并保留 `ProjectionBias`；
- 换行和 wrapping 只在 grapheme cluster 边界进行；
- `ClusterMetrics` 只负责提供 advance，默认 `MonospaceMetrics` 用于没有字体 shaping 的阶段；
- `BlockProjection` 可直接进入 layout，fenced code 的 body 仍由 CodeProjection 保持 Code style。

`EditorDocument::block_layout` 暂时返回新建 snapshot，不引入 layout cache、viewport virtualization
或 GPU 状态；这些生命周期问题在 layout contract 通过后单独处理。

## 结果

- layout、projection、editor selection 共用 Revision 和 source/visual mapping；
- macOS hit-test 可以先接入纯 Rust layout contract，再连接 AppKit 坐标；
- 将来字体 shaping 只需实现 `ClusterMetrics`，不改变 line/cluster/caret API；
- 测试可以在没有窗口和 GPU 的环境验证 Unicode、wrap、hidden delimiter 和 hit-test。

## 限制

当前 layout 会为 block-local source range 构造临时字符串以进行 grapheme segmentation；尚未
提供字体 fallback、BiDi、真实 glyph metrics、跨 block viewport virtualization 或 layout cache。
