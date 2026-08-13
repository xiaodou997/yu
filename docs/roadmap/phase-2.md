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
- [ ] macOS 文件监听与 debounce，不在后台线程持有可变 `EditorDocument`
- [ ] autosave/recovery 文件格式和崩溃恢复策略
- [ ] 文件权限、软链接、Windows replace semantics 与跨平台原子保存适配
- [ ] 编码/换行策略（当前只接受 UTF-8，不自动规范化 CRLF）

## Track B：进入产品窗口前的共享模型

- [ ] workspace/tab/session 生命周期，不复制 source
- [ ] macOS 文档窗口 host：打开、保存、冲突提示、关闭前 dirty 询问
- [ ] 平台剪贴板格式与 source-backed Markdown/纯文本导出
- [ ] 文件路径、标题、dirty 和 Revision 的 Accessibility/菜单状态投影
- [ ] 以 `DocumentSession` 为输入的 headless vertical slice benchmark

## 约束

1. `yu-storage` 不拥有第二份 Markdown source；读写都以 `EditorDocument::snapshot()` 为准。
2. 外部文件发生变化时保存必须停止并返回冲突，不能静默覆盖用户或其他程序的修改。
3. dirty 不是简单的字符串比较；它绑定当前 Revision 与最近保存 Revision。
4. 平台文件监听只产生“需要检查”的提示，最终指纹比较和 reload/save 决策由 session 完成。

## 下一步

下一阶段建议先实现 macOS 文件监听/关闭前 dirty 流程的无窗口状态机，再接最小 AppKit 文档窗口；窗口
只负责生命周期和状态展示，文本输入仍复用现有 `NSTextInputClient` spike 与 `yu-editor-ffi`。
