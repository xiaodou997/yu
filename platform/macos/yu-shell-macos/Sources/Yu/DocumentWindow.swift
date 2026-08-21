import AppKit
import Darwin
import Foundation
import UniformTypeIdentifiers
import YuStorageFFI

// 窗口、视图控制器、菜单与文件监视——纯粹的平台外壳职责。

/// Watches the containing directory rather than the file inode. Rust still
/// owns the file fingerprint and all reload/conflict decisions; this object
/// only turns native vnode notifications into a main-thread callback. Watching
/// the directory keeps atomic-save rename replacement observable.
final class NativeFileWatcher {
    private let descriptor: Int32
    private let source: DispatchSourceFileSystemObject

    init(directory: URL, handler: @escaping () -> Void) throws {
        let descriptor = open(directory.path, O_EVTONLY)
        guard descriptor >= 0 else { throw BridgeError.watcher(errno) }
        self.descriptor = descriptor
        source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: descriptor,
            eventMask: [.write, .extend, .attrib, .delete, .rename],
            queue: .main
        )
        source.setEventHandler(handler: handler)
        source.setCancelHandler { close(descriptor) }
        source.resume()
    }

    deinit {
        source.cancel()
    }
}
final class DocumentViewController: NSViewController, NSMenuItemValidation {
    private let bridge: StorageBridge
    private lazy var textView = DocumentTextView(bridge: bridge)
    private let surfaceHostView = MacosSurfaceHostView()
    private let surfaceCoordinator: MacosSurfaceHostCoordinator
    private let statusLabel = NSTextField(labelWithString: "")
    private var saveButton: NSButton?
    private var reloadButton: NSButton?
    private var initialState: NativeStorageState
    private var fileWatcher: NativeFileWatcher?
    private var externalCheckWorkItem: DispatchWorkItem?
    private var promptedExternalDisk: DiskState?
    private var surfaceBoundsObserver: NSObjectProtocol?
    private weak var documentScrollView: NSScrollView?
    private var visualPointerAdapterEnabled = false
    private var visualPointerLayoutWidth: CGFloat = -1.0
    /// The source TextKit mirror must have one complete layout pass before
    /// optional Rust projection/surface work is allowed to run. This keeps
    /// opening a document on the primary editing path independent from the
    /// enhancement layer's first drawable/metrics publication.
    private var visualEnhancementsReady = false
    /// True only when the current decoration sibling came from the same Rust
    /// shaped frame that may become the primary visual surface. TextKit
    /// projected decorations are a fallback overlay and must never hide the
    /// source mirror or leave a stale Metal frame visible underneath it.

