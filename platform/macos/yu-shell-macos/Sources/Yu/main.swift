import AppKit
import Darwin
import UniformTypeIdentifiers
import YuStorageFFI

// 可执行入口。Swift 只允许 main.swift 含有顶层可执行语句，因此 self-check
// 的命令行分发也必须留在这里；具体实现见 SelfChecks.swift。
//
// 产品代码按职责分布在：
//   StorageBridge.swift     Rust FFI 封装与跨边界结构镜像
//   DocumentTextView.swift  NSTextInputClient / Accessibility 宿主（不绘制）
//   SurfaceHost.swift       Metal surface 宿主与帧提交调度
//   DocumentWindow.swift    窗口、视图控制器、菜单、文件监视
//   Accessibility.swift     AppKit Accessibility 元素

let app = NSApplication.shared
if let flag = CommandLine.arguments.firstIndex(of: "--selection-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runSelectionSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--undo-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runUndoSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--document-workflow-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runDocumentWorkflowSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--document-interaction-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runDocumentInteractionSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--shaped-projection-hit-test-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runShapedProjectionHitTestSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--shaped-vertical-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runShapedVerticalSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--macos-table-resize-coordinator-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runMacosTableResizeCoordinatorSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--macos-task-checkbox-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runMacosTaskCheckboxSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--clipboard-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runClipboardSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--outline-panel-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runOutlinePanelSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--search-panel-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runSearchPanelSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--multi-cursor-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runMultiCursorSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--code-highlight-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runCodeHighlightSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--accessibility-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runAccessibilitySelfCheck(path: CommandLine.arguments[flag + 1])
}
let delegate = AppDelegate()
app.setActivationPolicy(.regular)
app.delegate = delegate
app.run()
