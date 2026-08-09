# 坐标与位置

Yu 不使用同一个 `usize` 表示所有位置。

| 类型 | 用途 | 是否可跨 Revision 保存 |
| --- | --- | --- |
| `ByteOffset` | UTF-8 存储、parser、source range | 否 |
| `Utf16Offset` | AppKit/TSF 等原生桥接 | 否 |
| `GraphemeOffset` | 用户感知的字符移动与删除 | 否 |
| `TextAnchor` | selection、异步结果、批注 | 是，需要映射 |
| `VisualPosition` | 投影后的逻辑位置 | 否 |
| `Point` | 布局坐标 | 否 |

## Anchor affinity

Anchor 位于插入点时必须说明黏附方向：

```text
文本: ab|cd
插入: XY

Before affinity -> ab|XYcd
After affinity  -> abXY|cd
```

Replacement 内部的 Anchor 会折叠到 replacement 的左边或右边，具体由 affinity 决定。

## Snapshot boundary

所有裸 offset 都隐含所属 Revision。跨线程或跨异步边界传递 offset 时，必须同时携带 Revision，
或者先转换为可映射 Anchor。

