# ADR 0046：EditorHistory 与逆 Transaction 分组

## 状态

已接受（Phase 1）

## 背景

`yu-text::AppliedTransaction` 已经能返回严格绑定结果 Revision 的 inverse Transaction，但
`EditorDocument` 之前只执行并丢弃它。若把每个按键都暴露成一个 Undo 步骤，输入体验会很差；若
保存完整 Snapshot，又会抵消 Piece Tree/Persistent Rope 的共享优势。

## 决策

- `EditorHistory` 只保存 bounded inverse `Transaction`，默认最多保留 512 个 entry；不保存完整
  文档 Snapshot。每个 entry 带有轻量 group id，redo 栈保存 Undo 回放产生的 forward transaction。
- 连续 `InsertText` 归为 `Typing`，连续删除归为 `Deletion`，Enter/任务切换/缩进归为
  `ListEditing`，composition commit 单独归为 `Composition`；移动 caret、设置 selection、开始/
  取消 composition 和 reset source 会断开当前 group。
- Undo/Redo 一次取出一个 group，并按逆序/正序依次回放 entry。由于每个 entry 的 source range
  属于其相邻历史状态，回放时只把 transaction base Revision 重绑定到当前 Revision，不重算或
  复制 source range；回放本身绕过 history recording。
- 回放失败时先用已经成功回放 entry 的 inverse 尝试恢复文档，再恢复原 history group；正常路径下
  每个 transaction 的 base Revision 都由前一步结果保证。
- 新的永久 edit 会清空 redo；没有可用 Undo/Redo 时命令返回 `changed = false`，不推进 Revision。

## 结果

- 连续输入可以一次 Undo/Redo，光标移动后新输入会形成新的历史步骤。
- task toggle、列表续行、缩进/反缩进和 composition commit 都通过同一个逆 Transaction 协议可撤销。
- 历史只保留编辑 range 和插入/删除文本的 Arc，不保留第二份完整 Markdown 文档；容量上限避免长期
  session 无界增长。

## 限制

本阶段不实现持久化 history、跨文件 workspace history、命令宏、选择状态快照或智能合并相邻
非连续编辑。Undo caret 的精细视觉 affinity 仍由后续 editor/UI 层补充；canonical source、
Revision、projection/layout cache 始终是正确性边界。
