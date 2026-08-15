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
- [x] autosave/recovery 文件格式和崩溃恢复策略（调用方驱动的 `RecoveryStore` envelope）
- [x] macOS/Unix 软链接保存跟随 canonical target，并保留目标文件权限
- [ ] Windows replace semantics 与其余跨平台原子保存适配
- [ ] 编码/换行策略（当前只接受 UTF-8，不自动规范化 CRLF）

## Track B：进入产品窗口前的共享模型

- [x] workspace/tab/session 生命周期，不复制 source（headless `Workspace`/`WorkspaceTab`）
- [x] 无窗口 close-before-discard 状态机：save、discard、cancel 与 external conflict
- [x] macOS 最小文档窗口 host：打开、源码镜像、标题/dirty 状态、保存/重载和关闭提示
- [x] Rust `DocumentEditorSession`：把 `DocumentSession`、`EditorDocument`、composition 和 close 绑定到一个可变会话
- [x] 统一 session FFI：command、selection、native key route 和 composition 通过同一 handle
- [x] macOS 可编辑文档 host：将 `NSTextInputClient` 的 marked range/source sync 接入统一 session FFI
- [x] macOS 基础纯文本剪贴板：copy/paste/cut/selectAll 全部回到统一 session
- [x] macOS source-backed Markdown/纯文本剪贴板：copy/cut 发布 Markdown UTI，paste 优先保留源码
- [x] `yu-export` Revision-bound source selection：Markdown/纯文本回退与保守语义 HTML payload
- [x] macOS copy/cut 同时发布 canonical Markdown、纯文本和 HTML pasteboard 类型
- [x] macOS 文档目录 vnode watcher：事件合并后由 Rust session 复核磁盘指纹；clean 文档可重载，dirty 文档只提示冲突
- [x] Rust 跨平台剪贴板格式契约：Markdown/纯文本/HTML 的 MIME、macOS UTI 和 payload 映射
- [x] 保守 GFM table range parser 与 semantic HTML `<table>` 导出
- [x] 受控 HTML fragment→Markdown policy：allowlist、危险 URL/属性拒绝、Markdown fallback 和 headless round trip
- [x] macOS HTML fallback native adapter：Markdown > 纯文本 > 受控 HTML、拒绝回退与无窗口 self-check
- [x] macOS 跨应用 HTML fixture corpus：semantic mail、GFM table、browser wrapper 与 unsafe HTML
- [ ] Windows/Linux native clipboard adapter 与跨应用粘贴回归
- [x] macOS 基础文件路径/标题、dirty、Revision、磁盘状态的状态栏、菜单和 TextKit Accessibility 投影
- [x] macOS source-backed Accessibility 快照 FFI：UTF-16 字符数、选区、逻辑行范围和位置查询均绑定 Revision
- [x] macOS Accessibility 回调在 close/reload/外部替换边界上失败可恢复，不因快照失效触发宿主崩溃
- [x] Revision-bound source-backed Markdown semantic Accessibility node count/fill 与稳定 C ABI
- [x] macOS host 将 semantic nodes 映射为 Revision-bound AppKit Accessibility children，并提供无窗口 self-check
- [x] macOS Heading/Link custom rotor、旧 child `uiElementDestroyed` 通知与 task checkbox value self-check
- [x] semantic link destination URL 与 task checkbox press 的 Revision-bound action contract/self-check
- [x] macOS 真实 VoiceOver 朗读验收
- [ ] Rotor/语义 action 的真实导航回归与跨平台 Accessibility 适配
- [x] 以 `DocumentEditorSession` 为输入的 headless vertical slice benchmark，并记录局部/传播编辑基线
- [x] viewport block sync 使用 key 索引，避免大文档编辑后的 O(blocks²) cache remap
- [x] 未发生 viewport 查询时保持 block entries 惰性，避免纯编辑路径物化全文索引
- [x] 已 materialize viewport 时优先用有序 merge 保留 block entry，异常顺序再回退 key 索引

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
或 history。目录级 DispatchSource watcher 只触发带 debounce 的状态复核，原子替换也由目录监听覆盖；
Rust `disk_state` 仍是 reload/save/close 的唯一权威。Accessibility 查询另通过
`YuStorageAccessibilitySnapshot`、Revision-bound line/range ABI 和 semantic node count/fill ABI
读取 canonical source；每个 semantic node 只有 role、父子关系和 source/label UTF-16 ranges，Swift
以及可选 destination/action block metadata；Swift 将其映射为由 `DocumentTextView` 持有、实现
`NSAccessibilityElementProtocol` 的 children，几何按
当前 TextKit 布局计算但仍绑定节点 Revision。Heading/Link custom rotor 只查询这棵当前 child tree，
旧 child 在 refresh 前收到 `uiElementDestroyed`。无窗口 `--accessibility-self-check` 会验证父子关系、
label、角色、task 状态/press、URL 属性、Rotor 返回目标和编辑后的 stale node；macOS 真实 VoiceOver
朗读已由用户人工确认，Rotor/语义 action 的跨平台回归仍待后续完成。
它仍不承担 Markdown visual projection 或最终渲染。

## 下一步

下一阶段应完成 macOS 真实跨应用 HTML paste 回归，并固定 URL 打开策略、图片/表格 action 和
Windows/Linux clipboard/action adapter 的产品边界；Rotor/语义 action 的真实导航回归仍需补一轮。
完整 Markdown visual projection 仍放在这些边界稳定之后。在
进入完整 Markdown visual projection 前，继续保持一个 `DocumentEditorSession` handle，不要恢复
storage/editor 两个独立 handle。
