# ADR 0171：Composition 活跃期间的永久命令边界

## 状态

已接受（2026-08-18）

## 背景

macOS `NSTextInputClient` 的 marked text 是覆盖在 canonical Markdown source 之上的
transient `CompositionOverlay`。菜单/FFI 已经在 overlay 活跃时把普通编辑命令标记为不可用，
但如果其它调用方直接调用 `EditorDocument::execute`，仍可能绕过这层提示，在 IME preedit 尚未
提交时创建永久 Transaction。这样会使固定的 composition replacement range、selection、Revision
和 Undo history 失去一致性。

## 决策

1. `EditorDocument::execute` 在清理上一条 `SourceChange` 后，首先拒绝所有普通命令并返回
   `EditorDocumentError::CompositionActive`；overlay 必须先 commit 或 cancel。
2. `route_key`、`yu-editor-ffi` 和 `yu-storage-ffi` 继续保留各自的 availability/status 映射，
   但它们不再是唯一防线。
3. `apply_transaction` 仍允许外部文件/会话层模拟并检测 composition 期间的 revision 变化；这
   是为了让后续 commit 得到 stale-revision 错误，而不是静默覆盖外部编辑，不等同于普通用户命令。
4. preedit/update 的 UTF-16 selection 必须落在 scalar 边界；失败更新不替换已有 overlay。

## 结果

- canonical source、Revision、selection 和 history 不会被 composition 活跃期间的普通命令污染。
- ZWJ 家庭 emoji、组合重音、反向选区、连续删除 history group、composition commit/undo/redo
  和多 edit transaction 都有独立行为回归。
- native host 仍只需要执行 commit/cancel，再回到统一 command path；不增加第二套文本模型。

## 验证

- `cargo test -p yu-editor --test editor_behavior`
- `cargo test -p yu-text --test transaction_model`
- workspace 全量 test、clippy、format 和 `git diff --check`
