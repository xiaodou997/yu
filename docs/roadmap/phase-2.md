# Phase 2：Document Sessions & Product Shell Contracts

## 目标

Phase 1 固定了编辑器内核、Markdown 投影和 macOS 输入/渲染风险边界。Phase 2 从无窗口的文档会话层
开始，把“一个 Markdown 文件如何进入和离开 `EditorDocument`”固定下来，再逐步接入 macOS 文档窗口。

本阶段仍然不承诺完整 CommonMark、最终 UI 或跨平台产品质量；所有平台壳都必须消费同一套
`DocumentSession`、`EditorDocument` 和 Revision-bound 结果。

## Track A：文件与文档会话

- [x] 新建 `yu-storage::DocumentSession`，让 `EditorDocument` 保持 canonical source
- [x] UTF-8 load/save；UTF-8 BOM 作为元数据保留，不进入 source offset/parser range
- [x] Revision-bound dirty 与明确的 saved Revision
- [x] 文件指纹与外部修改/目标删除冲突检测
- [x] 同目录临时文件、写入/sync、原子 rename 保存路径
- [x] clean reload 通过 `EditorDocument::reset_source` 重建 parser/selection/cache
- [x] headless 集成测试覆盖 invalid UTF-8、BOM、composition、save、reload 和冲突
- [x] macOS 文件通知 flag 适配与共享 debounce，不在后台线程持有可变 `EditorDocument`
- [ ] autosave/recovery 文件格式和崩溃恢复策略
- [ ] 文件权限、软链接、Windows replace semantics 与跨平台原子保存适配
- [ ] 编码/换行策略（当前只接受 UTF-8，不自动规范化 CRLF）

## Track B：进入产品窗口前的共享模型

- [ ] workspace/tab/session 生命周期，不复制 source
- [x] 无窗口 close-before-discard 状态机：save、discard、cancel 与 external conflict
- [x] macOS 最小文档窗口 host：打开、源码镜像、标题/dirty 状态、保存/重载和关闭提示
- [x] Rust `DocumentEditorSession`：把 `DocumentSession`、`EditorDocument`、composition 和 close 绑定到一个可变会话
- [x] 统一 session FFI：command、selection、native key route 和 composition 通过同一 handle
- [x] macOS 可编辑文档 host：将 `NSTextInputClient` 的 marked range/source sync 接入统一 session FFI
- [x] macOS 基础纯文本剪贴板：copy/paste/cut/selectAll 全部回到统一 session
- [ ] 跨平台剪贴板格式与 source-backed Markdown/HTML/纯文本导出
- [ ] 文件路径、标题、dirty 和 Revision 的 Accessibility/菜单状态投影
- [ ] 以 `DocumentSession` 为输入的 headless vertical slice benchmark

## 约束

1. `yu-storage` 不拥有第二份 Markdown source；读写都以 `EditorDocument::snapshot()` 为准。
2. 外部文件发生变化时保存必须停止并返回冲突，不能静默覆盖用户或其他程序的修改。
3. dirty 不是简单的字符串比较；它绑定当前 Revision 与最近保存 Revision。
4. 平台文件监听只产生“需要检查”的提示，最终指纹比较和 reload/save 决策由 session 完成。

## 已落地的边界

`yu-storage::FileWatchDebouncer` 只合并通知并返回指纹复核请求；`platform/macos/yu-storage-macos`
只转换 FSEvents/DispatchSource vnode flags，不拥有 native watcher 生命周期。`DocumentSession` 负责
最终 `disk_state`，`CloseStateMachine` 负责 close prompt 状态，二者都不复制 source 或持有 AppKit
对象。

`yu-storage-ffi` 是当前 macOS 产品壳的窄 ABI：Rust `YuStorageSession` 独占可变
`DocumentEditorSession`，Swift 只能取得 owned path/source snapshot、状态和 close/save/reload 结果，
并以 Revision-bound command、selection、source-range copy 与 composition generation 驱动 native
mirror。`experiments/macos-document-host` 的 `DocumentTextView` 现在可以把普通字符、命令、marked
text、commit/cancel 接回同一个 Rust session；TextKit 字符串仍只是可丢弃的投影，不拥有 source、dirty
或 history。它仍不承担 Markdown visual projection 或最终渲染。

## 下一步

下一阶段应补齐可写 host 的手工 macOS 输入源/VoiceOver/剪贴板验收，并把文件 watcher 刷新和
Accessibility/菜单状态投影绑定到同一 native mirror；富文本/Markdown 剪贴板格式仍待设计。在
进入完整 Markdown visual projection 前，继续保持一个 `DocumentEditorSession` handle，不要恢复
storage/editor 两个独立 handle。