    init(bridge: StorageBridge) {
        self.bridge = bridge
        self.surfaceCoordinator = MacosSurfaceHostCoordinator(bridge: bridge)
        self.initialState = bridge.state
        super.init(nibName: nil, bundle: nil)
        surfaceCoordinator.onSurfaceStateChange = { [weak self] in
            self?.textView.refreshTableResizeAccessibility()
            self?.syncSourceGlyphVisibility()
        }
        surfaceCoordinator.onError = { [weak self] error in
            // The source TextKit mirror remains usable when a machine has no
            // Metal drawable; surface lifecycle failure is diagnostic, not a
            // reason to interrupt editing with a modal alert.
            self?.statusLabel.toolTip = "Native surface inactive: \(error.localizedDescription)"
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

    deinit {
        if let surfaceBoundsObserver {
            NotificationCenter.default.removeObserver(surfaceBoundsObserver)
        }
        surfaceCoordinator.detach()
    }

    override func loadView() {
        let root = NSView()
        let scrollView = NSScrollView()
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = false
        scrollView.autohidesScrollers = true
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        surfaceHostView.translatesAutoresizingMaskIntoConstraints = true
        surfaceHostView.autoresizingMask = []
        surfaceHostView.setAccessibilityElement(false)
        documentScrollView = scrollView

        textView.isEditable = true
        textView.isSelectable = true
        textView.usesFindBar = true
        textView.onDocumentChange = { [weak self] in
            guard let self else { return }
            self.initialState = self.bridge.state
            self.surfaceCoordinator.resetTableResizeAfterDocumentChange()
            self.textView.refreshTableResizeAccessibility()
            self.updateStatus()
            self.syncSourceGlyphVisibility()
            self.scheduleVisualSubmit()
        }
        textView.onCaretChange = { [weak self] in
            guard let self else { return }
            // 光标移动不推进 Revision，但会改变 caret 与选区装饰。Rust 的帧
            // 身份已经把 selection 算在内，平台不需要再显式作废任何东西。
            self.scheduleVisualSubmit()
            // AppKit may deliver selection changes while TextKit is still
            // inside its event callback. Defer the scroll mutation until the
            // same main-thread turn has finished, while retaining the Rust
            // Revision captured by the coordinator's query.
            DispatchQueue.main.async { [weak self] in
                guard let self, self.visualEnhancementsReady else { return }
                self.surfaceCoordinator.revealCaretIfNeeded()
            }
        }
        textView.onError = { [weak self] error in self?.show(error) }
        textView.onTableResizeHover = { [weak self] point in
            self?.surfaceCoordinator.tableResizeHover(at: point) ?? false
        }
        textView.onTaskCheckboxPress = { [weak self] point in
            guard let self,
                  let hit = self.surfaceCoordinator.taskCheckboxHit(at: point) else {
                return false
            }
            return self.textView.toggleTaskPointerHit(hit)
        }
        textView.onTableResizeBegin = { [weak self] point in
            self?.surfaceCoordinator.beginTableResize(at: point) ?? false
        }
        textView.onTableResizeUpdate = { [weak self] point in
            self?.surfaceCoordinator.updateTableResize(at: point) ?? false
        }
        textView.onTableResizeFinish = { [weak self] in
            self?.surfaceCoordinator.finishTableResize() ?? false
        }
        textView.onTableResizeCancel = { [weak self] in
            self?.surfaceCoordinator.cancelTableResize() ?? false
        }
        textView.tableResizeAccessibilityProvider = { [weak self] in
            self?.surfaceCoordinator.tableResizeAccessibilityDividers() ?? []
        }
        textView.tableResizeAccessibilityFrameProvider = { [weak self] descriptor in
            self?.surfaceCoordinator.tableResizeAccessibilityFrame(for: descriptor)
                ?? .zero
        }
        textView.onTableResizeAccessibilityAction = { [weak self] descriptor, direction in
            guard let self else { return false }
            return self.surfaceCoordinator.adjustTableResizeAccessibility(
                descriptor,
                direction: direction
            )
        }
        scrollView.documentView = textView
        // `DocumentTextView` is created before the window has a laid-out
        // content size. Give the scroll view a real initial document frame
        // and let its text container track the viewport width; otherwise an
        // NSTextView created with the designated initializer can retain a
        // zero-sized document view while AX still exposes its source value.
        textView.frame = NSRect(x: 0, y: 0, width: 900, height: 620)
        textView.autoresizingMask = [.width]
        textView.textContainer?.containerSize = NSSize(
            width: scrollView.contentSize.width,
            height: CGFloat.greatestFiniteMagnitude
        )
        textView.textContainer?.widthTracksTextView = true

        surfaceCoordinator.bind(
            surfaceView: surfaceHostView,
            scrollView: scrollView,
            fontSize: textView.font?.pointSize ?? 16.0
        )
        textView.refreshTableResizeAccessibility()
        surfaceHostView.onWindowStateChange = { [weak self] attached in
            guard let self else { return }
            if attached {
                if self.visualEnhancementsReady {
                    self.surfaceCoordinator.scheduleSubmit()
                    self.syncSourceGlyphVisibility()
                } else {
                }
                self.textView.refreshTableResizeAccessibility()
            } else {
                self.surfaceCoordinator.detach()
                self.textView.refreshTableResizeAccessibility()
            }
        }
        surfaceHostView.onGeometryChange = { [weak self] in
            self?.scheduleVisualSubmit()
            self?.syncSourceGlyphVisibility()
            self?.textView.refreshTableResizeAccessibility()
        }
        scrollView.contentView.postsBoundsChangedNotifications = true
        surfaceBoundsObserver = NotificationCenter.default.addObserver(
            forName: NSView.boundsDidChangeNotification,
            object: scrollView.contentView,
            queue: .main
        ) { [weak self] _ in
            self?.scheduleVisualSubmit()
            self?.syncSourceGlyphVisibility()
            self?.textView.refreshTableResizeAccessibility()
        }

        statusLabel.setAccessibilityElement(true)
        statusLabel.setAccessibilityLabel("文档状态")

        let toolbar = NSStackView()
        toolbar.orientation = .horizontal
        toolbar.alignment = .centerY
        toolbar.spacing = 10
        toolbar.translatesAutoresizingMaskIntoConstraints = false

        let saveButton = NSButton(title: "保存", target: self, action: #selector(save))
        let reloadButton = NSButton(title: "重新加载", target: self, action: #selector(reload))
        self.saveButton = saveButton
        self.reloadButton = reloadButton
        toolbar.addArrangedSubview(saveButton)
        toolbar.addArrangedSubview(reloadButton)
        toolbar.addArrangedSubview(statusLabel)

        root.addSubview(toolbar)
        root.addSubview(scrollView)
        // The Rust surface is a visual projection above the TextKit mirror.
        // Its hitTest returns nil, so keyboard, IME, selection and scrolling
        // remain owned by the source view underneath it. The frame is synced
        // to the clip viewport in viewDidLayout, excluding native scrollers.
        root.addSubview(surfaceHostView, positioned: .above, relativeTo: scrollView)
        NSLayoutConstraint.activate([
            toolbar.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 16),
            toolbar.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -16),
            toolbar.topAnchor.constraint(equalTo: root.topAnchor, constant: 12),
            scrollView.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            scrollView.topAnchor.constraint(equalTo: toolbar.bottomAnchor, constant: 10),
            scrollView.bottomAnchor.constraint(equalTo: root.bottomAnchor),
        ])
        view = root
        startFileWatcher()
        updateStatus()
    }

    override func viewDidLayout() {
        super.viewDidLayout()
        guard let scrollView = documentScrollView else { return }
        let viewportFrame = view.convert(scrollView.contentView.frame, from: scrollView)
        if surfaceHostView.frame != viewportFrame {
            surfaceHostView.frame = viewportFrame
        }
        guard visualEnhancementsReady else {
            // Keep the native source mirror fully visible during the first
            // layout. The enhancement layer is enabled from viewDidAppear,
            // after AppKit has a real window/clip geometry to report.
            return
        }
        let visualWidth = max(
            textView.bounds.width - 2.0 * textView.textContainerOrigin.x,
            1.0
        )
        surfaceCoordinator.setContentWidth(visualWidth)
        textView.refreshTableResizeAccessibility()
        // 指针命中测试直接走 Rust layout，不需要预先建立任何 TextKit 镜像，
        // 因而也没有「适配器未就绪」这个状态。
        syncSourceGlyphVisibility()
        textView.refreshTableResizeAccessibility()
        surfaceCoordinator.scheduleSubmit()
        surfaceCoordinator.revealCaretIfNeeded()
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        guard !visualEnhancementsReady else { return }
        visualEnhancementsReady = true
        // Defer the first optional projection submit by one main-thread turn
        // so source TextKit focus/IME setup has completed before any native
        // surface callback can run.
        DispatchQueue.main.async { [weak self] in
            guard let self, self.view.window != nil else { return }
            self.view.needsLayout = true
            self.scheduleVisualSubmit()
        }
    }

    func refreshFromRust() {
        textView.refreshFromRust()
        surfaceCoordinator.resetTableResizeAfterDocumentChange()
        textView.refreshTableResizeAccessibility()
        initialState = bridge.state
        if initialState.disk == .unchanged {
            promptedExternalDisk = nil
        }
        updateStatus()
        syncSourceGlyphVisibility()
        textView.refreshTableResizeAccessibility()
        scheduleVisualSubmit()
        if visualEnhancementsReady {
            surfaceCoordinator.revealCaretIfNeeded()
        }
    }

    func detachSurfaceHost() {
        surfaceCoordinator.detach()
    }


    /// Rust surface 是唯一渲染路径（不变量 I5）。TextKit 永不绘制像素，
    /// 因此这里没有 gate、没有 fallback reason、没有 coverage 判断：
    /// surface 一旦 attach 就保持可见，未支持的语法由 Rust 按源码文本绘制。
    private func syncSourceGlyphVisibility() {
        surfaceHostView.setNativeContentVisible(true)
    }



    private func scheduleVisualSubmit() {
        guard visualEnhancementsReady else { return }
        surfaceCoordinator.scheduleSubmit()
    }


    @objc private func save() {
        do {
            try bridge.save()
            refreshFromRust()
        } catch { show(error) }
    }

    @objc private func reload() {
        do {
            try bridge.reload()
            refreshFromRust()
        } catch { show(error) }
    }

    @objc fileprivate func saveFromMenu(_ sender: Any?) {
        save()
    }

    @objc fileprivate func reloadFromMenu(_ sender: Any?) {
        reload()
    }

    @objc fileprivate func closeFromMenu(_ sender: Any?) {
        view.window?.performClose(sender)
    }

    @objc fileprivate func copyFromMenu(_ sender: Any?) {
        textView.copy(sender)
    }

    @objc fileprivate func cutFromMenu(_ sender: Any?) {
        textView.cut(sender)
    }

    @objc fileprivate func undoFromMenu(_ sender: Any?) {
        textView.performUndo()
    }

    @objc fileprivate func redoFromMenu(_ sender: Any?) {
        textView.performRedo()
    }

    @objc fileprivate func pasteFromMenu(_ sender: Any?) {
        textView.paste(sender)
    }

    @objc fileprivate func selectAllFromMenu(_ sender: Any?) {
        textView.selectAll(sender)
    }

    func focusDocument() {
        _ = view.window?.makeFirstResponder(textView)
    }

    /// 真实窗口下的帧调度自检。
    ///
    /// 「这一帧是否等价于屏幕上那一帧」的判断已经移入 Rust。判断漏掉一项不会
    /// 报错，只会让画面停住——光标不动、preedit 不更新、拖动中的列宽不动，
    /// 三者都表现为「编辑器卡了」而没有任何日志。headless self-check 覆盖不到
    /// 这条路径：它需要真实的 NSWindow 与 Metal surface 才会有「已提交的帧」。
    ///
    /// 反向验证：把 `MacosFrameKey` 的 `selection` 去掉，第 3 步失败。
    func runFrameSchedulingSelfCheck() throws {
        struct Failure: LocalizedError {
            let message: String
            var errorDescription: String? { message }
        }
        func require(_ condition: Bool, _ message: String) throws {
            guard condition else { throw Failure(message: message) }
        }

        // 1. 真实 surface 上必须先有一帧。
        let snapshot = try surfaceCoordinator.submitNow(force: true)
        try require(snapshot?.submitted == true, "首帧未提交")
        try require((snapshot?.commandCount ?? 0) > 0, "首帧没有任何绘制指令")

        // 2. 状态没变时必须判为等价，否则每一次布局回调都会整帧重画。
        try require(surfaceCoordinator.hasCurrentFrame(), "刚提交的帧未被判为当前帧")

        // 3. 光标移动不推进 Revision，但必须让帧失效。
        let sourceLength = bridge.source.utf16.count
        try require(sourceLength > 4, "fixture 太短，无法移动光标")
        let before = bridge.selection
        try bridge.setSelection(NSRange(location: 3, length: 0))
        try require(bridge.state.revision == before.revision, "移动光标不应推进 Revision")
        try require(
            !surfaceCoordinator.hasCurrentFrame(),
            "光标移动后仍被判为当前帧——caret 会停在原处且不会报错"
        )

        // 4. 重新提交之后必须再次等价。
        let republished = try surfaceCoordinator.submitNow()
        try require(republished?.submitted == true, "光标移动后的重提交失败")
        try require(surfaceCoordinator.hasCurrentFrame(), "重提交后未恢复为当前帧")

        print(
            "Yu frame scheduling self-check: commands=\(snapshot?.commandCount ?? 0) "
                + "caret=\(republished?.caretDecorationCount ?? 0) "
                + "selection=\(republished?.selectionDecorationCount ?? 0)"
        )
    }

    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        let state = bridge.state
        if menuItem.action == #selector(saveFromMenu(_:)) {
            return state.dirty
        }
        if menuItem.action == #selector(reloadFromMenu(_:)) {
            return !state.dirty && state.disk != .unchanged
        }
        if menuItem.action == #selector(undoFromMenu(_:)) {
            return textView.canUndo()
        }
        if menuItem.action == #selector(redoFromMenu(_:)) {
            return textView.canRedo()
        }
        if menuItem.action == #selector(copyFromMenu(_:)) ||
            menuItem.action == #selector(cutFromMenu(_:)) {
            return textView.selectedRange().length > 0
        }
        if menuItem.action == #selector(pasteFromMenu(_:)) {
            return textView.hasSourceOnPasteboard
        }
        if menuItem.action == #selector(selectAllFromMenu(_:)) {
            return textView.string.utf16.count > 0
        }
        return true
    }

    func requestClose() -> Bool {
        do {
            let request = try bridge.requestClose()
            switch request.result {
            case 0:
                return true
            case 1:
                return prompt(request)
            default:
                return false
            }
        } catch {
            show(error)
            return false
        }
    }

    private func prompt(_ request: YuStorageCloseRequest) -> Bool {
        let alert = NSAlert()
        alert.alertStyle = request.close_state >= 3 ? .warning : .informational
        alert.messageText = request.close_state >= 3 ? "文件已被外部修改" : "保存更改？"
        alert.informativeText = request.close_state >= 3
            ? "保存会覆盖外部版本；请选择丢弃本地修改或取消。"
            : "这个 Markdown 文档有未保存更改。"
        alert.addButton(withTitle: request.close_state >= 3 ? "丢弃本地修改" : "保存")
        alert.addButton(withTitle: "取消")
        let response = alert.runModal()
        do {
            if response == .alertFirstButtonReturn {
                if request.close_state >= 3 {
                    try bridge.discardAndClose()
                } else {
                    try bridge.saveAndClose()
                }
                return true
            }
            try bridge.cancelClose()
            return false
        } catch {
            show(error)
            return false
        }
    }

    private func startFileWatcher() {
        guard fileWatcher == nil else { return }
        let directory = URL(fileURLWithPath: bridge.path).deletingLastPathComponent()
        do {
            fileWatcher = try NativeFileWatcher(directory: directory) { [weak self] in
                self?.scheduleExternalStateCheck()
            }
        } catch {
            // The session remains correct without a native notification source;
            // the status/menu state can still be refreshed by explicit actions.
            statusLabel.toolTip = error.localizedDescription
        }
    }

    private func scheduleExternalStateCheck() {
        externalCheckWorkItem?.cancel()
        let workItem = DispatchWorkItem { [weak self] in
            self?.checkExternalState()
        }
        externalCheckWorkItem = workItem
        DispatchQueue.main.asyncAfter(
            deadline: .now() + .milliseconds(150),
            execute: workItem
        )
    }

    private func checkExternalState() {
        let state = bridge.state
        initialState = state
        updateStatus()
        guard state.disk != .unchanged else {
            promptedExternalDisk = nil
            return
        }
        guard promptedExternalDisk != state.disk else { return }
        promptedExternalDisk = state.disk

        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = state.disk == .missing ? "文件已被删除或移动" : "文件已被外部修改"
        if state.dirty {
            alert.informativeText =
                "Rust session 已确认磁盘版本变化。本地有未保存修改，Yu 不会自动覆盖或重载。请保存或关闭窗口处理冲突。"
            alert.addButton(withTitle: "知道了")
        } else {
            alert.informativeText =
                "当前没有本地未保存修改，可以重新加载磁盘上的版本。"
            alert.addButton(withTitle: "重新加载")
            alert.addButton(withTitle: "稍后")
        }
        let response = alert.runModal()
        guard !state.dirty, response == .alertFirstButtonReturn else { return }
        do {
            try bridge.reload()
            refreshFromRust()
        } catch {
            show(error)
        }
    }

    private func updateStatus() {
        let state = bridge.state
        let dirty = state.dirty ? "● 未保存" : "已保存"
        let bom = state.bom ? "UTF-8 BOM" : "UTF-8"
        let status = "\(dirty) · Rev \(state.revision) · \(state.disk.label) · \(bom)"
        statusLabel.stringValue = status
        statusLabel.setAccessibilityValue(status)
        saveButton?.isEnabled = state.dirty
        reloadButton?.isEnabled = !state.dirty && state.disk != .unchanged
    }

    private func show(_ error: Error) {
        let alert = NSAlert(error: error)
        alert.runModal()
    }
}
final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    private var window: NSWindow?
    private var controller: DocumentViewController?
    private var launchSelfCheck = false

    func applicationDidFinishLaunching(_ notification: Notification) {
        let path: String
        launchSelfCheck = CommandLine.arguments.contains("--launch-window-self-check")
        if let argument = CommandLine.arguments.dropFirst().first(where: { !$0.hasPrefix("-") }) {
            path = URL(fileURLWithPath: argument).path
        } else {
            let panel = NSOpenPanel()
            // `.md` is not consistently classified as `UTType.text` by
            // Finder (especially for files created by scripts). Include the
            // Markdown declaration explicitly while retaining ordinary text
            // files in the first-stage host.
            var contentTypes: [UTType] = [.text, .plainText]
            if let markdown = UTType(filenameExtension: "md") {
                contentTypes.append(markdown)
            }
            panel.allowedContentTypes = contentTypes
            panel.canChooseDirectories = false
            guard panel.runModal() == .OK, let url = panel.url else {
                NSApp.terminate(nil)
                return
            }
            path = url.path
        }

        do {
            let bridge = try StorageBridge(path: path)
            let controller = DocumentViewController(bridge: bridge)
            let window = NSWindow(
                contentViewController: controller
            )
            window.setContentSize(NSSize(width: 900, height: 620))
            window.styleMask = [.titled, .closable, .miniaturizable, .resizable]
            window.title = URL(fileURLWithPath: bridge.path).lastPathComponent
            window.center()
            window.delegate = self
            window.isReleasedWhenClosed = false
            window.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
            self.controller = controller
            self.window = window
            installMainMenu(for: controller)
            controller.focusDocument()
            print("Yu document host opened path=\(bridge.path) revision=\(bridge.state.revision)")
            if launchSelfCheck {
                // Give AppKit one complete appearance/layout turn. This is a
                // real window smoke test: source fallback must become visible
                // before optional Rust surface work is allowed to run.
                DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(500)) {
                    guard window.isVisible else {
                        fputs("Yu launch self-check failed: window is not visible\n", stderr)
                        exit(EXIT_FAILURE)
                    }
                    print("Yu launch self-check: window appeared and remained stable")
                    do {
                        try controller.runFrameSchedulingSelfCheck()
                    } catch {
                        fputs("Yu frame scheduling self-check failed: \(error)\n", stderr)
                        exit(EXIT_FAILURE)
                    }
                    NSApp.terminate(nil)
                }
            }
        } catch {
            if launchSelfCheck {
                fputs("Yu launch self-check failed: \(error)\n", stderr)
                exit(EXIT_FAILURE)
            }
            let alert = NSAlert(error: error)
            alert.runModal()
            NSApp.terminate(nil)
        }
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        let shouldClose = controller?.requestClose() ?? true
        if shouldClose {
            controller?.detachSurfaceHost()
        }
        return shouldClose
    }

    func windowWillClose(_ notification: Notification) {
        controller?.detachSurfaceHost()
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard let controller else { return .terminateNow }
        guard controller.requestClose() else { return .terminateCancel }
        controller.detachSurfaceHost()
        return .terminateNow
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }

    private func installMainMenu(for controller: DocumentViewController) {
        let mainMenu = NSMenu()

        let appMenuItem = NSMenuItem()
        let appMenu = NSMenu(title: "Yu")
        appMenu.addItem(
            withTitle: "关于 Yu",
            action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)),
            keyEquivalent: ""
        )
        appMenu.addItem(.separator())
        let quit = NSMenuItem(
            title: "退出 Yu",
            action: #selector(NSApplication.terminate(_:)),
            keyEquivalent: "q"
        )
        quit.target = NSApp
        appMenu.addItem(quit)
        appMenuItem.submenu = appMenu
        mainMenu.addItem(appMenuItem)

        let fileMenuItem = NSMenuItem()
        let fileMenu = NSMenu(title: "文件")
        let save = NSMenuItem(
            title: "保存",
            action: #selector(DocumentViewController.saveFromMenu(_:)),
            keyEquivalent: "s"
        )
        save.target = controller
        fileMenu.addItem(save)
        let reload = NSMenuItem(
            title: "重新加载",
            action: #selector(DocumentViewController.reloadFromMenu(_:)),
            keyEquivalent: "r"
        )
        reload.target = controller
        fileMenu.addItem(reload)
        fileMenu.addItem(.separator())
        let close = NSMenuItem(
            title: "关闭窗口",
            action: #selector(DocumentViewController.closeFromMenu(_:)),
            keyEquivalent: "w"
        )
        close.target = controller
        fileMenu.addItem(close)
        fileMenuItem.submenu = fileMenu
        mainMenu.addItem(fileMenuItem)

        let editMenuItem = NSMenuItem()
        let editMenu = NSMenu(title: "编辑")
        let editItems: [(String, Selector, String)] = [
            ("撤销", #selector(DocumentViewController.undoFromMenu(_:)), "z"),
            ("重做", #selector(DocumentViewController.redoFromMenu(_:)), "Z"),
            ("剪切", #selector(DocumentViewController.cutFromMenu(_:)), "x"),
            ("复制", #selector(DocumentViewController.copyFromMenu(_:)), "c"),
            ("粘贴", #selector(DocumentViewController.pasteFromMenu(_:)), "v"),
            ("全选", #selector(DocumentViewController.selectAllFromMenu(_:)), "a"),
        ]
        for (title, action, keyEquivalent) in editItems.prefix(2) {
            let item = NSMenuItem(title: title, action: action, keyEquivalent: keyEquivalent)
            item.target = controller
            editMenu.addItem(item)
        }
        editMenu.addItem(.separator())
        for (title, action, keyEquivalent) in editItems.dropFirst(2) {
            let item = NSMenuItem(title: title, action: action, keyEquivalent: keyEquivalent)
            item.target = controller
            editMenu.addItem(item)
        }
        editMenuItem.submenu = editMenu
        mainMenu.addItem(editMenuItem)

        NSApp.mainMenu = mainMenu
    }
}
