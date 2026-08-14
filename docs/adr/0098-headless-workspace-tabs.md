# ADR 0098：Headless workspace/tab/session ownership

## 状态

已接受（2026-08-14）

## 背景

`DocumentEditorSession` 已经把文件状态、编辑器、composition 和 close lifecycle 绑定到一个
Rust handle，但产品层还没有定义多个文档如何共存。若 macOS 窗口各自持有 storage/editor
对象，后续 tab 切换、重复打开和 dirty close 很容易重新产生第二份 source 或悬空状态。

## 决策

`yu-workspace::Workspace` 只拥有 `WorkspaceTab` 列表和 active `TabId`；每个 tab 恰好拥有一个
`DocumentEditorSession`：

- `open_path` 对已打开的相同路径复用 tab 并激活它；新路径创建新 session。
- `new_document` 创建未保存 session，source 仍只存在于 session 内。
- `request_close` 对 clean tab 立即移除，对 dirty/conflicted tab 返回 prompt 并保持稳定
  `TabId`；`resolve_close` 只有在 save/discard 成功后才移除 tab，cancel 不改变 tab。
- 外部文件冲突阻止 save，tab 保持可恢复的 prompting 状态；discard 只移除内存 session，
  不覆盖外部文件。
- active tab 被移除后选择同位置的后继 tab，否则选择前一个 tab；空 workspace 的 active
  为 `None`。

workspace 不负责窗口、菜单、计时器或第二份 Markdown source，macOS host 只需把窗口生命周期
映射到这些 headless API。

## 结果

- tab 生命周期可在无窗口测试中固定，GUI 接入不会重新发明 close/dirty 语义。
- source、Revision、composition、viewport 和 recovery 都继续沿单一 session handle 流动。
- 当前路径复用采用 exact `PathBuf` identity；canonicalization、软链接和跨平台路径策略留给
  文件系统适配阶段。

## 验证

- `opening_an_existing_path_reuses_one_session_and_activates_it`
- `clean_close_removes_tab_and_rehomes_active_tab`
- `dirty_close_can_cancel_then_save_without_removing_the_tab_early`
- `external_conflict_blocks_save_but_discard_closes_without_overwrite`
