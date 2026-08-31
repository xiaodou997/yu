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
    private let outlinePanel = OutlinePanel()
    private var outlineRevision: UInt64?
    private let searchPanel = SearchPanel()
    /// 结果列表是照哪一版画的：Revision 或查询任一变化都要重画。查询本身不
    /// 推进 Revision，所以两者都要记。
    private var searchRevision: UInt64?
    private var searchQuery = ""
    private weak var sidebarStack: NSStackView?
    private var saveButton: NSButton?
    private var reloadButton: NSButton?
    private var initialState: NativeStorageState
    private var fileWatcher: NativeFileWatcher?
    private var externalCheckWorkItem: DispatchWorkItem?
    private var promptedExternalDisk: DiskState?
    private var surfaceBoundsObserver: NSObjectProtocol?
    private weak var documentScrollView: NSScrollView?
    private weak var documentSplitView: NSSplitView?
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
            self.refreshOutline()
            // 编辑之后匹配整体重扫过（Rust 侧），结果列表要跟着换。
            self.refreshSearch()
            self.syncSourceGlyphVisibility()
            self.scheduleVisualSubmit()
        }
        textView.onCaretChange = { [weak self] in
            guard let self else { return }
            // 「当前命中」是从选区推出来的，所以选区一动，结果列表上高亮的
            // 那一行也要跟着动。
            self.searchPanel.highlightRow(matching: self.bridge.selection.range)
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

        // 大纲面板与文档并排。surfaceHostView 仍然直接挂在 root 上、盖在
        // 文档的 clip view 上方——它的 frame 在 viewDidLayout 里由
        // scrollView 的 contentView 换算而来，与分栏无关。
        outlinePanel.onSelect = { [weak self] item in
            self?.textView.navigateToOutlineItem(item)
        }
        searchPanel.onQueryChange = { [weak self] query in
            self?.applySearchQuery(query)
        }
        searchPanel.onSelect = { [weak self] match in
            self?.textView.navigateToSearchMatch(match)
        }

        // 侧栏里两个面板上下叠。用 NSStackView 而不是第二个 NSSplitView：
        // 后者的 holding priority 会再压过首选高度约束一次（陷阱 26 是它的
        // 水平版），而这里根本不需要用户拖分隔线。
        let sidebar = NSStackView(views: [outlinePanel.scrollView, searchPanel.view])
        sidebar.orientation = .vertical
        sidebar.spacing = 0.0
        sidebar.distribution = .fill
        // 横向宽度**显式钉在侧栏上**。竖直 stack 的 alignment 给不出这件事：
        // 默认 `.centerX` 让两个面板按各自的固有宽度居中，而 `.width` 是按
        // 「最宽的那个」对齐——`NSScrollView` 根本没有固有宽度，两种都让查询框
        // 和结果列表缩成窄窄一条，文字被直接裁断。这条是截图抓出来的，全部
        // 自动化断言都绿（与陷阱 26「约束给不出 NSSplitView 的初始分栏位置」
        // 是同一类：布局容器的默认策略与你以为的不是一回事）。
        sidebar.alignment = .leading
        sidebar.translatesAutoresizingMaskIntoConstraints = false
        sidebar.setHuggingPriority(NSLayoutConstraint.Priority(249.0), for: .vertical)
        searchPanel.view.isHidden = true
        sidebarStack = sidebar

        let splitView = NSSplitView()
        splitView.isVertical = true
        splitView.dividerStyle = .thin
        splitView.translatesAutoresizingMaskIntoConstraints = false
        splitView.addArrangedSubview(sidebar)
        splitView.addArrangedSubview(scrollView)
        // 面板守住自己的宽度，缩放窗口时让文档吸收——否则拖窗口会把大纲挤没。
        splitView.setHoldingPriority(
            NSLayoutConstraint.Priority(260.0),
            forSubviewAt: 0
        )
        splitView.setHoldingPriority(
            NSLayoutConstraint.Priority(250.0),
            forSubviewAt: 1
        )
        documentSplitView = splitView

        root.addSubview(toolbar)
        root.addSubview(splitView)
        // The Rust surface is a visual projection above the TextKit mirror.
        // Its hitTest returns nil, so keyboard, IME, selection and scrolling
        // remain owned by the source view underneath it. The frame is synced
        // to the clip viewport in viewDidLayout, excluding native scrollers.
        root.addSubview(surfaceHostView, positioned: .above, relativeTo: splitView)
        NSLayoutConstraint.activate([
            toolbar.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 16),
            toolbar.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -16),
            toolbar.topAnchor.constraint(equalTo: root.topAnchor, constant: 12),
            splitView.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            splitView.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            splitView.topAnchor.constraint(equalTo: toolbar.bottomAnchor, constant: 10),
            splitView.bottomAnchor.constraint(equalTo: root.bottomAnchor),
            sidebar.widthAnchor.constraint(greaterThanOrEqualToConstant: 150.0),
            sidebar.widthAnchor.constraint(lessThanOrEqualToConstant: 420.0),
            outlinePanel.scrollView.widthAnchor.constraint(equalTo: sidebar.widthAnchor),
            searchPanel.view.widthAnchor.constraint(equalTo: sidebar.widthAnchor),
            // 搜索面板占侧栏下半部的一块固定高度，大纲吃掉剩下的。
            searchPanel.view.heightAnchor.constraint(greaterThanOrEqualToConstant: 140.0),
            searchPanel.view.heightAnchor.constraint(
                lessThanOrEqualTo: sidebar.heightAnchor,
                multiplier: 0.6
            ),
        ])
        view = root
        startFileWatcher()
        updateStatus()
        refreshOutline()
    }

    /// 大纲是一份跟着 Revision 走的派生视图，Revision 没动就不必重建——
    /// 光标移动不推进 Revision，不该让整棵树塌一次又展开一次。
    private func refreshOutline(force: Bool = false) {
        let revision = bridge.state.revision
        guard force || outlineRevision != revision else { return }
        guard let items = bridge.outlineItemsIfAvailable else { return }
        outlineRevision = revision
        outlinePanel.reload(
            items: items,
            source: bridge.source as NSString,
            // 剥行内标记：区间由 Rust 给（唯一实现在 DecorationSet 里），
            // 减法在 PanelLabel 里。面板自己不认识 bridge。
            hidden: { [bridge] item in
                bridge.blockHiddenSpans(block: item.block, in: item.labelRange)
            }
        )
    }

    /// 换一份查询：Rust 立刻重扫，结果列表与高亮跟着走。
    ///
    /// 高亮不用平台做任何事——它是场景里的图元，而帧身份带着
    /// `search_generation`，所以一次重提交就够了。
    private func applySearchQuery(_ query: String) {
        guard bridge.setSearchQuery(query) else { return }
        searchQuery = query
        searchRevision = nil
        refreshSearch()
        scheduleVisualSubmit()
    }

    /// 结果列表是一份跟着 (Revision, 查询) 走的派生视图。
    private func refreshSearch(force: Bool = false) {
        guard !searchPanel.view.isHidden else { return }
        let revision = bridge.state.revision
        guard force || searchRevision != revision else { return }
        guard let matches = bridge.searchMatchesIfAvailable else { return }
        searchRevision = revision
        let mirror = bridge.source as NSString
        let rows = matches.map { match in
            SearchResults.row(for: match, in: mirror) { [bridge] block, range in
                // 结果那一行也要剥语法标记，走的是与大纲同一份实现。
                bridge.blockHiddenSpans(block: block, in: range)
            }
        }
        searchPanel.reload(rows: rows, query: searchQuery)
        searchPanel.highlightRow(matching: bridge.selection.range)
    }

    @objc fileprivate func findFromMenu(_ sender: Any?) {
        if searchPanel.view.isHidden {
            showSearchPanel()
        } else {
            view.window?.makeFirstResponder(searchPanel.focusTarget)
        }
    }

    /// 展开搜索面板并把焦点交给查询框。
    ///
    /// `updateSidebarVisibility` 不能少：两个面板都收起时整条侧栏也收起了，
    /// 只把搜索面板的 `isHidden` 翻回来，它仍然在一条隐藏的侧栏里——按 `⌘F`
    /// 什么也不会出现，而且不报错。
    private func showSearchPanel() {
        searchPanel.view.isHidden = false
        updateSidebarVisibility()
        refreshSearch(force: true)
        view.window?.makeFirstResponder(searchPanel.focusTarget)
    }

    @objc fileprivate func findNextFromMenu(_ sender: Any?) {
        advanceSearch(forward: true)
    }

    @objc fileprivate func findPreviousFromMenu(_ sender: Any?) {
        advanceSearch(forward: false)
    }

    /// 跳到下一处/上一处命中。
    ///
    /// **走的是同一个导航入口**（`DocumentTextView.navigate(toSource:)`）：
    /// 选中那一段，滚动由随之而来的 `onCaretChange` 交给 viewport 那条路。
    private func advanceSearch(forward: Bool) {
        guard let matches = bridge.searchMatchesIfAvailable, !matches.isEmpty else { return }
        guard let next = SearchResults.next(
            after: bridge.selection.range,
            in: matches,
            forward: forward
        ) else { return }
        textView.navigateToSearchMatch(next)
    }

    /// 把当前查询的**每一处**匹配都选中，一处一根光标。
    ///
    /// 这是多光标的主入口：匹配已经是有序、互不重叠的一组
    /// （`SearchState::matches`），恰好就是 `Selections` 要的形状。primary 取
    /// **当前那一处**——光标本来在哪，替换之后就还在哪，滚动也不会跳。
    @objc fileprivate func selectAllMatchesFromMenu(_ sender: Any?) {
        guard let matches = bridge.searchMatchesIfAvailable, !matches.isEmpty else { return }
        let ranges = matches.map { $0.range }
        let cursor = bridge.selection.range
        let primary = ranges.firstIndex(where: { $0 == cursor })
            ?? ranges.firstIndex(where: { $0.location >= cursor.location })
            ?? 0
        textView.navigate(toSources: ranges, primary: primary)
        view.window?.makeFirstResponder(textView)
    }

    @objc fileprivate func toggleSearchFromMenu(_ sender: Any?) {
        let hidden = !searchPanel.view.isHidden
        searchPanel.view.isHidden = hidden
        if hidden {
            // 收起面板就收掉搜索：留着高亮而看不见结果列表，是「画面上有东西
            // 但没人说得清它从哪来」。
            bridge.setSearchQuery(nil)
            searchQuery = ""
            searchRevision = nil
            scheduleVisualSubmit()
            if view.window?.firstResponder === searchPanel.focusTarget {
                focusDocument()
            }
            updateSidebarVisibility()
        } else {
            showSearchPanel()
        }
    }

    /// 两个面板都收起来时，整条侧栏也收起来——否则会留下一条空白。
    private func updateSidebarVisibility() {
        sidebarStack?.isHidden = outlinePanel.scrollView.isHidden && searchPanel.view.isHidden
        documentSplitView?.adjustSubviews()
    }

    var searchIsVisible: Bool { !searchPanel.view.isHidden }

    @objc fileprivate func toggleOutlineFromMenu(_ sender: Any?) {
        let hidden = !outlinePanel.scrollView.isHidden
        outlinePanel.scrollView.isHidden = hidden
        updateSidebarVisibility()
        if hidden, view.window?.firstResponder === outlinePanel.focusTarget {
            focusDocument()
        }
    }

    var outlineIsVisible: Bool { !outlinePanel.scrollView.isHidden }

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
        // 分栏的初始位置只能显式放一次：NSSplitView 给 subview 0 加的
        // holding priority 压过 `.defaultLow` 的首选宽度约束，光靠约束面板会
        // 缩到最小值。之后用户拖动仍然生效，min/max 由上面两条约束兜住。
        documentSplitView?.setPosition(220.0, ofDividerAt: 0)
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
        refreshOutline(force: true)
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
        func require<T>(_ value: T?, _ message: String) throws -> T {
            guard let value else { throw Failure(message: message) }
            return value
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

        // 5. 可滚动范围必须等于 Rust 这一帧的内容高度。
        //    它此前来自 document view 自己的 TextKit 排版，两套布局高度不同，
        //    长文档的尾部因此滚不到——没有报错，只是滚不下去。
        let contentHeight = republished?.contentHeight ?? 0.0
        try require(contentHeight > 0.0, "帧未报告内容高度")
        guard let scrollView = documentScrollView,
              let documentView = scrollView.documentView else {
            throw Failure(message: "没有可滚动的 document view")
        }
        let expectedExtent = max(contentHeight, scrollView.contentView.bounds.height)
        try require(
            abs(documentView.frame.height - expectedExtent) <= 0.5,
            "可滚动范围 \(documentView.frame.height) 不等于内容高度 \(expectedExtent)"
        )

        // 6. 大纲面板：选中一行必须把文档滚到那条标题。
        //    headless 压不住这一条——那里没有 scroll view，
        //    `revealCaretIfNeeded` 一进门就返回。这里是它唯一能被证伪的地方。
        let items = try require(bridge.outlineItemsIfAvailable, "拿不到大纲")
        try require(!items.isEmpty, "fixture 里没有标题，这一条压不住任何东西")
        try require(
            outlinePanel.rowCountForSelfCheck == items.count,
            "面板画了 \(outlinePanel.rowCountForSelfCheck) 行，大纲有 \(items.count) 条"
        )
        let lastRow = outlinePanel.rowCountForSelfCheck - 1
        let target = try require(outlinePanel.nodeForSelfCheck(row: lastRow), "取不到最后一行")
        let scrollBefore = scrollView.contentView.bounds.origin.y
        outlinePanel.clickRowForSelfCheck(lastRow)
        try require(
            bridge.selection.range
                == NSRange(location: target.item.labelRange.location, length: 0),
            "选中面板最后一行之后，光标不在 \(target.label) 的正文起点"
        )
        // 产品里这一步由 onCaretChange 排在下一个 main-thread turn 上；
        // self-check 在同一个 turn 里，所以显式跑一次同样那个入口。
        surfaceCoordinator.revealCaretIfNeeded()
        let scrollAfter = scrollView.contentView.bounds.origin.y
        try require(
            scrollAfter > scrollBefore,
            "面板导航之后视口没有滚动（\(scrollBefore) → \(scrollAfter)）"
        )

        // 7. 搜索高亮真的进了屏幕上那一帧。
        //    headless 压不住这一条：那里没有 surface，场景根本不提交。判据是
        //    场景里的矩形条数，不是 `searchMatchesIfAvailable`——后者是被测那
        //    条路的上游，拿它当参照只能证明「我把它读出来了」。
        //    先滚回文首：第 6 步把视口滚到了最后一条标题，而场景只画可见范围，
        //    拿一个不在屏幕上的词去搜会得到一条假红。
        outlinePanel.clickRowForSelfCheck(0)
        surfaceCoordinator.revealCaretIfNeeded()
        let firstNode = try require(outlinePanel.nodeForSelfCheck(row: 0), "取不到第一行")
        let baseline = try require(surfaceCoordinator.submitNow(), "滚回文首之后的重提交失败")
        try require(
            baseline.searchDecorationCount == 0,
            "还没有查询就画出了 \(baseline.searchDecorationCount) 个搜索矩形"
        )
        let needle = String(firstNode.label.prefix(2))
        try require(!needle.isEmpty, "第一条标题没有文字，这一条压不住任何东西")
        try require(bridge.setSearchQuery(needle), "设查询失败")
        try require(
            !surfaceCoordinator.hasCurrentFrame(),
            "换查询之后仍被判为当前帧——搜索框里打字画面会一动不动，而且不报错"
        )
        let searched = try require(surfaceCoordinator.submitNow(), "换查询之后的重提交失败")
        try require(
            searched.searchDecorationCount > 0,
            "查询「\(needle)」在场景里没有画出任何高亮"
        )

        // 8. 收掉搜索，矩形必须一起消失。
        try require(bridge.setSearchQuery(nil), "收掉搜索失败")
        let cleared = try require(surfaceCoordinator.submitNow(), "收掉搜索之后的重提交失败")
        try require(
            cleared.searchDecorationCount == 0,
            "收掉搜索之后还剩 \(cleared.searchDecorationCount) 个高亮"
        )

        // 9. 人工验收 D4 里能自动化的那两条：菜单项上的勾，与「收起面板之后
        //    焦点不能留在面板上」。剩下的（上下键在条目间走）是 AppKit 自己的
        //    行为，不是这里的逻辑。
        let outlineItem = NSMenuItem(
            title: "大纲",
            action: #selector(toggleOutlineFromMenu(_:)),
            keyEquivalent: ""
        )
        _ = validateMenuItem(outlineItem)
        try require(outlineItem.state == .on, "面板可见时菜单项上应当有勾")
        view.window?.makeFirstResponder(outlinePanel.focusTarget)
        try require(
            view.window?.firstResponder === outlinePanel.focusTarget,
            "面板拿不到键盘焦点"
        )
        toggleOutlineFromMenu(nil)
        _ = validateMenuItem(outlineItem)
        try require(outlineItem.state == .off, "面板收起后菜单项上的勾没有跟着变")
        try require(
            view.window?.firstResponder !== outlinePanel.focusTarget,
            "面板收起了，键盘焦点却还留在它上面"
        )
        toggleOutlineFromMenu(nil)
        try require(outlineIsVisible, "面板没有再展开")

        // 10. **多光标真的进了屏幕上那一帧。**
        //
        //     headless 压不住这一条，理由与第 7 步同：那里没有 surface，场景
        //     根本不提交。判据是场景里的 **caret 矩形条数**，不是
        //     `selectionsIfAvailable`——后者是被测那条路的上游。
        //
        //     只画 primary 的表现是「按下选中全部匹配，屏幕上还是一根光标」：
        //     选区在 Rust 里是对的，编辑也是对的，就是看不见——不报错。
        outlinePanel.clickRowForSelfCheck(0)
        surfaceCoordinator.revealCaretIfNeeded()
        let oneCaret = try require(surfaceCoordinator.submitNow(force: true), "单光标帧提交失败")
        try require(
            oneCaret.caretDecorationCount == 1,
            "单光标时画了 \(oneCaret.caretDecorationCount) 根 caret"
        )

        //     ⌥ 点：headless 只能调 `addCaret(atSource:)`，**坐标→源码那一步
        //     只有这里能被证伪**（它要已发布的 viewport 几何）。落点取第一条
        //     标题正文里第三个字符的 caret 矩形——「偏移换不出坐标」与「坐标
        //     换不回偏移」都会让这一条红。
        let caretStart = bridge.selection.range.location
        let optionTarget = caretStart + 2
        let caretRect = try require(
            textView.shapedCaretRectForSelfCheck(sourceUTF16: optionTarget),
            "拿不到偏移 \(optionTarget) 的 caret 矩形"
        )
        let nextRect = try require(
            textView.shapedCaretRectForSelfCheck(sourceUTF16: optionTarget + 1),
            "拿不到偏移 \(optionTarget + 1) 的 caret 矩形"
        )
        //     落点取那个字符格子的**靠左四分之一处**，不是格子的左边界：命中
        //     测试落在两个 caret 位置的正中间时归哪一边是没有定义的，正好点在
        //     边界上会时对时错。
        let optionPoint = NSPoint(
            x: caretRect.origin.x + (nextRect.origin.x - caretRect.origin.x) * 0.25,
            y: caretRect.origin.y + caretRect.height * 0.5
        )
        try require(
            textView.addCaretAtVisualPointForSelfCheck(optionPoint),
            "⌥ 点没有加上光标——坐标换不成源码偏移"
        )
        let added = try require(bridge.selectionsIfAvailable, "拿不到选区")
        try require(
            added.ranges.count == 2,
            "⌥ 点之后应当有两根光标，实际 \(added.ranges.count) 根：\(added.ranges.map { $0.range })"
        )
        //     **判据是它落在点的那个位置附近，不只是「多了一根」。**
        //
        //     只断「两根位置不同」的话，把坐标→源码那一步换成常数 0 也能过
        //     ——⌥ 点到哪里光标都跑到文首，而且不报错。
        //
        //     容差是 1 个 UTF-16 单位，而不是精确相等：点正好落在两个 caret
        //     位置之间时归哪一边没有定义，**精确的边界归属是
        //     `--shaped-projection-hit-test-self-check` 的职责**，这一步只证明
        //     这条路真的走通了。
        let addedLocations = added.ranges.map { $0.range.location }.sorted()
        try require(
            addedLocations.first == caretStart,
            "原来那根光标不该动：\(addedLocations)"
        )
        let landed = try require(addedLocations.last, "没有第二根光标")
        try require(
            abs(landed - optionTarget) <= 1,
            "⌥ 点落在 \(landed)，离点的位置 \(optionTarget) 太远——坐标没有真的换成源码偏移"
        )
        try require(
            !surfaceCoordinator.hasCurrentFrame(),
            "加一根光标之后仍被判为当前帧——画面会一动不动，而且不报错"
        )
        let twoCarets = try require(surfaceCoordinator.submitNow(), "双光标帧提交失败")
        try require(
            twoCarets.caretDecorationCount == 2,
            "两根光标只画出了 \(twoCarets.caretDecorationCount) 根 caret"
        )

        //     选中全部匹配：N 段选区底色都要进帧。
        //     第 7 步那个 needle 取自第一条标题，未必重复。这里要的是一个
        //     **至少两处、而且都落在文首这一屏里**的查询：场景只画可见范围，
        //     拿散在全文的匹配去数矩形会得到一条假红（第 7 步踩过同一个坑）。
        let multiNeedle = "层级一"
        try require(bridge.setSearchQuery(multiNeedle), "设查询失败")
        let allMatches = try require(bridge.searchMatchesIfAvailable, "拿不到匹配")
        try require(
            allMatches.count >= 2,
            "fixture 里「\(multiNeedle)」不足两处，这一条压不住任何东西"
        )
        selectAllMatchesFromMenu(nil)

        //     两个判据分开：**Rust 侧真的选中了 N 条**（选区那条路），
        //     与**画面上真的出现了 N 块底色**（场景那条路）。前者证明命令干活
        //     了，后者证明干的活看得见——只画 primary 的话前者绿、后者红。
        let everything = try require(bridge.selectionsIfAvailable, "拿不到选区")
        try require(
            everything.ranges.count == allMatches.count,
            "选中全部匹配之后只有 \(everything.ranges.count) 条选区，匹配有 \(allMatches.count) 处"
        )
        let selectedAll = try require(surfaceCoordinator.submitNow(), "全部选中之后的重提交失败")
        try require(
            selectedAll.selectionDecorationCount >= allMatches.count,
            "\(allMatches.count) 处匹配只画出了 \(selectedAll.selectionDecorationCount) 块选区底色"
        )
        try require(bridge.setSearchQuery(nil), "收掉搜索失败")
        textView.navigate(toSource: NSRange(location: 0, length: 0))
        let highlightFrame = try require(
            surfaceCoordinator.submitNow(),
            "收掉搜索之后的重提交失败"
        )

        // 11. **代码高亮真的进了屏幕上那一帧。**
        //
        //     headless 的 `--code-highlight-self-check` 数的是 retained frame；
        //     这一条数的是**真实 Metal surface 提交的那一帧**。第三刀与第四刀
        //     各有一个缺陷是在自动化全绿、headless 也全绿之后才被真实窗口抓到
        //     的，两次都是颜色——这一刀改的就是字形颜色。
        //
        //     判据是场景图元的颜色数（`highlightedGlyphCount`），不是装饰、
        //     不是 `TextRole`。fixture 首屏里有一个 ```rust 块（见
        //     Fixtures/outline.md 里那段说明），没有它这一条会假红。
        try require(
            highlightFrame.highlightedGlyphCount > 0,
            "屏幕上那一帧一个高亮字形都没有——代码块的颜色没走到场景里"
        )
        //     不是所有字形都被刷成同一种颜色：正文与标题必须还是正文色。
        //     只断「大于零」的话，一个把每个字形都上色的实现也能过。
        let highlighted = highlightFrame.highlightedGlyphCount
        let commands = Int(highlightFrame.commandCount)
        try require(
            highlighted < commands,
            "这一帧的字形全被算成了高亮（\(highlighted) / \(commands) 条指令）"
        )

        print(
            "Yu frame scheduling self-check: commands=\(snapshot?.commandCount ?? 0) "
                + "caret=\(republished?.caretDecorationCount ?? 0) "
                + "selection=\(republished?.selectionDecorationCount ?? 0) "
                + "search=\(searched.searchDecorationCount)→\(cleared.searchDecorationCount) "
                + "carets=\(oneCaret.caretDecorationCount)→\(twoCarets.caretDecorationCount) "
                + "multiSelection=\(selectedAll.selectionDecorationCount) "
                + "extent=\(Int(documentView.frame.height)) "
                + "outlineRows=\(outlinePanel.rowCountForSelfCheck) "
                + "outlineScroll=\(Int(scrollBefore))→\(Int(scrollAfter)) "
                + "highlightedGlyphs=\(highlighted)"
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
        if menuItem.action == #selector(toggleOutlineFromMenu(_:)) {
            menuItem.state = outlineIsVisible ? .on : .off
            return true
        }
        if menuItem.action == #selector(toggleSearchFromMenu(_:)) {
            menuItem.state = searchIsVisible ? .on : .off
            return true
        }
        if menuItem.action == #selector(findNextFromMenu(_:)) ||
            menuItem.action == #selector(findPreviousFromMenu(_:)) ||
            menuItem.action == #selector(selectAllMatchesFromMenu(_:)) {
            // 没有查询就没有「下一个」，也没有「全部匹配」。灰掉比按下去什么
            // 也不发生要诚实。
            return !(bridge.searchMatchesIfAvailable ?? []).isEmpty
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
                    try bridge.resolveClose(UInt8(YU_STORAGE_CLOSE_RESOLVE_DISCARD))
                } else {
                    try bridge.resolveClose(UInt8(YU_STORAGE_CLOSE_RESOLVE_SAVE))
                }
                return true
            }
            try bridge.resolveClose(UInt8(YU_STORAGE_CLOSE_RESOLVE_CANCEL))
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
        editMenu.addItem(.separator())
        let findItems: [(String, Selector, String, NSEvent.ModifierFlags)] = [
            ("查找", #selector(DocumentViewController.findFromMenu(_:)), "f", [.command]),
            ("查找下一个", #selector(DocumentViewController.findNextFromMenu(_:)), "g", [.command]),
            (
                "查找上一个",
                #selector(DocumentViewController.findPreviousFromMenu(_:)),
                "g",
                [.command, .shift]
            ),
            (
                "选中全部匹配",
                #selector(DocumentViewController.selectAllMatchesFromMenu(_:)),
                "l",
                [.command, .shift]
            ),
        ]
        for (title, action, keyEquivalent, modifiers) in findItems {
            let item = NSMenuItem(title: title, action: action, keyEquivalent: keyEquivalent)
            item.keyEquivalentModifierMask = modifiers
            item.target = controller
            editMenu.addItem(item)
        }
        editMenuItem.submenu = editMenu
        mainMenu.addItem(editMenuItem)

        let viewMenuItem = NSMenuItem()
        let viewMenu = NSMenu(title: "显示")
        let outline = NSMenuItem(
            title: "大纲",
            action: #selector(DocumentViewController.toggleOutlineFromMenu(_:)),
            keyEquivalent: "1"
        )
        outline.keyEquivalentModifierMask = [.command, .option]
        outline.target = controller
        viewMenu.addItem(outline)
        let searchToggle = NSMenuItem(
            title: "搜索结果",
            action: #selector(DocumentViewController.toggleSearchFromMenu(_:)),
            keyEquivalent: "2"
        )
        searchToggle.keyEquivalentModifierMask = [.command, .option]
        searchToggle.target = controller
        viewMenu.addItem(searchToggle)
        viewMenuItem.submenu = viewMenu
        mainMenu.addItem(viewMenuItem)

        NSApp.mainMenu = mainMenu
    }
}
