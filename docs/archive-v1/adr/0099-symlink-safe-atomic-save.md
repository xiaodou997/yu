# ADR 0099：软链接安全的原子保存

## 状态

已接受（Phase 2，macOS/Unix）。Windows replace semantics 仍未定义。

## 背景

`DocumentSession` 原先把用户传入的路径同时用于读取、指纹和同目录原子替换。
如果这个路径是软链接，直接对它执行 `rename(temp, link)` 会替换软链接本身，
而不是更新用户真正编辑的目标文件。这会破坏路径语义，也可能让后续程序看到一个
普通文件而不是原来的链接。软链接被重新指向另一个目标时，旧的文件指纹也不能继续
被当作当前目标的身份。

## 决策

- `DocumentSession::path()` 保留用户打开时的路径，用于标题、错误和恢复记录。
- `DocumentSession::storage_path()` 保存 `fs::canonicalize(path)` 的结果；打开现有文件时
  该 canonical target 是读取、指纹、重载和 atomic replace 的唯一存储路径。
- `disk_state()` 每次先 canonicalize 用户路径。路径消失返回 `Missing`；canonical target
  与会话记录不一致（例如软链接被重新指向）返回 `Changed`，保存不得覆盖新目标。
- 原子保存仍在 canonical target 所在目录创建临时文件、写入并 `sync_all`，然后 rename
  到 target；因此软链接 inode 保持不变。
- `atomic_replace` 在替换前读取 target 权限并复制到临时文件，避免正常保存把 Unix/macOS
  文件 mode 重置为临时文件的默认 mode。
- `DocumentSession::new` 的路径尚不存在时暂不解析 canonical target；首次保存仍遵循“路径
  出现即外部冲突”的既有安全策略。

## 验证

- Unix/macOS 集成测试打开软链接、编辑并保存，确认链接仍是链接、目标内容更新、目标 mode
  保持不变。
- 集成测试将软链接重定向到另一个文件，确认 `disk_state()` 为 `Changed` 且 save 不修改
  两个目标。
- 保持 `cargo test --workspace` 与 `cargo clippy --workspace --all-targets -- -D warnings`
  作为提交门禁。

## 后续

Windows 的 reparse point、ACL、`ReplaceFileW`/rename 行为需要单独的原生实验和测试，不能把
Unix `canonicalize`/mode 规则直接假设为跨平台语义。
