# ADR 0097：调用方驱动的 autosave/recovery envelope

## 状态

已接受（2026-08-14）

## 背景

`DocumentSession` 已经拥有 canonical source、Revision、saved Revision、BOM 和目标文件冲突
检测，但还没有崩溃恢复边界。把 autosave 定时器、窗口生命周期或恢复决策塞进 Rust session
会引入隐式线程和第二套产品状态，也可能在外部文件发生变化时静默覆盖用户文件。

## 决策

新增 `yu_storage::RecoveryStore`，由产品壳或上层调度器显式调用：

- dirty session 写入独立目录中的 versioned `YURECOV1` 二进制 envelope；clean session 清除
  同一路径的旧记录。
- envelope 保存目标路径、当前 Revision、saved Revision、BOM 和完整 UTF-8 source，并以
  FNV-1a checksum 检测截断/损坏。
- 写入先创建同目录临时文件、`sync_all`、Unix `0600` 权限，再原子 rename；目标 Markdown
  文件永远不会被 recovery 写入改动。
- 读取先限制文件大小，再校验 magic/version/长度/UTF-8/checksum/目标路径；失败返回明确的
  `RecoveryError`，不产生部分恢复结果。
- `RecoveryStore` 不启动 timer、不拥有 session、不自动恢复。读取结果只是候选，产品层必须
  结合当前目标文件状态、用户选择和冲突策略决定是否打开/丢弃。

## 结果

- autosave 频率和生命周期可由 macOS host、未来 workspace 或测试 harness 独立决定。
- canonical source 仍只有 `EditorDocument` 一份；recovery 是明确的磁盘快照，不是编辑状态副本。
- 崩溃恢复格式可以在 GUI 之前进行 round-trip、损坏和权限测试。
- Windows 的 replace semantics、目录权限和跨平台用户数据目录选择仍需平台适配阶段处理。

## 验证

- `recovery_round_trip_preserves_source_revision_and_bom`
- `clean_recovery_clears_stale_record_and_corruption_is_rejected`
- `cargo test --workspace`
