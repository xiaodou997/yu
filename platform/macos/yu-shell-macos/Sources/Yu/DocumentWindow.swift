import AppKit
import ScreenCaptureKit
import CryptoKit
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
final class DocumentViewController: NSViewController, NSMenuItemValidation, NSToolbarDelegate {
    private let bridge: StorageBridge
    let persistence: NativeDocumentPersistence
    var documentURL: URL { URL(fileURLWithPath: bridge.path) }
    var onValidateSaveDestination: ((URL) throws -> Void)?
    var onDocumentURLChange: (() -> Void)?
    // Inject user decisions in lifecycle checks; production uses native panels.
    var savePanelDecision: ((NSSavePanel) -> URL?)?
    var closeAlertDecision: ((NSAlert) -> NSApplication.ModalResponse)?
    var externalAlertDecision: ((NSAlert) -> NSApplication.ModalResponse)?
    var fileErrorPresenter: ((Error) -> Void)?
    func withFileInputForSelfCheck(_ action: (DocumentTextView) -> Void) { action(textView) }
    private lazy var textView = DocumentTextView(bridge: bridge)
    func makeTableMenu() -> NSMenu { textView.makeTableMenu() }
    @objc fileprivate func editImagePropertiesFromMenu(_ sender: NSMenuItem?) { textView.editImagePropertiesFromMenu(sender) }
    private let surfaceHostView = MacosSurfaceHostView()
    private let surfaceCoordinator: MacosSurfaceHostCoordinator
    private let statusLabel = NSTextField(labelWithString: "")
    private let statusDetailLabel = NSTextField(labelWithString: "")
    private let outlinePanel = OutlinePanel()
    private var outlineRevision: UInt64?
    private let searchPanel = SearchPanel()
    /// 结果列表是照哪一版画的：Revision 或查询任一变化都要重画。查询本身不
    /// 推进 Revision，所以两者都要记。
    private var searchRevision: UInt64?
    private var searchQuery = ""
    private weak var sidebarStack: NSStackView?
    private weak var sidebarContainer: NSView?
    private lazy var filePanel = FilePanel(directory: persistence.isUntitled
        ? (FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first ?? FileManager.default.homeDirectoryForCurrentUser)
        : documentURL.deletingLastPathComponent())
    private let sidebarTabs = NSSegmentedControl()
    private let splitController = NSSplitViewController()
    private var sidebarItem: NSSplitViewItem?
    private let findAccessory = NSSplitViewItemAccessoryViewController()
    private var sidebarHidden = false
    var onOpenDocument: ((URL) -> Void)?
    private var initialState: NativeStorageState
    private var fileWatcher: NativeFileWatcher?
    private var externalCheckWorkItem: DispatchWorkItem?
    private var promptedExternalDisk: DiskState?
    private var surfaceBoundsObserver: NSObjectProtocol?
    private var surfaceFrameObserver: NSObjectProtocol?
    private var scrollLifecycleObservers: [NSObjectProtocol] = []
    private weak var documentScrollView: NSScrollView?
    private weak var documentSplitView: NSSplitView?
    private var chromeTopConstraint: NSLayoutConstraint?
    private var memoryPressure: DispatchSourceMemoryPressure?
    private var visualPointerAdapterEnabled = false
    private var visualPointerLayoutWidth: CGFloat = -1.0
    /// Native input geometry is enabled once the view has a valid reading column.
    private var visualEnhancementsReady = false
    private var isCapturingVisualAcceptance = false
    private var tracksSidebarWidth = false
    private var readingZoom: CGFloat = 1
    private var preferredSidebarWidth: CGFloat = {
        let saved = UserDefaults.standard.double(forKey: "Yu.sidebarWidth")
        return (180...400).contains(saved) ? CGFloat(saved) : 240
    }()

    init(bridge: StorageBridge, recovered: Bool = false) {
        self.bridge = bridge
        self.persistence = NativeDocumentPersistence(bridge: bridge, recovered: recovered)
        self.surfaceCoordinator = MacosSurfaceHostCoordinator(bridge: bridge)
        self.initialState = bridge.state
        super.init(nibName: nil, bundle: nil)
        persistence.onChange = { [weak self] in
            guard let self, self.isViewLoaded else { return }
            self.initialState = self.bridge.state
            self.updateStatus()
        }
        surfaceCoordinator.onSurfaceStateChange = { [weak self] in
            self?.textView.refreshTableResizeAccessibility()
            self?.syncSourceGlyphVisibility()
        }
        surfaceCoordinator.onPresentationStorageError = { [weak self] error in
            self?.show(error)
        }
        surfaceCoordinator.onError = { [weak self] error in
            // Report an unavailable surface without pretending a fallback rendered it.
            self?.statusLabel.stringValue = "文档暂时无法显示"
            self?.statusLabel.toolTip = error.localizedDescription
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

    deinit {
        if let surfaceBoundsObserver {
            NotificationCenter.default.removeObserver(surfaceBoundsObserver)
        }
        if let surfaceFrameObserver {
            NotificationCenter.default.removeObserver(surfaceFrameObserver)
        }
        for observer in scrollLifecycleObservers {
            NotificationCenter.default.removeObserver(observer)
        }
        memoryPressure?.cancel()
        surfaceCoordinator.detach()
    }

    override func loadView() {
        let root = NSView()
        let scrollView = NSScrollView()
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = false
        scrollView.autohidesScrollers = true
        // The seamless shell and reading geometry already own all top/bottom
        // insets. AppKit's extra titlebar inset would create false overflow.
        scrollView.automaticallyAdjustsContentInsets = false
        scrollView.contentInsets = NSEdgeInsets(top: 0, left: 0, bottom: 0, right: 0)
        // Reserve the native scroller's width in the reading viewport, as in
        // the fixed Typora reference. All input and Metal geometry use this
        // same clip view; no compensating offset is added to the text.
        scrollView.scrollerStyle = .overlay
        scrollView.verticalScrollElasticity = .automatic
        scrollView.drawsBackground = true
        scrollView.backgroundColor = YuVisualTokens.canvas
        scrollView.contentView.backgroundColor = YuVisualTokens.canvas
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        surfaceHostView.translatesAutoresizingMaskIntoConstraints = true
        surfaceHostView.autoresizingMask = []
        surfaceHostView.setAccessibilityElement(false)
        documentScrollView = scrollView

        do { try bridge.setFocusMode(NativeWritingPreferences.shared.focusMode) } catch { show(error) }
        textView.onImageImport = { [weak self] inputs, target in try self?.importImages(inputs, at: target) ?? false }
        textView.isEditable = true
        textView.isSelectable = true
        textView.onSpellingChange = { [weak self] in self?.scheduleVisualSubmit() }
        textView.onResourceChange = { [weak self] in self?.scheduleVisualSubmit() }
        textView.onDocumentChange = { [weak self] in
            guard let self else { return }
            self.view.window?.invalidateRestorableState()
            self.initialState = self.bridge.state
            self.persistence.documentChanged()
            self.surfaceCoordinator.resetTableResizeAfterDocumentChange()
            self.textView.refreshTableResizeAccessibility()
            self.updateStatus()
            self.refreshOutline()
            // 编辑之后匹配整体重扫过（Rust 侧），结果列表要跟着换。
            self.refreshSearch()
            self.syncSourceGlyphVisibility()
            self.scheduleVisualSubmit()
            if NativeWritingPreferences.shared.typewriterMode {
                let generation = self.surfaceCoordinator.caretRevealGeneration
                DispatchQueue.main.async { [weak self] in
                    guard let self, self.visualEnhancementsReady,
                          !self.isCapturingVisualAcceptance else { return }
                    self.surfaceCoordinator.revealCaretIfNeeded(typewriter: true, generation: generation)
                }
            }
        }
        textView.onCaretChange = { [weak self] in
            guard let self else { return }
            // 「当前命中」是从选区推出来的，所以选区一动，结果列表上高亮的
            // 那一行也要跟着动。
            self.view.window?.invalidateRestorableState()
            self.searchPanel.highlightRow(matching: self.bridge.selection.range)
            self.outlinePanel.highlightHeading(containing: Int(self.bridge.selectionEndpoints.focusUTF16))
            // 光标移动不推进 Revision，但会改变 caret 与选区装饰。Rust 的帧
            // 身份已经把 selection 算在内，平台不需要再显式作废任何东西。
            self.scheduleVisualSubmit()
            // AppKit may deliver selection changes while TextKit is still
            // inside its event callback. Defer the scroll mutation until the
            // same main-thread turn has finished, while retaining the Rust
            // Revision captured by the coordinator's query.
            let generation = self.surfaceCoordinator.caretRevealGeneration
            DispatchQueue.main.async { [weak self] in
                guard let self, self.visualEnhancementsReady, !self.isCapturingVisualAcceptance else { return }
                self.surfaceCoordinator.revealCaretIfNeeded(generation: generation)
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
        // NSView does not track the clip width like NSTextView. Keep its
        // initial frame usable; updateReadingColumnInsets owns the final width.
        textView.frame = NSRect(x: 0, y: 0, width: 900, height: 620)
        // 初始 inset 与 updateReadingColumnInsets 的算法同源（token）：窗口
        // 第一次布局前，占位值也要是「窄窗边距 30pt、顶部 30pt」的形状。
        textView.contentInsets = NSSize(
            width: YuVisualTokens.readingColumnMinGutter,
            height: YuVisualTokens.readingColumnTopInset
        )
        textView.autoresizingMask = []


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
            self?.updateReadingColumnInsets()
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
            self?.syncSurfaceGeometry()
            self?.surfaceCoordinator.noteBoundsEvent()
            self?.textView.scheduleSpellingCheck()
            self?.textView.inputContext?.invalidateCharacterCoordinates()
            self?.scheduleVisualSubmit()
            self?.syncSourceGlyphVisibility()
            self?.textView.refreshTableResizeAccessibility()
        }

        // NSScrollView can finish resizing its clip view after the controller's
        // viewDidLayout callback. Follow that final frame, not the earlier size.
        scrollView.contentView.postsFrameChangedNotifications = true
        surfaceFrameObserver = NotificationCenter.default.addObserver(
            forName: NSView.frameDidChangeNotification,
            object: scrollView.contentView, queue: .main
        ) { [weak self] _ in
            self?.updateReadingColumnInsets()
            self?.syncSurfaceGeometry()
            self?.scheduleVisualSubmit()
        }

        let notifications: [(Notification.Name, () -> Void)] = [
            (NSScrollView.willStartLiveScrollNotification, { [weak self] in
                self?.surfaceCoordinator.beginLiveScroll()
            }),
            (NSScrollView.didLiveScrollNotification, { [weak self] in
                self?.scheduleVisualSubmit()
            }),
            (NSScrollView.didEndLiveScrollNotification, { [weak self] in
                self?.view.window?.invalidateRestorableState()
                self?.surfaceCoordinator.endLiveScroll()
                self?.scheduleVisualSubmit()
            }),
        ]
        scrollLifecycleObservers = notifications.map { name, handler in
            NotificationCenter.default.addObserver(
                forName: name,
                object: scrollView,
                queue: .main
            ) { _ in handler() }
        }

        statusLabel.setAccessibilityElement(true)
        statusLabel.setAccessibilityLabel("文档状态")
        statusLabel.translatesAutoresizingMaskIntoConstraints = false
        statusLabel.font = NSFont.systemFont(ofSize: YuVisualTokens.statusBarFontSize)
        statusLabel.textColor = NativeTheme.color(\.text).withAlphaComponent(0.65)

        statusDetailLabel.font = NSFont.monospacedDigitSystemFont(
            ofSize: YuVisualTokens.statusBarFontSize,
            weight: .regular
        )
        statusDetailLabel.textColor = NativeTheme.color(\.text).withAlphaComponent(0.65)
        statusDetailLabel.alignment = .right
        statusDetailLabel.translatesAutoresizingMaskIntoConstraints = false
        statusDetailLabel.setAccessibilityElement(true)
        statusDetailLabel.setAccessibilityLabel("字数")

        let statusBar = YuStatusBarView()
        statusBar.addSubview(statusLabel)
        statusBar.addSubview(statusDetailLabel)
        NSLayoutConstraint.activate([
            statusLabel.leadingAnchor.constraint(equalTo: statusBar.leadingAnchor, constant: 12.0),
            statusLabel.centerYAnchor.constraint(equalTo: statusBar.centerYAnchor),
            statusDetailLabel.trailingAnchor.constraint(equalTo: statusBar.trailingAnchor, constant: -12.0),
            statusDetailLabel.centerYAnchor.constraint(equalTo: statusBar.centerYAnchor),
        ])

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
        sidebarTabs.segmentCount = 2
        sidebarTabs.setLabel("文件", forSegment: 0)
        sidebarTabs.setLabel("大纲", forSegment: 1)
        sidebarTabs.selectedSegment = 1
        sidebarTabs.segmentStyle = .automatic
        sidebarTabs.segmentDistribution = .fillEqually
        sidebarTabs.toolTip = "切换文件或大纲"
        sidebarTabs.target = self
        sidebarTabs.action = #selector(selectSidebarTab(_:))
        sidebarTabs.translatesAutoresizingMaskIntoConstraints = false
        sidebarTabs.setAccessibilityLabel("侧栏内容")
        let sidebarHeader = NSView()
        sidebarHeader.translatesAutoresizingMaskIntoConstraints = false
        sidebarHeader.addSubview(sidebarTabs)
        NSLayoutConstraint.activate([
            sidebarTabs.topAnchor.constraint(equalTo: sidebarHeader.topAnchor, constant: 8),
            sidebarTabs.bottomAnchor.constraint(equalTo: sidebarHeader.bottomAnchor, constant: -8),
            sidebarTabs.leadingAnchor.constraint(equalTo: sidebarHeader.leadingAnchor, constant: 12),
            sidebarTabs.trailingAnchor.constraint(equalTo: sidebarHeader.trailingAnchor, constant: -12),
            sidebarTabs.centerYAnchor.constraint(equalTo: sidebarHeader.centerYAnchor)
        ])
        filePanel.onOpen = { [weak self] url in self?.onOpenDocument?(url) }
        filePanel.view.isHidden = true
        let sidebar = NSStackView(views: [sidebarHeader, outlinePanel.scrollView, filePanel.view])
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

        let sidebarContainer = NSView()
        sidebarContainer.translatesAutoresizingMaskIntoConstraints = false
        sidebarContainer.addSubview(sidebar)
        NSLayoutConstraint.activate([
            sidebar.leadingAnchor.constraint(equalTo: sidebarContainer.leadingAnchor),
            sidebar.trailingAnchor.constraint(equalTo: sidebarContainer.trailingAnchor),
            sidebar.topAnchor.constraint(equalTo: sidebarContainer.safeAreaLayoutGuide.topAnchor),
            sidebar.bottomAnchor.constraint(equalTo: sidebarContainer.bottomAnchor),
        ])
        self.sidebarContainer = sidebarContainer

        let editor = NSView()
        editor.addSubview(scrollView)
        let sidebarController = NSViewController()
        sidebarController.view = sidebarContainer
        let editorController = NSViewController()
        editorController.view = editor
        addChild(splitController)
        let sidebarItem = NSSplitViewItem(sidebarWithViewController: sidebarController)
        sidebarItem.minimumThickness = 180
        sidebarItem.maximumThickness = 400
        sidebarItem.canCollapse = true
        self.sidebarItem = sidebarItem
        let editorItem = NSSplitViewItem(viewController: editorController)
        editorItem.automaticallyAdjustsSafeAreaInsets = true
        findAccessory.view = searchPanel.view
        findAccessory.isHidden = true
        editorItem.addTopAlignedAccessoryViewController(findAccessory)
        searchPanel.onClose = { [weak self] in self?.setSearchPanelHidden(true) }
        searchPanel.onNext = { [weak self] forward in self?.advanceSearch(forward: forward) }
        splitController.addSplitViewItem(sidebarItem)
        splitController.addSplitViewItem(editorItem)
        let splitView = splitController.splitView
        splitView.isVertical = true
        // Keep the controller's complete hierarchy attached. Modern AppKit
        // owns accessory/safe-area layout around the split view itself.
        let splitHost = splitController.view
        splitHost.translatesAutoresizingMaskIntoConstraints = false
        documentSplitView = splitView
        scrollLifecycleObservers.append(NotificationCenter.default.addObserver(
            forName: NSSplitView.didResizeSubviewsNotification, object: splitView, queue: .main
        ) { [weak self] _ in
            guard let self, self.tracksSidebarWidth, !self.sidebarHidden, !self.isCapturingVisualAcceptance,
                  let width = self.sidebarContainer?.frame.width, width >= 180, width <= 400 else { return }
            self.preferredSidebarWidth = width
            self.view.window?.invalidateRestorableState()
            if !CommandLine.arguments.contains(where: { $0.hasSuffix("-self-check") }) {
                UserDefaults.standard.set(Double(width), forKey: "Yu.sidebarWidth")
            }
        })

        root.addSubview(splitHost)
        root.addSubview(statusBar)
        // The Rust surface is a visual projection above the native input host.
        // Its hitTest returns nil, so keyboard, IME, selection and scrolling
        // remain owned by the input view underneath it. The frame is synced
        // to the clip viewport in viewDidLayout, excluding native scrollers.
        root.addSubview(surfaceHostView, positioned: .above, relativeTo: splitHost)
        let chromeTop = splitHost.topAnchor.constraint(equalTo: root.topAnchor, constant: 0)
        chromeTopConstraint = chromeTop
        NSLayoutConstraint.activate([
            splitHost.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            splitHost.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            chromeTop,
            splitHost.bottomAnchor.constraint(equalTo: root.bottomAnchor),
            statusBar.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            statusBar.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            statusBar.bottomAnchor.constraint(equalTo: root.bottomAnchor),
            statusBar.heightAnchor.constraint(equalToConstant: YuVisualTokens.statusBarHeight),
            // 「侧栏内容 ≥ 设计宽度」钉在容器上而不是 navigation 上：隐藏的
            // 视图不参与 Auto Layout，「文档」模式（整条侧栏收起）只剩 rail。
            // 钉在 navigation 上会在两个面板都收起时留下一段空白侧栏。
            sidebarContainer.widthAnchor.constraint(greaterThanOrEqualToConstant: 180),
            sidebarContainer.widthAnchor.constraint(lessThanOrEqualToConstant: 400),
            sidebarHeader.widthAnchor.constraint(equalTo: sidebar.widthAnchor),
            outlinePanel.scrollView.widthAnchor.constraint(equalTo: sidebar.widthAnchor),
            filePanel.view.widthAnchor.constraint(equalTo: sidebar.widthAnchor),
            scrollView.leadingAnchor.constraint(equalTo: editor.safeAreaLayoutGuide.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: editor.safeAreaLayoutGuide.trailingAnchor),
            scrollView.topAnchor.constraint(equalTo: editor.safeAreaLayoutGuide.topAnchor),
            scrollView.bottomAnchor.constraint(equalTo: editor.safeAreaLayoutGuide.bottomAnchor),
        ])
        view = root
        // 初始显隐：大纲开、搜索关 → rail 选中「大纲」。之后的每次显隐突变都
        // 经 updateSidebarVisibility 同步，这里只补初始一拍。
        updateSidebarVisibility()
        scrollLifecycleObservers.append(NotificationCenter.default.addObserver(
            forName: NativeTheme.didChange, object: nil, queue: .main
        ) { [weak self] _ in
            guard let self else { return }
            self.textView.scheduleSpellingCheck()
            do { try self.bridge.setFocusMode(NativeWritingPreferences.shared.focusMode) } catch { self.show(error) }
            self.surfaceCoordinator.retainPositionForPresentationChange()
            let theme = NativeTheme.spec(dark: self.view.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua)
            let size = CGFloat(NativeWritingPreferences.shared.fontSize) * self.readingZoom
            self.textView.font = NativeTheme.font(identity: theme.body_font, size: size)
            self.surfaceCoordinator.setFontSize(size)
            self.textView.refreshTableResizeAccessibility()
            // These labels sit on the document canvas, so their contrast must
            // follow the reading theme even when AppKit uses another appearance.
            self.statusLabel.textColor = NativeTheme.color(\.text).withAlphaComponent(0.65)
            self.statusDetailLabel.textColor = NativeTheme.color(\.text).withAlphaComponent(0.65)
            self.documentScrollView?.backgroundColor = YuVisualTokens.canvas
            self.documentScrollView?.contentView.backgroundColor = YuVisualTokens.canvas
            self.view.needsLayout = true
            self.scheduleVisualSubmit()
        })
        let pressure = DispatchSource.makeMemoryPressureSource(eventMask: [.warning, .critical], queue: .main)
        pressure.setEventHandler { [weak self] in self?.surfaceCoordinator.releaseRebuildableCaches() }
        pressure.resume()
        memoryPressure = pressure
        startFileWatcher()
        updateStatus()
        refreshOutline()
    }

    /// 大纲是一份跟着 Revision 走的派生视图，Revision 没动就不必重建——
    /// 光标移动不推进 Revision，不该让整棵树塌一次又展开一次。
    private func refreshOutline(force: Bool = false) {
        let revision = bridge.revision
        guard force || outlineRevision != revision else { return }
        guard let items = bridge.outlineItemsIfAvailable else { return }
        outlineRevision = revision
        // 树的形状、每一行的文字与身份都由 Rust 给（`OutlineTree`）；面板
        // 只是把它喂给 NSOutlineView。
        outlinePanel.reload(items: items)
        outlinePanel.highlightHeading(containing: Int(bridge.selectionEndpoints.focusUTF16))
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
        let revision = bridge.revision
        guard force || searchRevision != revision else { return }
        guard let matches = bridge.searchMatchesIfAvailable else { return }
        searchRevision = revision
        // 每一行显示成什么字由 Rust 给（`SearchResults`），走的是与大纲
        // 同一份实现。
        searchPanel.reload(rows: matches, query: searchQuery)
        searchPanel.highlightRow(matching: bridge.selection.range)
    }

    @objc fileprivate func findFromMenu(_ sender: Any?) {
        if searchPanel.view.isHidden {
            showSearchPanel()
        } else {
            view.window?.makeFirstResponder(searchPanel.focusTarget)
        }
    }

    /// 展开搜索面板并把焦点交给查询框（焦点在 `setSearchPanelHidden(false)`
    /// 里给，这里不再重复）。
    ///
    /// `updateSidebarVisibility` 不能少：两个面板都收起时整条侧栏也收起了，
    /// 只把搜索面板的 `isHidden` 翻回来，它仍然在一条隐藏的侧栏里——按 `⌘F`
    /// 什么也不会出现，而且不报错。
    private func showSearchPanel() {
        setSearchPanelHidden(false)
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
        setSearchPanelHidden(!searchPanel.view.isHidden)
    }

    /// 搜索面板的显隐只有这一条写路径：菜单 `⌥⌘2`、rail 的「搜索」、
    /// `⌘F` 展开，都从这里走，收起的清理（撤查询、撤高亮、还焦点）不另写。
    private func setSearchPanelHidden(_ hidden: Bool) {
        findAccessory.isHidden = hidden
        searchPanel.view.isHidden = hidden
        if hidden {
            // 收起面板就收掉搜索：留着高亮而看不见结果列表，是「画面上有东西
            // 但没人说得清它从哪来」。
            bridge.setSearchQuery(nil)
            searchQuery = ""
            searchRevision = nil
            scheduleVisualSubmit()
            focusDocument()
        } else {
            refreshSearch(force: true)
            view.window?.makeFirstResponder(searchPanel.focusTarget)
        }
        updateSidebarVisibility()
    }

    private func updateSidebarVisibility() {
        view.window?.invalidateRestorableState()
        sidebarItem?.isCollapsed = sidebarHidden
        outlinePanel.scrollView.isHidden = sidebarTabs.selectedSegment != 1
        filePanel.view.isHidden = sidebarTabs.selectedSegment != 0
        documentSplitView?.adjustSubviews()
    }

    @objc private func selectSidebarTab(_ sender: Any?) {
        sidebarHidden = false
        updateSidebarVisibility()
    }

    var searchIsVisible: Bool { !searchPanel.view.isHidden }

    @objc fileprivate func toggleOutlineFromMenu(_ sender: Any?) {
        setOutlinePanelHidden(outlineIsVisible)
    }

    /// 大纲面板的显隐只有这一条写路径（rail 的「文档」/「搜索」与菜单
    /// `⌥⌘1` 共用）：区头与列表同生同灭，显隐不分开写。
    private func setOutlinePanelHidden(_ hidden: Bool) {
        sidebarHidden = hidden
        if !hidden { sidebarTabs.selectedSegment = 1 }
        updateSidebarVisibility()
        if hidden { focusDocument() }
    }

    var outlineIsVisible: Bool { !sidebarHidden && sidebarTabs.selectedSegment == 1 }

    private func syncSurfaceGeometry() {
        guard let scrollView = documentScrollView else { return }
        let viewportFrame = view.convert(scrollView.contentView.frame, from: scrollView)
        if visualEnhancementsReady {
            surfaceCoordinator.setHorizontalContentInset(2.0 * textView.contentOrigin.x)
        }
        if surfaceHostView.frame != viewportFrame {
            surfaceHostView.frame = viewportFrame
        }
    }

    override func viewWillLayout() {
        chromeTopConstraint?.constant = 0
        super.viewWillLayout()
    }

    override func viewDidLayout() {
        super.viewDidLayout()
        updateReadingColumnInsets()
        syncSurfaceGeometry()
        guard visualEnhancementsReady else {
            // Surface submission starts in viewDidAppear, once AppKit has a
            // real window and clip geometry to report.
            return
        }
        textView.refreshTableResizeAccessibility()
        // 指针命中测试直接走 Rust layout，不需要预先建立任何 TextKit 镜像，
        // 因而也没有「适配器未就绪」这个状态。
        syncSourceGlyphVisibility()
        textView.refreshTableResizeAccessibility()
        surfaceCoordinator.scheduleSubmit()
        surfaceCoordinator.refineCaretRevealIfNeeded()
    }

    /// The native input host and surface share the resolved theme's column.
    private func updateReadingColumnInsets() {
        guard isViewLoaded, let scrollView = documentScrollView else { return }
        let width = scrollView.contentView.bounds.width
        if width > 0, abs(textView.frame.width - width) > 0.01 {
            textView.setFrameSize(NSSize(width: width, height: textView.frame.height))
        }
        let geometry = NativeTheme.reading(width: width, windowWidth: view.window?.frame.width ?? view.bounds.width, dark: surfaceHostView.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua)
        let next = NSSize(width: CGFloat(geometry.origin_x), height: CGFloat(geometry.origin_y))
        if textView.contentInsets != next {
            textView.contentInsets = next
            textView.needsLayout = true
        }
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        guard !visualEnhancementsReady else { return }
        visualEnhancementsReady = true
        if let error = bridge.presentationStorageError {
            show(error)
        }
        // 分栏的初始位置只能显式放一次：NSSplitView 给 subview 0 加的
        // holding priority 压过 `.defaultLow` 的首选宽度约束，光靠约束面板会
        // 缩到最小值。之后用户拖动仍然生效，min/max 由上面两条约束兜住。
        documentSplitView?.setPosition(preferredSidebarWidth, ofDividerAt: 0)
        tracksSidebarWidth = true
        installToolbar()
        // Defer the first optional projection submit by one main-thread turn
        // so source TextKit focus/IME setup has completed before any native
        // surface callback can run.
        DispatchQueue.main.async { [weak self] in
            guard let self, self.view.window != nil else { return }
            self.view.needsLayout = true
            self.scheduleVisualSubmit()
        }
    }

    private func installToolbar() {
        guard let window = view.window, window.toolbar == nil else { return }
        let toolbar = NSToolbar(identifier: "Yu.document")
        toolbar.delegate = self
        toolbar.allowsUserCustomization = true
        toolbar.autosavesConfiguration = true
        toolbar.displayMode = .iconOnly
        window.toolbar = toolbar
        window.toolbarStyle = .unified
        window.autorecalculatesKeyViewLoop = true
    }

    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        [NSToolbarItem.Identifier("yu.sidebar"), .flexibleSpace,
         NSToolbarItem.Identifier("yu.find"), NSToolbarItem.Identifier("yu.source"), NSToolbarItem.Identifier("yu.more")]
    }

    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        toolbarDefaultItemIdentifiers(toolbar) + [.space]
    }

    func toolbar(_ toolbar: NSToolbar, itemForItemIdentifier identifier: NSToolbarItem.Identifier,
                 willBeInsertedIntoToolbar flag: Bool) -> NSToolbarItem? {
        if identifier.rawValue == "yu.more" {
            let item = NSMenuToolbarItem(itemIdentifier: identifier)
            item.label = "阅读主题"
            item.image = NSImage(systemSymbolName: "ellipsis.circle", accessibilityDescription: "更多")
            let menu = NSMenu(title: "阅读主题")
            for (index, title) in ["Yu · 跟随系统", "Github", "Night"].enumerated() {
                let action = NSMenuItem(title: title, action: #selector(selectReadingTheme(_:)), keyEquivalent: "")
                action.target = self
                action.tag = index
                menu.addItem(action)
            }
            item.menu = menu
            return item
        }
        let item = NSToolbarItem(itemIdentifier: identifier)
        item.target = self
        if identifier.rawValue == "yu.sidebar" {
            item.label = "侧栏"
            item.image = NSImage(systemSymbolName: "sidebar.left", accessibilityDescription: item.label)
            item.action = #selector(toggleSidebarToolbar(_:))
        } else if identifier.rawValue == "yu.find" {
            item.label = "查找"
            item.image = NSImage(systemSymbolName: "magnifyingglass", accessibilityDescription: item.label)
            item.action = #selector(toggleSearchFromMenu(_:))
        } else if identifier.rawValue == "yu.source" {
            item.label = bridge.sourceMode ? "即时预览" : "源码模式"
            item.toolTip = "切换 Markdown 源码与即时预览"
            item.image = NSImage(systemSymbolName: bridge.sourceMode ? "doc.richtext" : "chevron.left.forwardslash.chevron.right", accessibilityDescription: item.label)
            item.action = #selector(toggleSourceMode(_:))
        } else { return nil }
        return item
    }

    @objc fileprivate func toggleSourceMode(_ sender: Any?) {
        guard !textView.hasMarkedText() else { NSSound.beep(); return }
        do { try setSourceMode(!bridge.sourceMode) } catch { show(error) }
    }

    private func setSourceMode(_ enabled: Bool) throws {
        surfaceCoordinator.retainPositionForPresentationChange()
        try bridge.setSourceMode(enabled)
        surfaceCoordinator.resetTableResizeAfterDocumentChange()
        textView.refreshFromRust()
        textView.refreshTableResizeAccessibility()
        refreshOutline(force: true)
        view.needsLayout = true
        for item in view.window?.toolbar?.items ?? [] where item.itemIdentifier.rawValue == "yu.source" {
            item.label = enabled ? "即时预览" : "源码模式"
            item.image = NSImage(systemSymbolName: enabled ? "doc.richtext" : "chevron.left.forwardslash.chevron.right", accessibilityDescription: item.label)
        }
        scheduleVisualSubmit()
        view.window?.invalidateRestorableState()
        focusDocument()
    }

    @objc private func toggleSidebarToolbar(_ sender: Any?) {
        sidebarHidden.toggle()
        updateSidebarVisibility()
        focusDocument()
    }

    @objc private func selectReadingTheme(_ sender: NSMenuItem) {
        NativeTheme.selection = NativeTheme.Selection(rawValue: sender.tag) ?? .yu
        NotificationCenter.default.post(name: NativeTheme.didChange, object: nil)
    }

    @objc fileprivate func zoomInFromMenu(_ sender: Any?) { setReadingZoom(readingZoom + 0.125) }
    @objc fileprivate func zoomOutFromMenu(_ sender: Any?) { setReadingZoom(readingZoom - 0.125) }
    @objc fileprivate func resetZoomFromMenu(_ sender: Any?) { setReadingZoom(1) }

    private func setReadingZoom(_ zoom: CGFloat) {
        guard zoom.isFinite else { return }
        let next = min(3, max(0.5, zoom))
        guard abs(next - readingZoom) > 0.001 else { return }
        surfaceCoordinator.retainPositionForPresentationChange()
        readingZoom = next
        let theme = NativeTheme.spec(dark: view.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua)
        let size = CGFloat(NativeWritingPreferences.shared.fontSize) * next
        textView.font = NativeTheme.font(identity: theme.body_font, size: size)
        surfaceCoordinator.setFontSize(size)
        textView.refreshTableResizeAccessibility()
        view.needsLayout = true
        scheduleVisualSubmit()
        view.window?.invalidateRestorableState()
    }

    func encodeWindowState(to coder: NSCoder) {
        coder.encode(bridge.path as NSString, forKey: "Yu.documentPath")
        coder.encode(sidebarHidden, forKey: "Yu.sidebarHidden")
        coder.encode(sidebarTabs.selectedSegment, forKey: "Yu.sidebarPanel")
        coder.encode(bridge.sourceMode, forKey: "Yu.sourceMode")
        coder.encode(Double(preferredSidebarWidth), forKey: "Yu.sidebarWidth")
        coder.encode(Double(readingZoom), forKey: "Yu.readingZoom")
        let selection = bridge.selectionEndpoints
        coder.encode(Int64(selection.anchorUTF16), forKey: "Yu.selectionAnchor")
        coder.encode(Int64(selection.focusUTF16), forKey: "Yu.selectionFocus")
        coder.encode(Int(selection.affinity), forKey: "Yu.selectionAffinity")
        surfaceCoordinator.encodeReadingPosition(to: coder)
    }

    func restoreWindowState(from coder: NSCoder) {
        _ = view
        if coder.containsValue(forKey: "Yu.readingZoom") {
            setReadingZoom(CGFloat(coder.decodeDouble(forKey: "Yu.readingZoom")))
        }
        if coder.containsValue(forKey: "Yu.sidebarWidth") {
            let width = coder.decodeDouble(forKey: "Yu.sidebarWidth")
            if width.isFinite && (180...400).contains(width) { preferredSidebarWidth = width }
        }
        sidebarHidden = coder.decodeBool(forKey: "Yu.sidebarHidden")
        sidebarTabs.selectedSegment = min(1, max(0, coder.decodeInteger(forKey: "Yu.sidebarPanel")))
        updateSidebarVisibility()
        if coder.containsValue(forKey: "Yu.sourceMode") {
            do { try setSourceMode(coder.decodeBool(forKey: "Yu.sourceMode")) } catch { show(error) }
        }
        if coder.containsValue(forKey: "Yu.selectionAnchor"), coder.containsValue(forKey: "Yu.selectionFocus") {
            let anchor = coder.decodeInt64(forKey: "Yu.selectionAnchor")
            let focus = coder.decodeInt64(forKey: "Yu.selectionFocus")
            let affinity = coder.decodeInteger(forKey: "Yu.selectionAffinity")
            let count = (bridge.source as NSString).length
            if anchor >= 0 && focus >= 0 && anchor <= count && focus <= count && (0...1).contains(affinity) {
                try? bridge.setSelectionEndpoints(anchorUTF16: UInt64(anchor), focusUTF16: UInt64(focus), affinity: UInt8(affinity))
                textView.refreshFromRust()
            }
        }
        if visualEnhancementsReady { documentSplitView?.setPosition(preferredSidebarWidth, ofDividerAt: 0) }
        view.layoutSubtreeIfNeeded()
        surfaceCoordinator.restoreReadingPosition(from: coder)
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


    @discardableResult
    fileprivate func saveDocument() -> Bool {
        do {
            try textView.finishCompositionForFileOperation()
            if persistence.isUntitled { return chooseSaveDestination() }
            try persistence.save()
            updateStatus()
            return true
        } catch { show(error); return false }
    }

    @objc private func save() { _ = saveDocument() }

    @objc fileprivate func insertImageFromMenu(_ sender: Any?) {
        let panel = NSOpenPanel()
        panel.title = "插入图片"
        panel.allowedContentTypes = [.image]
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        guard panel.runModal() == .OK, !panel.urls.isEmpty else { return }
        do { _ = try importImages(panel.urls.map { .file($0) }) } catch { show(error) }
    }

    private func importImages(_ inputs: [NativeImageResources.Input], at dropTarget: NativeSelectionEndpoints? = nil) throws -> Bool {
        guard !inputs.isEmpty else { return false }
        try NativeImageResources.validate(inputs)
        try textView.finishCompositionForFileOperation()
        // A cancelled first save consumes the clipboard command but does not
        // publish source, copy resources or report a successful drag.
        if persistence.isUntitled, !saveDocument() { return false }
        try NativeImageResources.withImports(inputs, document: documentURL,
            directory: NativeWritingPreferences.shared.imageDirectory,
            reference: NativeWritingPreferences.shared.imagePolicy == .reference) { images in
                try textView.insertImportedImages(images, at: dropTarget)
            }
        focusDocument()
        return true
    }

    @objc fileprivate func saveAsFromMenu(_ sender: Any?) {
        do { try textView.finishCompositionForFileOperation() }
        catch { show(error); return }
        _ = chooseSaveDestination()
    }

    @discardableResult
    private func chooseSaveDestination() -> Bool {
        let panel = NSSavePanel()
        panel.title = persistence.isUntitled ? "保存文档" : "另存为"
        panel.nameFieldStringValue = persistence.isUntitled ? "未命名.md" : documentURL.lastPathComponent
        if !persistence.isUntitled { panel.directoryURL = documentURL.deletingLastPathComponent() }
        panel.allowedContentTypes = [UTType(filenameExtension: "md") ?? .plainText, .plainText]
        panel.canCreateDirectories = true
        let destination: URL?
        if let savePanelDecision { destination = savePanelDecision(panel) }
        else { destination = panel.runModal() == .OK ? panel.url : nil }
        guard let url = destination else { return false }
        do {
            try saveDocumentAs(to: url, replaceExisting: true)
            return true
        } catch { show(error); return false }
    }

    /// NSSavePanel supplies overwrite consent; programmatic checks pass it
    /// explicitly. The AppDelegate rejects destinations owned by other windows.
    func saveDocumentAs(to url: URL, replaceExisting: Bool) throws {
        try onValidateSaveDestination?(url)
        try textView.finishCompositionForFileOperation()
        surfaceCoordinator.detach()
        defer { scheduleVisualSubmit() }
        try persistence.saveAs(url, replaceExisting: replaceExisting)
        view.window?.representedURL = documentURL
        view.window?.title = documentURL.lastPathComponent
        view.window?.identifier = NSUserInterfaceItemIdentifier(bridge.path)
        view.window?.invalidateRestorableState()
        fileWatcher = nil
        filePanel.setDirectory(documentURL.deletingLastPathComponent())
        startFileWatcher()
        refreshFromRust()
        onDocumentURLChange?()
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

    private var settingsWindowIsKey: Bool { NSApp.keyWindow?.identifier?.rawValue == "yu-settings" }

    @objc fileprivate func closeFromMenu(_ sender: Any?) {
        (settingsWindowIsKey ? NSApp.keyWindow : view.window)?.performClose(sender)
    }

    func focusDocument() {
        _ = view.window?.makeFirstResponder(textView)
    }

    @MainActor
    func runWindowStateSelfCheck() async throws {
        func require(_ value: Bool, _ message: String) throws {
            if !value { throw NSError(domain: "YuWindowState", code: 1, userInfo: [NSLocalizedDescriptionKey: message]) }
        }
        func settle(_ controller: DocumentViewController) async throws {
            let deadline = Date().addingTimeInterval(25)
            while Date() < deadline {
                controller.view.layoutSubtreeIfNeeded()
                _ = try controller.surfaceCoordinator.submitNow()
                if controller.surfaceCoordinator.hasCurrentFrame(requirePresented: true),
                   controller.surfaceCoordinator.lastSnapshot?.layoutPending == false { return }
                try await Task.sleep(nanoseconds: 20_000_000)
            }
            throw NSError(domain: "YuWindowState", code: 2, userInfo: [NSLocalizedDescriptionKey: "Window did not settle: source=\(controller.bridge.sourceMode) current=\(controller.surfaceCoordinator.hasCurrentFrame()) presented=\(controller.surfaceCoordinator.hasCurrentFrame(requirePresented: true)) pending=\(String(describing: controller.surfaceCoordinator.lastSnapshot?.layoutPending)) clip=\(controller.documentScrollView?.contentView.bounds ?? .zero) window=\(controller.view.window != nil)"])
        }
        let original = bridge.source
        let disk = try Data(contentsOf: URL(fileURLWithPath: bridge.path))
        let range = (original as NSString).range(of: "恢复")
        try require(range.location != NSNotFound, "Restoration fixture needs 恢复")
        for sourceMode in [false, true] {
            print("Yu restoration stage: original mode=\(sourceMode)"); fflush(stdout)
            try setSourceMode(sourceMode)
            sidebarHidden = false
            sidebarTabs.selectedSegment = 0
            updateSidebarVisibility()
            documentSplitView?.setPosition(278, ofDividerAt: 0)
            try bridge.setSelectionEndpoints(anchorUTF16: UInt64(NSMaxRange(range)), focusUTF16: UInt64(range.location))
            textView.refreshFromRust()
            try await settle(self)
            guard let scroll = documentScrollView else { throw CocoaError(.coderInvalidValue) }
            let maximum = max(0, textView.frame.height - scroll.contentView.bounds.height)
            scroll.contentView.setBoundsOrigin(NSPoint(x: 0, y: maximum * 0.5))
            scroll.reflectScrolledClipView(scroll.contentView)
            try await settle(self)
            let archive = NSKeyedArchiver(requiringSecureCoding: true)
            encodeWindowState(to: archive)
            archive.finishEncoding()
            let data = archive.encodedData
            let expected = try NSKeyedUnarchiver(forReadingFrom: data)
            let source = expected.decodeInt64(forKey: "Yu.readingSource")
            let offset = expected.decodeDouble(forKey: "Yu.readingOffset")
            expected.finishDecoding()
            try require(source > 0, "Archive lost scrolled source anchor")

            let restored = DocumentViewController(bridge: try StorageBridge(path: bridge.path))
            print("Yu restoration stage: decode before attach"); fflush(stdout)
            // Decode before attaching a window: AppKit may restore before geometry exists.
            let decoder = try NSKeyedUnarchiver(forReadingFrom: data)
            restored.restoreWindowState(from: decoder)
            decoder.finishDecoding()
            let window = NSWindow(contentViewController: restored)
            window.styleMask = [.titled, .closable, .resizable, .fullSizeContentView]
            window.setContentSize(NSSize(width: 1040, height: 720))
            window.isReleasedWhenClosed = false
            window.isRestorable = false
            window.appearance = view.window?.appearance
            window.makeKeyAndOrderFront(nil)
            defer { restored.detachSurfaceHost(); window.close() }
            try await settle(restored)
            try require(restored.bridge.sourceMode == sourceMode, "Source mode was not restored")
            try require(!restored.sidebarHidden && restored.sidebarTabs.selectedSegment == 0, "Sidebar panel was not restored")
            try require(abs((restored.sidebarContainer?.frame.width ?? 0) - 278) < 1, "Sidebar width was overwritten during initial layout")
            let selection = restored.bridge.selectionEndpoints
            try require(selection.anchorUTF16 == UInt64(NSMaxRange(range)) && selection.focusUTF16 == UInt64(range.location), "Backward selection was not restored")
            let width = Float(max(restored.textView.bounds.width - 2 * restored.textView.contentOrigin.x, 1))
            let caret = try restored.bridge.sourceCaret(revision: restored.bridge.revision, sourceUTF16: UInt64(source), affinity: 0, size: 16, maxWidth: width)
            let top = Double(restored.textView.contentOrigin.y)
            let actual = Double(restored.documentScrollView?.contentView.bounds.minY ?? 0)
            try require(abs(Double(caret.point.y) + offset + top - actual) < 1, "Source anchor drifted after width change: actual=\(actual) target=\(Double(caret.point.y) + offset + top)")

            restored.setSearchPanelHidden(false)
            restored.view.layoutSubtreeIfNeeded()
            let field = restored.searchPanel.focusTarget as? NSControl
            try require(window.firstResponder === field?.currentEditor() || window.firstResponder === restored.searchPanel.focusTarget, "Find field did not receive keyboard focus")
            restored.setSearchPanelHidden(true)
            try require(window.firstResponder === restored.textView, "Closing Find did not restore editor focus")
            try require(restored.textView.accessibilityNumberOfCharacters() == (original as NSString).length, "AX character count diverged")
            try require(restored.textView.accessibilitySelectedText() == "恢复", "AX selected text diverged")
            try require((restored.textView.accessibilityValue() as? String) == original, "AX value diverged from source")
            try require(restored.bridge.source == original && bridge.source == original, "Window restoration changed document bytes")
            print("Yu window state self-check: mode=\(sourceMode ? "source" : "preview") secureArchive=true preAttachDecode=true sidebar=278 backwardSelection=true sourceAnchor=within-1pt findFocus=true axValue=true")
        }
        try require(try Data(contentsOf: URL(fileURLWithPath: bridge.path)) == disk, "Restoration wrote the file")
        print("Yu window state self-check passed: disk unchanged; real VoiceOver and OS relaunch remain separate acceptance")
    }

    @MainActor
    func runPresentationLatencySelfCheck(zoom: Bool, redraw: Bool = false) async throws {
        func require(_ condition: Bool, _ message: String) throws {
            if !condition { throw NSError(domain: "YuPresentationLatency", code: 1,
                userInfo: [NSLocalizedDescriptionKey: message]) }
        }
        guard let window = view.window else { throw CocoaError(.coderInvalidValue) }
        let original = bridge.source
        let originalBytes = try Data(contentsOf: URL(fileURLWithPath: bridge.path))
        let anchor = (original as NSString).range(of: "Latency anchor: ")
        try require(anchor.location != NSNotFound, "Missing benchmark anchor")
        sidebarHidden = true
        updateSidebarVisibility()
        window.appearance = NSAppearance(named: .aqua)
        window.setContentSize(NSSize(width: 1200, height: 800))
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        focusDocument()
        textView.font = NSFont.systemFont(ofSize: 16)
        surfaceCoordinator.setFontSize(16)
        textView.navigate(toSource: NSRange(location: NSMaxRange(anchor), length: 0))
        view.layoutSubtreeIfNeeded()
        let deadline = CACurrentMediaTime() + 90
        while CACurrentMediaTime() < deadline {
            if surfaceCoordinator.currentPresentationTime() != nil,
               surfaceCoordinator.lastSnapshot?.layoutPending == false { break }
            try await Task.sleep(nanoseconds: 5_000_000)
        }
        try require(surfaceCoordinator.currentPresentationTime() != nil
            && surfaceCoordinator.lastSnapshot?.layoutPending == false, "Initial layout did not settle")
        func active() -> Bool {
            NSApp.isActive && !NSApp.isHidden && window.isKeyWindow
                && window.isVisible && window.occlusionState.contains(.visible)
        }
        // Long initial measurement can outlast foreground ownership. Establish
        // the test's input context immediately before sampling, not 30s earlier.
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        focusDocument()
        let focusDeadline = CACurrentMediaTime() + 3
        while !active(), CACurrentMediaTime() < focusDeadline {
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        try require(active(), "Benchmark window is not active and visible")
        try require(bridge.source == original, "Source changed before sampling: \(bridge.source.prefix(100))")
        textView.navigate(toSource: NSRange(location: NSMaxRange(anchor), length: 0))
        // Diagnostic-only display-cycle observation; clean benchmarks do not
        // start a continuous display link or change the production scheduler.
        let cycleTrace = DisplayLinkPacer(traceCycles: true) {}
        if ProcessInfo.processInfo.environment["YU_RENDER_TIMING"] != nil {
            cycleTrace.start(view: surfaceHostView)
        }
        defer { cycleTrace.stop() }
        var samples = [[String: Any]]()
        func sample(_ kind: String, index: Int, action: () throws -> Void) async throws {
            let activeBefore = active()
            let started = CACurrentMediaTime()
            try action()
            let actionEnd = CACurrentMediaTime()
            let revision = bridge.revision
            let timeout = started + 20
            var presented: Double?
            while CACurrentMediaTime() < timeout {
                if let time = surfaceCoordinator.currentPresentationTime(), time >= started {
                    presented = time; break
                }
                // Observe the production scheduler; do not force extra frames.
                try await Task.sleep(nanoseconds: 1_000_000)
            }
            guard let presented else {
                throw NSError(domain: "YuPresentationLatency", code: 2,
                    userInfo: [NSLocalizedDescriptionKey: "No matching drawable presentation: \(kind) \(index)"])
            }
            let sample: [String: Any] = ["kind": kind, "index": index, "revision": revision,
                "start_time_s": started, "presented_time_s": presented,
                "action_ms": (actionEnd - started) * 1000,
                "presented_ms": (presented - started) * 1000,
                "observed_ms": (CACurrentMediaTime() - started) * 1000,
                "active_before": activeBefore, "active_after": active(),
                "app_active": NSApp.isActive, "key_window": window.isKeyWindow,
                "visible": window.isVisible, "unoccluded": window.occlusionState.contains(.visible),
                "frame_serial": surfaceCoordinator.lastSnapshot?.frameSerial ?? 0]
            samples.append(sample)
            let encoded = try JSONSerialization.data(withJSONObject: sample, options: [.sortedKeys])
            print("Yu latency sample: \(String(decoding: encoded, as: UTF8.self))"); fflush(stdout)
        }
        if redraw {
            let revision = bridge.revision
            for index in 0..<24 {
                try await sample("redraw", index: index) {
                    _ = try surfaceCoordinator.submitNow(force: true)
                }
                try require(bridge.revision == revision && bridge.source == original,
                    "Retained redraw changed source or revision")
            }
        } else if zoom {
            let sizes: [CGFloat] = [18, 14, 20, 16]
            for index in 0..<24 {
                try await sample("zoom", index: index) {
                    let size = sizes[index % sizes.count]
                    textView.font = NSFont.systemFont(ofSize: size)
                    surfaceCoordinator.setFontSize(size)
                }
                try require(bridge.source == original, "Zoom changed source")
            }
        } else {
            let values = ["羽", "e\u{301}", "🙂", "abc"]
            for index in 0..<24 {
                let inserted = values[index % values.count]
                let target = bridge.selection.range
                try require(target == NSRange(location: NSMaxRange(anchor), length: 0), "Input selection moved: \(target)")
                try await sample("insert", index: index) {
                    textView.insertText(inserted, replacementRange: NSRange(location: NSNotFound, length: 0))
                }
                let expected = (original as NSString).replacingCharacters(in: NSRange(location: NSMaxRange(anchor), length: 0), with: inserted)
                try require(bridge.source == expected, "Insert source mismatch: actual=\(bridge.source.prefix(100)) expected=\(expected.prefix(100))")
                try await sample("undo", index: index) { textView.performUndo() }
                try require(bridge.source == original, "Undo source mismatch")
            }
        }
        try bridge.save()
        try require(try Data(contentsOf: URL(fileURLWithPath: bridge.path)) == originalBytes, "Saved bytes changed")
        let result: [String: Any] = ["mode": redraw ? "redraw" : (zoom ? "zoom" : "input"), "samples": samples,
            "source_bytes": originalBytes.count, "font_size_pt": 16, "theme": NativeTheme.selection.rawValue,
            "content_width_pt": window.contentView?.bounds.width ?? 0,
            "content_height_pt": window.contentView?.bounds.height ?? 0,
            "display_max_fps": window.screen?.maximumFramesPerSecond ?? 0,
            "backing_scale": window.backingScaleFactor, "source_and_save_correct": true,
            "measurement": "Scripted input/zoom/retained redraw to matching drawable presentedTime; redraw is a control, not editing latency"]
        let encoded = try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys])
        if let path = ProcessInfo.processInfo.environment["YU_LATENCY_RESULT_PATH"] {
            // Native timing callbacks also write stdout. Keep the structured
            // record atomic rather than interleaving a long JSON line.
            try encoded.write(to: URL(fileURLWithPath: path), options: .atomic)
            print("Yu presentation latency result written")
        } else {
            print("Yu presentation latency result: \(String(decoding: encoded, as: UTF8.self))")
        }
    }

    @MainActor
    func runIdleResourceSelfCheck() async throws {
        func cpuSeconds() -> Double {
            var usage = rusage()
            getrusage(RUSAGE_SELF, &usage)
            return Double(usage.ru_utime.tv_sec + usage.ru_stime.tv_sec)
                + Double(usage.ru_utime.tv_usec + usage.ru_stime.tv_usec) / 1_000_000
        }
        func footprint() throws -> UInt64 {
            var info = task_vm_info_data_t()
            var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<integer_t>.size)
            let result = withUnsafeMutablePointer(to: &info) { pointer in
                pointer.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                    task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
                }
            }
            guard result == KERN_SUCCESS else { throw NSError(domain: NSMachErrorDomain, code: Int(result)) }
            return info.phys_footprint
        }
        let layoutStarted = ProcessInfo.processInfo.systemUptime
        let deadline = Date().addingTimeInterval(90)
        while Date() < deadline {
            _ = try surfaceCoordinator.submitNow()
            if surfaceCoordinator.hasCurrentFrame(requirePresented: true), surfaceCoordinator.lastSnapshot?.layoutPending == false { break }
            try await Task.sleep(nanoseconds: 50_000_000)
        }
        guard surfaceCoordinator.hasCurrentFrame(requirePresented: true), surfaceCoordinator.lastSnapshot?.layoutPending == false else {
            throw NSError(domain: "YuIdleCheck", code: 1, userInfo: [NSLocalizedDescriptionKey: "Initial document layout did not settle"])
        }
        let layoutSeconds = ProcessInfo.processInfo.systemUptime - layoutStarted
        // Hiding is deterministic and avoids activating another user app.
        // Record this narrower scenario rather than calling it visible-idle.
        NSApp.hide(nil)
        try await Task.sleep(nanoseconds: 5_000_000_000)
        let startCPU = cpuSeconds()
        let startTime = ProcessInfo.processInfo.systemUptime
        let startSerial = surfaceCoordinator.lastSnapshot?.frameSerial ?? 0
        var samples: [UInt64] = [try footprint()]
        var activationSamples = [["active": NSApp.isActive, "key": view.window?.isKeyWindow ?? false,
                                  "visible": view.window?.isVisible ?? false, "hidden": NSApp.isHidden]]
        var remainedInactive = !NSApp.isActive && !(view.window?.isKeyWindow ?? true)
        // isVisible describes window ordering; an application-hidden window
        // can retain that flag. NSApp.isHidden is the authoritative condition.
        var remainedHidden = NSApp.isHidden
        for _ in 0..<6 {
            try await Task.sleep(nanoseconds: 10_000_000_000)
            samples.append(try footprint())
            activationSamples.append(["active": NSApp.isActive, "key": view.window?.isKeyWindow ?? false,
                                      "visible": view.window?.isVisible ?? false, "hidden": NSApp.isHidden])
            remainedInactive = remainedInactive && !NSApp.isActive && !(view.window?.isKeyWindow ?? true)
            remainedHidden = remainedHidden && NSApp.isHidden
        }
        let wall = ProcessInfo.processInfo.systemUptime - startTime
        let cpu = (cpuSeconds() - startCPU) / wall * 100
        let result: [String: Any] = ["source_bytes": bridge.source.utf8.count, "wall_seconds": wall,
            "scenario": "hidden-inactive", "initial_layout_seconds": layoutSeconds, "activation_samples": activationSamples,
            "cpu_percent_one_core": cpu, "physical_footprint_bytes": samples,
            "remained_inactive": remainedInactive, "remained_hidden": remainedHidden, "window_visible": view.window?.isVisible ?? false,
            "start_frame_serial": startSerial, "end_frame_serial": surfaceCoordinator.lastSnapshot?.frameSerial ?? 0,
            "cpu_target_passed": remainedInactive && cpu <= 0.5]
        let data = try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys])
        print("Yu idle resource measurement: \(String(decoding: data, as: UTF8.self))")
    }

    /// 真实窗口下的帧调度自检。
    ///
    /// 「这一帧是否等价于屏幕上那一帧」的判断已经移入 Rust。判断漏掉一项不会
    /// 报错，只会让画面停住——光标不动、preedit 不更新、拖动中的列宽不动，
    /// 三者都表现为「编辑器卡了」而没有任何日志。headless self-check 覆盖不到
    /// 这条路径：它需要真实的 NSWindow 与 Metal surface 才会有「已提交的帧」。
    ///
    /// 反向验证：把 `MacosFrameKey` 的 `selection` 去掉，第 3 步失败。
    @MainActor
    func runFrameSchedulingSelfCheck() async throws {
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
        func submit(force: Bool = false) async throws -> NativeMacosRenderHostSurfaceSnapshot? {
            let deadline = Date().addingTimeInterval(3)
            repeat {
                // A scheduled UI submission may win the drawable before this
                // task resumes. Accept that exact current, submitted frame;
                // forcing another draw can otherwise starve this waiter.
                if surfaceCoordinator.hasCurrentFrame(),
                   let published = surfaceCoordinator.lastSnapshot {
                    return published
                }
                if let snapshot = try surfaceCoordinator.submitNow(force: force) {
                    return snapshot
                }
                try await Task.sleep(nanoseconds: 8_000_000)
            } while Date() < deadline
            throw Failure(message: "Timed out waiting for drawable presentation; root=\(view.bounds) split=\(documentSplitView?.frame ?? .zero) clip=\(documentScrollView?.contentView.bounds ?? .zero) surface=\(surfaceHostView.frame) visible=\(view.window?.occlusionState.contains(.visible) ?? false)")
        }

        // Stage Manager can report an active/key window while its compositor
        // image is still a thumbnail. Wait for native pixels, never resize them.
        func captureEditingImage() async throws -> CGImage {
            guard let window = view.window else { throw Failure(message: "编辑截图没有窗口") }
            let deadline = Date().addingTimeInterval(5)
            var detail = "compositor returned no image"
            var lastResubmit = Date()
            var resubmits = 0
            repeat {
                if !NSApp.isActive || !window.isKeyWindow {
                    window.makeKeyAndOrderFront(nil)
                    NSApp.activate(ignoringOtherApps: true)
                }
                guard surfaceCoordinator.hasCurrentFrame(requirePresented: true) else {
                    detail = "latest document frame has not been presented"
                    if resubmits < 3, Date().timeIntervalSince(lastResubmit) >= 1 {
                        _ = try surfaceCoordinator.submitNow(force: true)
                        resubmits += 1
                        lastResubmit = Date()
                    }
                    try await Task.sleep(nanoseconds: 100_000_000)
                    continue
                }
                if NSApp.isActive, window.isVisible, !window.isMiniaturized,
                   let image = try? await NativeWindowCapture.image(window) {
                    let width = Int((window.frame.width * window.backingScaleFactor).rounded())
                    let height = Int((window.frame.height * window.backingScaleFactor).rounded())
                    if image.width == width && image.height == height { return image }
                    detail = "pixels \(image.width)×\(image.height), expected \(width)×\(height)"
                }
                try await Task.sleep(nanoseconds: 100_000_000)
            } while Date() < deadline
            throw Failure(message: "编辑原尺寸截图超时：\(detail)")
        }

        // 1. 真实 surface 上必须先有一帧。
        let snapshot = try await submit(force: true)
        try require(snapshot?.submitted == true, "首帧未提交")
        try require((snapshot?.commandCount ?? 0) > 0, "首帧没有任何绘制指令")

        // 2. 状态没变时必须判为等价，否则每一次布局回调都会整帧重画。
        try require(surfaceCoordinator.hasCurrentFrame(), "刚提交的帧未被判为当前帧")

        // Exercise the actual toolbar target and shared frame invalidation.
        let modeSource = bridge.source
        let modeRevision = bridge.revision
        let modeSelection = bridge.selection.range
        toggleSourceMode(nil)
        try require(bridge.sourceMode, "源码模式入口未切换; marked=\(textView.hasMarkedText()) composition=\(bridge.composition.active)")
        try require(!surfaceCoordinator.hasCurrentFrame(), "源码模式仍复用预览帧")
        let literalFrame = try await submit(force: true)
        try require(literalFrame?.submitted == true && (literalFrame?.commandCount ?? 0) > 0,
                    "源码帧未提交")
        let literalStart = try require(textView.shapedCaretRectForSelfCheck(sourceUTF16: 0), "缺少源码行首几何")
        let afterMarker = try require(textView.shapedCaretRectForSelfCheck(sourceUTF16: 1), "缺少源码标记后几何")
        try require(afterMarker.minX > literalStart.minX + 1 && abs(literalStart.height - 26) < 0.5,
                    "源码首字符仍隐藏或沿用标题行高: \(literalStart) → \(afterMarker)")
        var literalRange = NSRange(location: NSNotFound, length: 0)
        let literalScreen = textView.firstRect(forCharacterRange: NSRange(location: 1, length: 0), actualRange: &literalRange)
        let literalWindow = try require(textView.window, "源码输入没有窗口")
        let literalLocal = textView.convert(literalWindow.convertFromScreen(literalScreen), from: nil)
        try require(literalRange.location == 1 && abs(literalLocal.minX - afterMarker.minX - textView.contentOrigin.x) < 0.5
                    && abs(literalLocal.minY - afterMarker.minY - textView.contentOrigin.y) < 0.5,
                    "源码候选框未使用绘制几何")
        try require(bridge.source == modeSource && bridge.revision == modeRevision && bridge.selection.range == modeSelection,
                    "源码切换改变了文档或选区")
        if let directory = ProcessInfo.processInfo.environment["YU_SOURCE_MODE_CAPTURE_DIR"] {
            let image = try await captureEditingImage()
            let url = URL(fileURLWithPath: directory)
            try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
            let name = view.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua ? "source-dark.png" : "source-light.png"
            let png = try require(NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]), "源码截图编码失败")
            try png.write(to: url.appendingPathComponent(name), options: .atomic)
        }
        toggleSourceMode(nil)
        _ = try await submit(force: true)
        try require(!bridge.sourceMode && bridge.source == modeSource && bridge.revision == modeRevision,
                    "返回预览改变了源码")
        print("Yu source mode window self-check: toolbar toggle, literal glyphs, frame invalidation and unchanged source/selection passed")

        // 3. 光标移动不推进 Revision，但必须让帧失效。
        let sourceLength = bridge.source.utf16.count
        try require(sourceLength > 4, "fixture 太短，无法移动光标")
        let before = bridge.selection
        try bridge.setSelection(NSRange(location: 3, length: 0))
        try require(bridge.revision == before.revision, "移动光标不应推进 Revision")
        try require(
            !surfaceCoordinator.hasCurrentFrame(),
            "光标移动后仍被判为当前帧——caret 会停在原处且不会报错"
        )

        // 4. 重新提交之后必须再次等价。
        let republished = try await submit()
        try require(republished?.submitted == true, "光标移动后的重提交失败")
        try require(surfaceCoordinator.hasCurrentFrame(), "重提交后未恢复为当前帧")

        // Native candidate rectangles must apply document -> view -> screen
        // conversion once, including a caret in a later paragraph.
        let inputWidth = textView.bounds.width
        let viewportWidth = documentScrollView?.contentView.bounds.width ?? 0
        try require(abs(inputWidth - viewportWidth) < 0.5,
                    "输入宿主宽度 \(inputWidth) 与阅读视口 \(viewportWidth) 不一致")
        let candidateOffset = max(sourceLength - 1, 0)
        let candidate = try require(
            textView.shapedCaretRectForSelfCheck(sourceUTF16: candidateOffset),
            "缺少文档尾部光标几何"
        )
        var actualRange = NSRange(location: NSNotFound, length: 0)
        let screenRect = textView.firstRect(
            forCharacterRange: NSRange(location: candidateOffset, length: 0),
            actualRange: &actualRange
        )
        let nativeWindow = try require(textView.window, "缺少输入窗口")
        let returnedRect = textView.convert(nativeWindow.convertFromScreen(screenRect), from: nil)
        try require(actualRange.location == candidateOffset, "输入法矩形返回了错误源码范围")
        try require(abs(returnedRect.minY - candidate.minY - textView.contentOrigin.y) < 0.5,
                    "输入法矩形的文档原点或滚动转换不一致")
        try require(abs(returnedRect.minX - candidate.minX - textView.contentOrigin.x) < 0.5,
                    "输入法矩形的阅读列转换不一致")
        try require(abs(returnedRect.height - candidate.height) < 0.5, "输入法矩形丢失真实行高")

        // 5. 滚动范围包含 Rust 文档高度及阅读区上下留白。
        //    留白属于同一坐标转换，必须能够滚到，不能把它裁在 document view 外。
        let contentHeight = republished?.contentHeight ?? 0.0
        try require(contentHeight > 0.0, "帧未报告内容高度")
        guard let scrollView = documentScrollView,
              let documentView = scrollView.documentView else {
            throw Failure(message: "没有可滚动的 document view")
        }
        let theme = NativeTheme.spec(dark: documentView.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua)
        let expectedExtent = max(contentHeight + CGFloat(theme.top + theme.bottom), scrollView.contentView.bounds.height)
        try require(
            abs(documentView.frame.height - expectedExtent) <= 0.5,
            "可滚动范围 \(documentView.frame.height) 不等于正文与留白的总高度 \(expectedExtent)"
        )

        try verifyScrollExtentForSelfCheck(scrollView)

        // 小幅滚动应当复用已发布的 retained coverage，只改变呈现视口，
        // 而不是再次触发完整 Markdown/CoreText frame build。
        let scrollRange = max(documentView.frame.height - scrollView.contentView.bounds.height, 0.0)
        if scrollRange > 1.0 {
            var origin = scrollView.contentView.bounds.origin
            origin.y = min(scrollRange, max(20.0, scrollView.contentView.bounds.height * 0.25))
            scrollView.contentView.setBoundsOrigin(origin)
            scrollView.reflectScrolledClipView(scrollView.contentView)
            let retained = try require(
                await submit(),
                "retained scroll frame 提交失败"
            )
            try require(
                retained.presentationReused,
                "coverage 内滚动没有复用 retained frame"
            )
        }

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
        let baseline = try require(await submit(), "滚回文首之后的重提交失败")
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
        let searched = try require(await submit(), "换查询之后的重提交失败")
        try require(
            searched.searchDecorationCount > 0,
            "查询「\(needle)」在场景里没有画出任何高亮"
        )

        // 8. 收掉搜索，矩形必须一起消失。
        try require(bridge.setSearchQuery(nil), "收掉搜索失败")
        let cleared = try require(await submit(), "收掉搜索之后的重提交失败")
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
        let oneCaret = try require(await submit(force: true), "单光标帧提交失败")
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
        let twoCarets = try require(await submit(), "双光标帧提交失败")
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
        let selectedAll = try require(await submit(), "全部选中之后的重提交失败")
        try require(
            selectedAll.selectionDecorationCount >= allMatches.count,
            "\(allMatches.count) 处匹配只画出了 \(selectedAll.selectionDecorationCount) 块选区底色"
        )
        try require(bridge.setSearchQuery(nil), "收掉搜索失败")
        textView.navigate(toSource: NSRange(location: 0, length: 0))
        let highlightFrame = try require(
            await submit(),
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

        if let window = textView.window {
            let original = window.frame
            var resized = original
            resized.size.width += 80
            resized.size.height += 40
            window.setFrame(resized, display: true)
            window.contentView?.layoutSubtreeIfNeeded()
            let resizedFrame = try require(await submit(), "resize presentation failed")
            try require(resizedFrame.submitted, "resize did not submit")
            try require(
                resizedFrame.surfaceGeneration > highlightFrame.surfaceGeneration,
                "resize did not advance surface generation"
            )
            window.setFrame(original, display: true)
            window.contentView?.layoutSubtreeIfNeeded()
            let restored = try require(await submit(), "restored presentation failed")
            try require(
                restored.surfaceGeneration > resizedFrame.surfaceGeneration,
                "restoring window size did not advance surface generation"
            )
            surfaceCoordinator.detach()
            try require(!surfaceCoordinator.hasCurrentFrame(), "detach retained a current frame")
            let rebound = try require(await submit(), "reattachment after resize failed")
            try require(rebound.submitted, "reattachment did not submit")
            try require(rebound.frameSerial > restored.frameSerial, "reattachment reused a stale publication serial")
            try require(surfaceCoordinator.hasCurrentFrame(), "reattachment did not restore current frame")
        }

        try await surfaceCoordinator.verifyLayoutCoordinatorForSelfCheck()
        let bidiRange = (bridge.source as NSString).range(of: "אבג def דהו xyz")
        if bidiRange.location != NSNotFound {
            let originalSource = bridge.source
            let savedSelection = bridge.selectionEndpoints
            let width = Float(max(textView.bounds.width - 2 * textView.contentOrigin.x, 1))
            let size = Float(textView.font?.pointSize ?? 16)
            let offset = UInt64(bidiRange.location + 3)
            var positions: [CGFloat] = []
            for affinity: UInt8 in [0, 1] {
                let caret = try bridge.sourceCaret(revision: bridge.revision,
                    sourceUTF16: offset, affinity: affinity, size: size, maxWidth: width)
                positions.append(caret.point.x)
                let point = NSPoint(x: caret.point.x, y: caret.point.y + caret.height / 2)
                let hit = try bridge.projectionHitTest(revision: bridge.revision,
                    point: point, size: size, maxWidth: width)
                try require(textView.applyVisualPointerSelectionForSelfCheck(at: point), "双向边界点击失败")
                let selection = bridge.selectionEndpoints
                try require(selection.focusUTF16 == hit.sourceUTF16 && selection.affinity == hit.affinity,
                    "输入宿主丢失双向命中的源码位置或 affinity")
                var actual = NSRange(location: NSNotFound, length: 0)
                let rect = textView.firstRect(forCharacterRange: NSRange(location: Int(selection.focusUTF16), length: 0), actualRange: &actual)
                let local = textView.convert(nativeWindow.convertFromScreen(rect), from: nil)
                try require(abs(local.minX - textView.contentOrigin.x - hit.point.x) < 0.5,
                    "双向边界候选框没有保留选择的 affinity")
                try require(actual.location == Int(selection.focusUTF16), "双向候选框源码范围不一致")
                textView.navigate(toSources: [NSRange(location: 0, length: 0)], primary: 0, affinities: [1])
                try require(textView.addCaretAtVisualPointForSelfCheck(point), "双向边界添加光标失败")
                let selections = try require(bridge.selectionsIfAvailable, "缺少多光标状态")
                try require(selections.ranges.contains { $0.range.location == Int(hit.sourceUTF16) && $0.affinity == hit.affinity },
                    "新光标没有保留双向边界 affinity")
                try require(selections.ranges.contains { $0.range.location == 0 && $0.affinity == 1 },
                    "添加双向光标改变了已有光标 affinity")
            }
            try require(abs(positions[0] - positions[1]) > 1, "语料没有覆盖双向边界的两个位置")
            try require(bridge.source == originalSource, "双向指针检查修改了源码")
            try bridge.setSelectionEndpoints(anchorUTF16: savedSelection.anchorUTF16,
                focusUTF16: savedSelection.focusUTF16, affinity: savedSelection.affinity)
            textView.refreshFromRust()
            print("Yu bidi pointer self-check: primary/secondary hit, source affinity, candidate rectangle and multicaret preserved")
        }

        if bridge.source.contains("Table P1 right") {
            let original = bridge.source
            let originalFont = textView.font
            let baseSize = originalFont?.pointSize ?? 16
            for zoom: CGFloat in [0.75, 1.0, 1.5] {
                let size = baseSize * zoom
                textView.font = NSFont(name: originalFont?.fontName ?? "Open Sans", size: size)
                surfaceCoordinator.setFontSize(size)
                let last = (bridge.source as NSString).range(of: "Table P1 right")
                textView.navigate(toSource: NSRange(location: last.location, length: 0))
                _ = try await submit(force: true)
                let dividers = surfaceCoordinator.tableResizeAccessibilityDividers()
                let divider = try require(dividers.first { $0.kind == UInt8(YU_STORAGE_TABLE_RESIZE_COLUMN) }, "缺少嵌套表格列分隔线")
                let point = NSPoint(x: divider.rect.midX, y: divider.rect.midY)
                try require(surfaceCoordinator.beginTableResize(at: point), "表格拖拽未开始")
                try require(surfaceCoordinator.updateTableResize(at: NSPoint(x: point.x + 18 * zoom, y: point.y)), "表格拖拽未更新")
                try require(surfaceCoordinator.finishTableResize(), "表格拖拽未完成")
                _ = try await submit(force: true)
                let resized = try require(surfaceCoordinator.tableResizeAccessibilityDividers().first {
                    $0.blockIndex == divider.blockIndex && $0.index == divider.index && $0.kind == divider.kind
                }, "拖拽后缺少同一列分隔线")
                try require(resized.rect.midX > divider.rect.midX + 0.5, "拖拽未改变实际发布的列宽几何")
                try require(bridge.source == original, "调整列宽改变了源码")
                if view.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .aqua {
                    try bridge.save()
                    let reopenedWidths = try StorageBridge(path: bridge.path)
                    try require(reopenedWidths.presentationStorageError == nil, "重新打开列宽存储失败")
                    let reopenedSelection = bridge.selectionEndpoints
                    try reopenedWidths.setSelectionEndpoints(anchorUTF16: reopenedSelection.anchorUTF16,
                        focusUTF16: reopenedSelection.focusUTF16, affinity: reopenedSelection.affinity)
                    let reopenedWidth = Float(max(textView.bounds.width - 2 * textView.contentOrigin.x, 1))
                    let reopenedCaret = try reopenedWidths.sourceCaret(revision: reopenedWidths.revision,
                        sourceUTF16: UInt64(last.location), affinity: 1, size: Float(size), maxWidth: reopenedWidth)
                    let reopenedDividers = try reopenedWidths.tableResizeAccessibilityDividers(
                        revision: reopenedWidths.revision, size: Float(size), maxWidth: reopenedWidth,
                        scrollY: Float(max(reopenedCaret.point.y - 100, 0)), viewportHeight: 400)
                    let restoredDivider = try require(reopenedDividers.first {
                        $0.tableSourceRange == resized.tableSourceRange && $0.index == resized.index && $0.kind == resized.kind
                    }, "重新打开后缺少同一列分隔线")
                    try require(abs(restoredDivider.rect.midX - resized.rect.midX) <= 1,
                        "重新打开后列宽几何变化超过 1pt")
                    let restoredHit = try reopenedWidths.tableResizeAtDocumentPoint(
                        revision: reopenedWidths.revision,
                        action: UInt8(YU_STORAGE_TABLE_RESIZE_PROBE),
                        size: Float(size),
                        maxWidth: reopenedWidth,
                        point: CGPoint(x: resized.rect.midX, y: reopenedCaret.point.y + reopenedCaret.height / 2),
                        tolerance: 1
                    )
                    try require(restoredHit.kind == resized.kind && restoredHit.index == resized.index
                        && abs(CGFloat(restoredHit.position) - resized.rect.midX) <= 1,
                        "重新打开后未恢复实际列分隔线位置")
                    try require(reopenedWidths.source == original, "列宽重开改写源码")
                    print("Yu table width persistence self-check: native reopen and divider hit passed zoom=\(zoom)")
                }
                textView.doCommand(by: #selector(NSResponder.insertTab(_:)))
                let appended = bridge.source
                try require(appended != original && appended.hasPrefix(original), "末格 Tab 未追加行或改写旧源码")
                let emptyFirstSource = bridge.selectionEndpoints.focusUTF16
                textView.doCommand(by: #selector(NSResponder.insertTab(_:)))
                let emptyLastSource = bridge.selectionEndpoints.focusUTF16
                _ = try await submit(force: true)
                let emptyWidth = Float(max(textView.bounds.width - 2 * textView.contentOrigin.x, 1))
                let emptyHeader = (appended as NSString).range(of: "Table P1 A")
                let emptyFrom = try bridge.sourceCaret(revision: bridge.revision, sourceUTF16: UInt64(emptyHeader.location), affinity: 1, size: Float(size), maxWidth: emptyWidth)
                let emptyTo = try bridge.sourceCaret(revision: bridge.revision, sourceUTF16: emptyLastSource, affinity: 1, size: Float(size), maxWidth: emptyWidth)
                try require(textView.selectTableCellsAtVisualPoint(NSPoint(x: emptyFrom.point.x + 1, y: emptyFrom.point.y + 1), starting: true), "空格子拖选起点失败")
                _ = try await submit(force: true)
                try require(textView.selectTableCellsAtVisualPoint(NSPoint(x: emptyTo.point.x + 1, y: emptyTo.point.y + 1), starting: false), "空格子拖选终点失败")
                textView.finishTableCellSelection()
                let emptySelectionFrame = try await submit(force: true)
                try require(bridge.tableSelectionColumns == 2 && bridge.selectionsIfAvailable?.ranges.count == 6, "空单元格丢失源码身份")
                try require(emptySelectionFrame?.selectionDecorationCount == 6, "空单元格缺少整格高亮")
                if let directory = ProcessInfo.processInfo.environment["YU_CELL_SELECTION_CAPTURE_DIR"] {
                    try await Task.sleep(nanoseconds: 100_000_000)
                    guard let window = view.window else {
                        throw Failure(message: "矩形选区采集失败：测试视图未绑定窗口")
                    }
                    let image = try await captureEditingImage()
                    guard let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]) else {
                        throw Failure(message: "矩形选区采集失败：PNG 编码失败")
                    }
                    let folder = URL(fileURLWithPath: directory, isDirectory: true)
                    try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
                    let dark = window.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
                    let name = "\(dark ? "night" : "github")-zoom-\(zoom)"
                    try png.write(to: folder.appendingPathComponent(name + ".png"))
                    try appended.write(to: folder.appendingPathComponent(name + ".md"), atomically: true, encoding: .utf8)
                    let metadata: [String: Any] = ["selected_cells": 6, "empty_cells": 2, "columns": 2,
                        "scale": window.backingScaleFactor, "width_pt": window.frame.width, "height_pt": window.frame.height,
                        "pixel_width": image.width, "pixel_height": image.height, "zoom": zoom, "reference_comparison": false]
                    try JSONSerialization.data(withJSONObject: metadata, options: [.prettyPrinted, .sortedKeys]).write(to: folder.appendingPathComponent(name + ".json"))
                }
                textView.navigate(toSource: NSRange(location: Int(emptyFirstSource), length: 0))
                textView.insertText("羽|🪶", replacementRange: NSRange(location: NSNotFound, length: 0))
                let firstEdited = bridge.source
                textView.doCommand(by: #selector(NSResponder.insertTab(_:)))
                let pasteboard = NSPasteboard.withUniqueName()
                defer { pasteboard.releaseGlobally() }
                try require(pasteboard.setString(#"👨‍👩‍👧‍👦\|tail"#, forType: .yuMarkdown), "无法设置表格粘贴语料")
                try textView.pasteFromPasteboardForSelfCheck(pasteboard)
                let edited = bridge.source
                try require(edited == appended.replacingOccurrences(of: "|  |  |", with: #"|羽\|🪶  |👨‍👩‍👧‍👦\|tail  |"#), "新增单元格编辑结果不一致")
                let editedFrame = try await submit(force: true)
                try require(editedFrame?.submitted == true && (editedFrame?.commandCount ?? 0) > 0,
                    "新增表格行未提交绘制")
                let inputWidth = Float(max(textView.bounds.width - 2 * textView.contentOrigin.x, 1))
                let oldRowCaret = try bridge.sourceCaret(revision: bridge.revision,
                    sourceUTF16: UInt64(last.location), affinity: 1, size: Float(size), maxWidth: inputWidth)
                let newRowOffset = UInt64((edited as NSString).range(of: "羽").location)
                let newRowCaret = try bridge.sourceCaret(revision: bridge.revision,
                    sourceUTF16: newRowOffset, affinity: 1, size: Float(size), maxWidth: inputWidth)
                try require(newRowCaret.point.y > oldRowCaret.point.y + size * 0.5,
                    "新增单元格没有排在原行下方")
                for fragment in [#"羽\|🪶"#, #"👨‍👩‍👧‍👦\|tail"#] {
                    let fragmentRange = (edited as NSString).range(of: fragment)
                    let escapeOffset = (fragment as NSString).range(of: #"\|"#).location
                    let from = UInt64(fragmentRange.location + escapeOffset)
                    let beforePipe = try bridge.projectionCaret(revision: bridge.revision, sourceUTF16: from, affinity: 1)
                    let afterPipe = try bridge.projectionCaret(revision: bridge.revision, sourceUTF16: from + 2, affinity: 1)
                    try require(afterPipe.visualUTF16 == beforePipe.visualUTF16 + 1,
                        "转义管道符没有投影成单个可见字符")
                }
                textView.doCommand(by: #selector(NSResponder.insertBacktab(_:)))
                try require(bridge.selectionEndpoints.focusUTF16 == UInt64((edited as NSString).range(of: "羽").location), "Shift-Tab 未返回第一格")
                for expected in [firstEdited, appended, original] {
                    textView.performUndo()
                    try require(bridge.source == expected, "表格连续撤销未逐步恢复源码")
                }
                for expected in [appended, firstEdited, edited] {
                    textView.performRedo()
                    try require(bridge.source == expected, "表格连续重做结果不一致")
                }
                try bridge.save()
                let reopened = try StorageBridge(path: bridge.path)
                try require(reopened.source == edited, "表格保存重新打开不一致")
                for _ in 0..<3 { textView.performUndo() }
                try require(bridge.source == original, "表格自检未恢复原文")
                // Exercise the real native menu action on the visible surface.
                textView.navigate(toSource: NSRange(location: last.location, length: 0))
                let tableMenu = textView.makeTableMenu()
                let insertColumn = try require(tableMenu.items.first {
                    $0.tag == Int(YU_STORAGE_COMMAND_TABLE_INSERT_COLUMN_AFTER)
                }, "缺少新增列菜单")
                try require(textView.validateMenuItem(insertColumn), "新增列菜单不可用")
                textView.editTableFromMenu(insertColumn)
                try require(bridge.source != original, "新增列菜单未修改源码")
                let columnFrame = try await submit(force: true)
                try require(columnFrame?.submitted == true, "新增列没有提交实际帧")
                let newDividers = surfaceCoordinator.tableResizeAccessibilityDividers().filter {
                    $0.kind == UInt8(YU_STORAGE_TABLE_RESIZE_COLUMN) && $0.blockIndex == divider.blockIndex
                }
                try require(newDividers.count > dividers.filter {
                    $0.kind == UInt8(YU_STORAGE_TABLE_RESIZE_COLUMN) && $0.blockIndex == divider.blockIndex
                }.count, "新增列未增加统一表格几何中的分隔线")
                textView.performUndo()
                try require(bridge.source == original, "菜单新增列撤销未恢复原文")
                _ = try await submit(force: true)
                let restoredDivider = try require(surfaceCoordinator.tableResizeAccessibilityDividers().first {
                    $0.blockIndex == divider.blockIndex && $0.index == divider.index && $0.kind == divider.kind
                }, "结构撤销后缺少列分隔线")
                try require(abs(restoredDivider.rect.midX - resized.rect.midX) <= 1,
                    "结构撤销丢失已确认列宽")
                let header = (original as NSString).range(of: "Table P1 A")
                let firstCell = try bridge.sourceCaret(revision: bridge.revision, sourceUTF16: UInt64(header.location), affinity: 1, size: Float(size), maxWidth: inputWidth)
                let lastCell = try bridge.sourceCaret(revision: bridge.revision, sourceUTF16: UInt64(last.location), affinity: 1, size: Float(size), maxWidth: inputWidth)
                try require(textView.selectTableCellsAtVisualPoint(NSPoint(x: firstCell.point.x + 1, y: firstCell.point.y + 1), starting: true), "矩形选区起点失败")
                _ = try await submit(force: true)
                try require(textView.selectTableCellsAtVisualPoint(NSPoint(x: lastCell.point.x + 1, y: lastCell.point.y + 1), starting: false), "矩形选区终点失败")
                textView.finishTableCellSelection()
                let cellFrame = try await submit(force: true)
                try require(bridge.tableSelectionColumns == 2 && bridge.selectionsIfAvailable?.ranges.count == 4, "矩形选区未覆盖四个单元格")
                try require(cellFrame?.selectionDecorationCount == 4 && cellFrame?.caretDecorationCount == 0, "矩形选区未绘制整格高亮或残留插入光标")
                let gridBoard = NSPasteboard.withUniqueName()
                defer { gridBoard.releaseGlobally() }
                try textView.copyToPasteboardForSelfCheck(gridBoard)
                textView.navigate(toSource: NSRange(location: last.location, length: 0))
                try textView.pasteFromPasteboardForSelfCheck(gridBoard)
                let pastedGridFrame = try await submit(force: true)
                try require(bridge.source != original && bridge.tableSelectionColumns == 2, "网格粘贴未扩展表格或保留选区")
                try require(pastedGridFrame?.submitted == true && pastedGridFrame?.selectionDecorationCount == 4, "网格粘贴未提交四格高亮")
                let gridDividers = surfaceCoordinator.tableResizeAccessibilityDividers().filter {
                    $0.kind == UInt8(YU_STORAGE_TABLE_RESIZE_COLUMN) && $0.blockIndex == divider.blockIndex
                }
                try require(gridDividers.count > dividers.filter {
                    $0.kind == UInt8(YU_STORAGE_TABLE_RESIZE_COLUMN) && $0.blockIndex == divider.blockIndex
                }.count, "网格粘贴未产生新列几何")
                textView.performUndo()
                try require(bridge.source == original, "网格粘贴撤销未恢复原文")
                _ = try await submit(force: true)
                guard let externalMarkdown = gridBoard.string(forType: .yuMarkdown) else {
                    throw Failure(message: "缺少外部 Markdown 表格表示")
                }
                gridBoard.clearContents()
                try require(gridBoard.setString(externalMarkdown, forType: .yuMarkdown), "无法设置外部 Markdown 剪贴板")
                textView.navigate(toSource: NSRange(location: last.location, length: 0))
                try textView.pasteFromPasteboardForSelfCheck(gridBoard)
                let externalGridFrame = try await submit(force: true)
                try require(bridge.source != original && bridge.tableSelectionColumns == 2, "外部 Markdown 表格未形成网格编辑")
                try require(externalGridFrame?.submitted == true && externalGridFrame?.selectionDecorationCount == 4, "外部表格未提交四格高亮")
                textView.performUndo()
                try require(bridge.source == original, "外部 Markdown 表格撤销未恢复原文")
                _ = try await submit(force: true)
                gridBoard.clearContents()
                try require(gridBoard.setString("BR_P1_FIRST<br>BR_P1_SECOND", forType: .yuMarkdown), "换行测试剪贴板失败")
                textView.navigate(toSource: NSRange(location: last.location, length: 0))
                try textView.pasteFromPasteboardForSelfCheck(gridBoard)
                let breakSource = bridge.source
                let breakFirst = (breakSource as NSString).range(of: "BR_P1_FIRST").location
                let breakSecond = (breakSource as NSString).range(of: "BR_P1_SECOND").location
                textView.navigate(toSource: NSRange(location: breakSecond, length: 0))
                let breakFrame = try await submit(force: true)
                try require(breakFrame?.submitted == true, "多行单元格未提交绘制")
                let firstBreakCaret = try bridge.sourceCaret(revision: bridge.revision, sourceUTF16: UInt64(breakFirst), affinity: 1, size: Float(size), maxWidth: inputWidth)
                let secondBreakCaret = try bridge.sourceCaret(revision: bridge.revision, sourceUTF16: UInt64(breakSecond), affinity: 1, size: Float(size), maxWidth: inputWidth)
                let firstBreakEnd = try bridge.sourceCaret(revision: bridge.revision, sourceUTF16: UInt64(breakFirst + "BR_P1_FIRST".utf16.count), affinity: 1, size: Float(size), maxWidth: inputWidth)
                let secondBreakEnd = try bridge.sourceCaret(revision: bridge.revision, sourceUTF16: UInt64(breakSecond + "BR_P1_SECOND".utf16.count), affinity: 1, size: Float(size), maxWidth: inputWidth)
                print("Yu BR diagnostic size=\(size) first=\(firstBreakCaret.point)→\(firstBreakEnd.point) second=\(secondBreakCaret.point)→\(secondBreakEnd.point)")
                try require(secondBreakCaret.point.y > firstBreakCaret.point.y + size * 0.5, "CoreText 未将 br 排为下一行")
                try require(firstBreakCaret.point.x > secondBreakCaret.point.x + size * 0.5, "右对齐表格的短行未逐行靠右 size=\(size) first=\(firstBreakCaret.point) second=\(secondBreakCaret.point)")
                let breakHit = try bridge.projectionHitTest(revision: bridge.revision, point: CGPoint(x: secondBreakCaret.point.x + 0.1, y: secondBreakCaret.point.y + 1), size: Float(size), maxWidth: inputWidth)
                try require(breakHit.sourceUTF16 == UInt64(breakSecond), "多行单元格点击没有返回第二行源码")
                // Bring the full last cell into view, then restore the second-line
                // caret so capture includes wrapped trailing text and bottom border.
                textView.navigate(toSource: NSRange(location: (breakSource as NSString).length, length: 0))
                _ = try await submit(force: true)
                textView.navigate(toSource: NSRange(location: breakSecond, length: 0))
                _ = try await submit(force: true)
                if let directory = ProcessInfo.processInfo.environment["YU_CELL_SELECTION_CAPTURE_DIR"] {
                    try await Task.sleep(nanoseconds: 100_000_000)
                    guard let window = view.window else { throw Failure(message: "编辑截图没有窗口") }
                    let image = try await captureEditingImage()
                    guard let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]) else {
                        throw Failure(message: "多行单元格原尺寸截图失败")
                    }
                    let folder = URL(fileURLWithPath: directory, isDirectory: true)
                    let dark = window.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
                    let name = "break-\(dark ? "night" : "github")-zoom-\(zoom)"
                    try png.write(to: folder.appendingPathComponent(name + ".png"))
                    try breakSource.write(to: folder.appendingPathComponent(name + ".md"), atomically: true, encoding: .utf8)
                    let metadata: [String: Any] = ["scale": window.backingScaleFactor, "width_pt": window.frame.width, "height_pt": window.frame.height,
                        "pixel_width": image.width, "pixel_height": image.height, "zoom": zoom, "reference_comparison": false,
                        "first_line_y": firstBreakCaret.point.y, "second_line_y": secondBreakCaret.point.y, "second_line_source_utf16": breakSecond,
                        "first_line_x": firstBreakCaret.point.x, "second_line_x": secondBreakCaret.point.x]
                    try JSONSerialization.data(withJSONObject: metadata, options: [.prettyPrinted, .sortedKeys]).write(to: folder.appendingPathComponent(name + ".json"))
                }
                try bridge.save()
                let breakReopened = try StorageBridge(path: bridge.path)
                try require(breakReopened.source == breakSource, "br 保存重开源码不一致")
                textView.performUndo()
                try require(bridge.source == original, "br 粘贴撤销未恢复原文")
                _ = try await submit(force: true)
                gridBoard.clearContents()
                try require(gridBoard.setString("\" TSVFIRST\nTSVSECOND \"\t\"a\tb\"", forType: .tabularText), "TSV 剪贴板失败")
                textView.navigate(toSource: NSRange(location: last.location, length: 0))
                try textView.pasteFromPasteboardForSelfCheck(gridBoard)
                let tsvSource = bridge.source
                try require(tsvSource.contains("&#32;TSVFIRST<br>TSVSECOND&#32;") && tsvSource.contains("a&#9;b"), "TSV 单元格编码失败")
                let tsvFirst = (tsvSource as NSString).range(of: "TSVFIRST").location
                let tsvSecond = (tsvSource as NSString).range(of: "TSVSECOND").location
                textView.navigate(toSource: NSRange(location: tsvSecond, length: 0))
                let tsvFrame = try await submit(force: true)
                try require(tsvFrame?.submitted == true, "TSV 未提交绘制")
                let firstTSVCaret = try bridge.sourceCaret(revision: bridge.revision, sourceUTF16: UInt64(tsvFirst), affinity: 1, size: Float(size), maxWidth: inputWidth)
                let secondTSVCaret = try bridge.sourceCaret(revision: bridge.revision, sourceUTF16: UInt64(tsvSecond), affinity: 1, size: Float(size), maxWidth: inputWidth)
                try require(secondTSVCaret.point.y > firstTSVCaret.point.y + size * 0.5, "TSV 多行几何失败")
                let tsvHit = try bridge.projectionHitTest(revision: bridge.revision, point: CGPoint(x: secondTSVCaret.point.x + 0.1, y: secondTSVCaret.point.y + 1), size: Float(size), maxWidth: inputWidth)
                try require(tsvHit.sourceUTF16 == UInt64(tsvSecond), "TSV 第二行点击源码偏移错误")
                if let directory = ProcessInfo.processInfo.environment["YU_CELL_SELECTION_CAPTURE_DIR"] {
                    try await Task.sleep(nanoseconds: 100_000_000)
                    guard let window = view.window else { throw Failure(message: "编辑截图没有窗口") }
                    let image = try await captureEditingImage()
                    guard let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]) else {
                        throw Failure(message: "TSV 原尺寸截图失败")
                    }
                    let folder = URL(fileURLWithPath: directory, isDirectory: true)
                    let dark = window.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
                    let name = "tsv-\(dark ? "night" : "github")-zoom-\(zoom)"
                    try png.write(to: folder.appendingPathComponent(name + ".png"))
                    try tsvSource.write(to: folder.appendingPathComponent(name + ".md"), atomically: true, encoding: .utf8)
                    let metadata: [String: Any] = ["scale": window.backingScaleFactor, "width_pt": window.frame.width, "height_pt": window.frame.height,
                        "pixel_width": image.width, "pixel_height": image.height, "zoom": zoom, "reference_comparison": false,
                        "first_line_y": firstTSVCaret.point.y, "second_line_y": secondTSVCaret.point.y, "second_line_source_utf16": tsvSecond]
                    try JSONSerialization.data(withJSONObject: metadata, options: [.prettyPrinted, .sortedKeys]).write(to: folder.appendingPathComponent(name + ".json"))
                }
                try bridge.save()
                let tsvReopened = try StorageBridge(path: bridge.path)
                try require(tsvReopened.source == tsvSource, "TSV 保存重开不一致")
                textView.performUndo()
                try require(bridge.source == original, "TSV 撤销未恢复原文")
                _ = try await submit(force: true)
                textView.navigate(toSource: NSRange(location: last.location, length: 0))
                try bridge.save()
                textView.refreshFromRust()
                _ = try await submit(force: true)
                // Each zoom tests the same starting proportions. Text undo now
                // intentionally retains confirmed widths, so reset the gesture
                // explicitly instead of relying on the former width-loss bug.
                let finalDivider = try require(surfaceCoordinator.tableResizeAccessibilityDividers().first {
                    $0.blockIndex == divider.blockIndex && $0.index == divider.index && $0.kind == divider.kind
                }, "缩放测试结束后缺少列分隔线")
                try require(abs(finalDivider.rect.midX - resized.rect.midX) <= 1,
                    "连续编辑撤销后丢失已确认列宽 size=\(size) expected=\(resized.rect) actual=\(finalDivider.rect) viewport=\(textView.bounds.width)")
                let finalPoint = NSPoint(x: finalDivider.rect.midX, y: finalDivider.rect.midY)
                try require(surfaceCoordinator.beginTableResize(at: finalPoint), "还原测试列宽未开始")
                try require(surfaceCoordinator.updateTableResize(at: NSPoint(x: divider.rect.midX, y: finalPoint.y)), "还原测试列宽未更新")
                try require(surfaceCoordinator.finishTableResize(), "还原测试列宽未完成")
                _ = try await submit(force: true)
                let resetDivider = try require(surfaceCoordinator.tableResizeAccessibilityDividers().first {
                    $0.blockIndex == divider.blockIndex && $0.index == divider.index && $0.kind == divider.kind
                }, "还原测试列宽后缺少分隔线")
                try require(abs(resetDivider.rect.midX - divider.rect.midX) <= 1,
                    "缩放测试未恢复起始列宽")
            }
            textView.font = originalFont
            surfaceCoordinator.setFontSize(baseSize)
            _ = try await submit(force: true)
            print("Yu table editing self-check: nested Tab/Shift-Tab, append, Unicode edit, undo/redo, resize and save/reopen passed at 3 zooms")
            print("Yu rectangular cell window self-check: shared pointer geometry, 4/6 cell highlights including empty cells and no text carets passed at 3 zooms")
            print("Yu HTML break window self-check: CoreText multiline cells, second-line hit/source mapping, save/reopen and undo passed at 3 zooms")
            print("Yu external Markdown table window self-check: external clipboard representation, grid growth, submitted four-cell highlight and undo passed at 3 zooms")
            print("Yu grid paste window self-check: copied rectangle, single-cell target growth, submitted selection/column geometry and undo passed at 3 zooms")
            print("Yu table menu window self-check: native column action, submitted geometry and undo passed at 3 zooms")
            print("Yu table literal input self-check: typed pipe, escaped Markdown paste and single-character projection passed at 3 zooms")
        }

        // Validate navigation after a real menu target/action round trip, not
        // merely whether its two panels became visible. Run after initial layout.
        let savedSidebar = sidebarTabs.selectedSegment
        let savedSidebarHidden = sidebarHidden
        let savedEndpoints = bridge.selectionEndpoints
        let sidebarAction = try require(sidebarTabs.action, "侧栏菜单缺少 action")
        for index in [0, 1, 0, 1] {
            sidebarTabs.selectedSegment = index
            try require(NSApp.sendAction(sidebarAction, to: sidebarTabs.target, from: sidebarTabs),
                "侧栏菜单 action 未送达")
            try require(filePanel.view.isHidden == (index != 0)
                && outlinePanel.scrollView.isHidden == (index != 1), "侧栏标题菜单未切换对应面板")
            if index == 1 {
                for row in [0, outlinePanel.rowCountForSelfCheck - 1] {
                    let node = try require(outlinePanel.nodeForSelfCheck(row: row), "切换后缺少大纲行")
                    outlinePanel.clickRowForSelfCheck(row)
                    try require(bridge.selection.range == NSRange(location: node.item.labelRange.location, length: 0),
                        "切换后大纲导航错误 row=\(row) expected=\(node.item.labelRange.location) actual=\(bridge.selection.range)")
                }
            }
        }
        sidebarTabs.selectedSegment = savedSidebar
        sidebarHidden = savedSidebarHidden
        updateSidebarVisibility()
        try bridge.setSelectionEndpoints(anchorUTF16: savedEndpoints.anchorUTF16,
            focusUTF16: savedEndpoints.focusUTF16, affinity: savedEndpoints.affinity)
        textView.refreshFromRust()
        _ = try await submit(force: true)
        print("Yu sidebar navigation self-check: native menu dispatch, repeated file/outline switching and first/last heading navigation passed")

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

    /// Scripted real-window protocol checks. These never synthesize live-scroll
    /// boundaries, so their presents cannot be counted as trackpad acceptance.
    /// A submitted command buffer is insufficient evidence that the document
    /// reached the compositor. The fixed visual fixture must have visible ink
    /// inside the editor, excluding sidebar, title and status text.
    private func captureContainsDocumentInk(_ image: CGImage, dark: Bool) -> Bool {
        let bitmap = NSBitmapImageRep(cgImage: image)
        let packed = NativeTheme.spec(dark: dark).background
        // NSBitmapImageRep.colorAt exposes calibrated components. Convert the
        // expected framebuffer components through the same space before comparing.
        let expected = NSColor(calibratedRed: CGFloat((packed >> 24) & 255) / 255,
                               green: CGFloat((packed >> 16) & 255) / 255,
                               blue: CGFloat((packed >> 8) & 255) / 255, alpha: 1).usingColorSpace(.sRGB)!
        let background = [expected.redComponent, expected.greenComponent, expected.blueComponent]
        guard let window = view.window, let scroll = textView.enclosingScrollView else { return false }
        let scale = window.backingScaleFactor
        let surface = surfaceHostView.convert(surfaceHostView.bounds, to: nil)
        let startX = max(0, Int(((surface.minX + textView.contentOrigin.x) * scale).rounded(.up)))
        let endX = min(image.width, Int(((surface.maxX - textView.contentOrigin.x) * scale).rounded(.down)))
        let top = window.frame.height - surface.maxY + max(0, textView.contentOrigin.y - scroll.contentView.bounds.origin.y)
        let startY = max(0, Int((top * scale).rounded(.up)))
        let endY = min(image.height, Int(((window.frame.height - surface.minY - 1) * scale).rounded(.down)))
        guard startX < endX, startY < endY else { return false }
        // Ink alone accepts an entire stale white canvas as "ink" in Night.
        // The reading-column margin must already show the requested theme.
        let marginX = max(Int((surface.minX + 2) * scale), startX - Int(4 * scale))
        var matchingBackground = 0
        var backgroundSamples = 0
        for y in stride(from: startY, to: endY, by: 8) {
            guard let color = bitmap.colorAt(x: marginX, y: y)?.usingColorSpace(.sRGB) else { continue }
            backgroundSamples += 1
            let difference = abs(color.redComponent - background[0]) + abs(color.greenComponent - background[1]) + abs(color.blueComponent - background[2])
            if difference < 0.1 { matchingBackground += 1 }
        }
        guard backgroundSamples > 0, matchingBackground * 5 >= backgroundSamples * 4 else { return false }
        var inkSamples = 0
        for y in stride(from: startY, to: endY, by: 4) {
            for x in stride(from: startX, to: endX, by: 4) {
                guard let color = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else { continue }
                let difference = abs(color.redComponent - background[0]) + abs(color.greenComponent - background[1]) + abs(color.blueComponent - background[2])
                if difference > 0.35 { inkSamples += 1 }
                if inkSamples >= 40 { return true }
            }
        }
        return false
    }

    @MainActor
    /// Captures the real compositor result, including Metal, never an NSView
    /// cache that could silently omit the document surface.
    func runLayoutCoordinatorSelfCheck() async throws {
        try await surfaceCoordinator.verifyLayoutCoordinatorForSelfCheck()
    }

    private func verifyScrollExtentForSelfCheck(_ scroll: NSScrollView) throws {
        struct Failure: LocalizedError {
            let message: String
            var errorDescription: String? { message }
        }
        let insets = scroll.contentInsets
        guard !scroll.automaticallyAdjustsContentInsets,
              insets.top == 0, insets.bottom == 0, insets.left == 0, insets.right == 0 else {
            throw Failure(message: "AppKit added scroll insets outside the shared reading geometry")
        }
        let overflow = textView.frame.height - scroll.contentView.bounds.height
        let hidden = scroll.verticalScroller?.isHidden ?? true
        if overflow <= 0.01 && !hidden {
            throw Failure(message: "Fitting document retained a vertical scroller: overflow=\(overflow)")
        }
        if overflow > 1 && hidden {
            throw Failure(message: "Overflowing document lost its vertical scroller: overflow=\(overflow)")
        }
        guard abs(textView.bounds.width - scroll.contentView.bounds.width) < 0.5 else {
            throw Failure(message: "Document width did not follow native scroller visibility")
        }
    }

    func captureVisualAcceptance(to directory: URL) async throws {
        struct Failure: LocalizedError {
            let message: String
            var errorDescription: String? { message }
        }
        guard let window = view.window, let scroll = documentScrollView else {
            throw Failure(message: "Visual acceptance requires a real window")
        }
        func digest(_ data: Data) -> String {
            SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
        }
        guard let executable = Bundle.main.executableURL else {
            throw Failure(message: "Capture requires an identifiable application executable")
        }
        let sourceAtStart = bridge.source
        let revisionAtStart = bridge.revision
        let executableDigest = digest(try Data(contentsOf: executable, options: .mappedIfSafe))
        let sourceDigest = digest(Data(sourceAtStart.utf8))
        var fontDigests: [String: String] = [:]
        if let fonts = Bundle.main.resourceURL?.appendingPathComponent("Fonts") {
            for url in try FileManager.default.contentsOfDirectory(at: fonts, includingPropertiesForKeys: nil)
                where ["ttf", "otf"].contains(url.pathExtension.lowercased()) {
                fontDigests[url.lastPathComponent] = digest(try Data(contentsOf: url, options: .mappedIfSafe))
            }
        }
        let captureStartedAt = ISO8601DateFormatter().string(from: Date())
        let captureFraction = Double(ProcessInfo.processInfo.environment["YU_VISUAL_SCROLL_FRACTION"] ?? "0") ?? -1
        guard captureFraction.isFinite, (0...1).contains(captureFraction) else {
            throw Failure(message: "YU_VISUAL_SCROLL_FRACTION must be between 0 and 1")
        }
        isCapturingVisualAcceptance = true
        defer { isCapturingVisualAcceptance = false }
        let activationDeadline = Date().addingTimeInterval(45)
        while (!NSApp.isActive || !window.isKeyWindow), Date() < activationDeadline {
            try await Task.sleep(nanoseconds: 100_000_000)
        }
        guard NSApp.isActive, window.isKeyWindow else {
            throw Failure(message: "Activate the Yu window before capture; background Stage Manager thumbnails are not acceptance screenshots")
        }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try bridge.setSelection(NSRange(location: (bridge.source as NSString).length, length: 0), affinity: 1)
        textView.refreshFromRust()
        outlinePanel.highlightHeading(containing: Int(bridge.selectionEndpoints.focusUTF16))
        // Calibration may match an explicitly measured reference sidebar width;
        // ordinary windows keep the product default. Record the actual geometry.
        let captureSidebarWidth: CGFloat
        if let value = ProcessInfo.processInfo.environment["YU_VISUAL_SIDEBAR_WIDTH"] {
            guard let width = Double(value), width.isFinite, (180...400).contains(width) else {
                throw Failure(message: "Capture sidebar width must be between 180 and 400pt")
            }
            captureSidebarWidth = CGFloat(width)
        } else {
            captureSidebarWidth = YuVisualTokens.sidebarWidth
        }
        var cases: [[String: Any]] = []
        for size in [NSSize(width: 900, height: 620), NSSize(width: 1200, height: 800), NSSize(width: 1600, height: 1000)] {
            for dark in [false, true] {
                let themeName: String
                switch NativeTheme.selection {
                case .yu: themeName = dark ? "yu-dark" : "yu-light"
                case .github: themeName = "github"
                case .night: themeName = "night"
                }
                let appearanceName = dark ? "dark" : "light"
                for sidebar in [true, false] {
                    sidebarHidden = !sidebar
                    updateSidebarVisibility()
                    window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
                    window.setFrame(NSRect(origin: window.frame.origin, size: size), display: true)
                    view.layoutSubtreeIfNeeded()
                    if sidebar {
                        documentSplitView?.setPosition(captureSidebarWidth, ofDividerAt: 0)
                        view.layoutSubtreeIfNeeded()
                    }
                    updateReadingColumnInsets()
                    syncSurfaceGeometry()
                    guard abs(textView.bounds.width - scroll.contentView.bounds.width) < 0.5 else {
                        throw Failure(message: "Input and painted viewport widths diverged after resize")
                    }
                    surfaceCoordinator.noteBoundsEvent()
                    scroll.contentView.setBoundsOrigin(.zero)
                    surfaceCoordinator.scheduleSubmit()
                    let deadline = Date().addingTimeInterval(20)
                    while (surfaceHostView.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua) != dark, Date() < deadline {
                        try await Task.sleep(nanoseconds: 20_000_000)
                    }
                    guard (surfaceHostView.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua) == dark else {
                        throw Failure(message: "Surface appearance did not reach requested theme")
                    }
                    // A drawable submitted before the window becomes visible can
                    // complete with presentedTime == 0. Submission deduplication
                    // must not leave this acceptance wait stuck on that drawable.
                    // Retry a bounded number of times; success still requires an
                    // actual presentation callback for the latest frame.
                    var presentationRetries = 0
                    var nextPresentationRetry = Date().addingTimeInterval(1)
                    // A presented viewport may still use estimated heights for
                    // offscreen blocks. Acceptance manifests must wait for the
                    // progressive layout as well as the visible drawable.
                    while (!surfaceCoordinator.hasCurrentFrame(requirePresented: true)
                        || surfaceCoordinator.lastSnapshot?.layoutPending != false), Date() < deadline {
                        let retryPresentation = surfaceCoordinator.hasCurrentFrame()
                            && presentationRetries < 3 && Date() >= nextPresentationRetry
                        _ = try surfaceCoordinator.submitNow(force: retryPresentation)
                        if retryPresentation {
                            presentationRetries += 1
                            nextPresentationRetry = Date().addingTimeInterval(1)
                        }
                        try await Task.sleep(nanoseconds: 20_000_000)
                    }
                    guard surfaceCoordinator.hasCurrentFrame(requirePresented: true),
                          surfaceCoordinator.lastSnapshot?.layoutPending == false,
                          surfaceCoordinator.lastSnapshot?.commandCount ?? 0 > 0 else {
                        throw Failure(message: "No current native frame for visual capture: submitted=\(surfaceCoordinator.hasCurrentFrame()) presented=\(surfaceCoordinator.hasCurrentFrame(requirePresented: true)) layoutPending=\(String(describing: surfaceCoordinator.lastSnapshot?.layoutPending)) commands=\(surfaceCoordinator.lastSnapshot?.commandCount ?? 0) active=\(NSApp.isActive) visible=\(window.isVisible)")
                    }
                    let captureScrollY = max(0, textView.frame.height - scroll.contentView.bounds.height) * captureFraction
                    scroll.contentView.setBoundsOrigin(NSPoint(x: 0, y: captureScrollY))
                    surfaceCoordinator.scheduleSubmit()
                    let scrollDeadline = Date().addingTimeInterval(20)
                    while (!surfaceCoordinator.hasCurrentFrame(requirePresented: true)
                        || surfaceCoordinator.lastSnapshot?.layoutPending != false), Date() < scrollDeadline {
                        _ = try surfaceCoordinator.submitNow()
                        try await Task.sleep(nanoseconds: 20_000_000)
                    }
                    guard surfaceCoordinator.hasCurrentFrame(requirePresented: true),
                          surfaceCoordinator.lastSnapshot?.layoutPending == false else {
                        throw Failure(message: "No current native frame at requested scroll position")
                    }
                    var captured: CGImage?
                    var rejected: CGImage?
                    let captureDeadline = Date().addingTimeInterval(5)
                    let expectedWidth = Int((window.frame.width * window.backingScaleFactor).rounded())
                    let expectedHeight = Int((window.frame.height * window.backingScaleFactor).rounded())
                    var captureFailure = "capture unavailable"
                    while Date() < captureDeadline {
                        try await Task.sleep(nanoseconds: 100_000_000)
                        guard surfaceCoordinator.hasCurrentFrame(requirePresented: true),
                              surfaceCoordinator.lastSnapshot?.layoutPending == false else {
                            _ = try surfaceCoordinator.submitNow()
                            captureFailure = "latest geometry/theme has not presented or layout remains pending"; continue
                        }
                        guard NSApp.isActive else { captureFailure = "application inactive"; continue }
                        let image: CGImage
                        do {
                            image = try await NativeWindowCapture.image(window)
                        } catch {
                            captureFailure = error.localizedDescription
                            continue
                        }
                        guard image.width == expectedWidth, image.height == expectedHeight else {
                            captureFailure = "received \(image.width)×\(image.height) pixels"; continue
                        }
                        guard captureContainsDocumentInk(image, dark: dark) else {
                            rejected = image
                            captureFailure = "canvas theme or document ink mismatch"; continue
                        }
                        captured = image
                        break
                    }
                    guard let image = captured else {
                        if let rejected, let png = NSBitmapImageRep(cgImage: rejected).representation(using: .png, properties: [:]) {
                            let name = "rejected-\(Int(size.width))x\(Int(size.height))-\(themeName)-chrome-\(appearanceName)-sidebar-\(sidebar ? "on" : "off").png"
                            try png.write(to: directory.appendingPathComponent(name), options: .atomic)
                        }
                        throw Failure(message: "Full-window capture failed: \(captureFailure); expected \(expectedWidth)×\(expectedHeight) pixels")
                    }
                    let bitmap = NSBitmapImageRep(cgImage: image)
                    guard let png = bitmap.representation(using: .png, properties: [:]) else {
                        throw Failure(message: "Screenshot PNG encoding failed")
                    }
                    guard abs(scroll.contentView.bounds.origin.y - captureScrollY) < 0.5 else {
                        throw Failure(message: "Screenshot scroll origin changed during capture")
                    }
                    try verifyScrollExtentForSelfCheck(scroll)
                    let name = "\(Int(size.width))x\(Int(size.height))-\(themeName)-chrome-\(appearanceName)-sidebar-\(sidebar ? "on" : "off")-\(Int(window.backingScaleFactor))x"
                    guard bridge.revision == revisionAtStart, bridge.source == sourceAtStart else {
                        throw Failure(message: "Document changed during visual acceptance capture")
                    }
                    let screenshotURL = directory.appendingPathComponent(name + ".png")
                    guard !FileManager.default.fileExists(atPath: screenshotURL.path) else {
                        throw Failure(message: "Refusing to overwrite an existing acceptance screenshot: \(name)")
                    }
                    try png.write(to: screenshotURL, options: .atomic)
                    let theme = NativeTheme.spec(dark: dark)
                    let screenNumber = window.screen?.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber
                    let displayID = CGDirectDisplayID(screenNumber?.uint32Value ?? 0)
                    guard let displayMode = CGDisplayCopyDisplayMode(displayID) else {
                        throw Failure(message: "Cannot identify capture display mode")
                    }
                    cases.append(["case": name, "window_width": window.frame.width,
                        "png_sha256": digest(png), "theme": themeName,
                        "chrome_appearance": appearanceName,
                        "resolved_theme": NativeTheme.resolved(dark: dark),
                        "font_size_pt": textView.font?.pointSize ?? CGFloat(theme.body_size),
                        "declared_body_font": NativeTheme.font(identity: theme.body_font, size: CGFloat(theme.body_size)).fontName,
                        "declared_heading_font": NativeTheme.font(identity: theme.heading_font, size: CGFloat(theme.body_size)).fontName,
                        "declared_code_font": NativeTheme.font(identity: theme.code_font, size: CGFloat(theme.body_size)).fontName,
                        "display_id": displayID,
                        "display_pixel_width": displayMode.pixelWidth,
                        "display_pixel_height": displayMode.pixelHeight,
                        "display_mode_width": displayMode.width,
                        "display_mode_height": displayMode.height,
                        "display_point_width": window.screen?.frame.width ?? 0,
                        "display_point_height": window.screen?.frame.height ?? 0,
                        "window_height": window.frame.height, "pixel_width": image.width,
                        "pixel_height": image.height, "scale": window.backingScaleFactor,
                        "titlebar_height": window.frame.height - window.contentLayoutRect.height,
                        "surface_left": surfaceHostView.convert(surfaceHostView.bounds, to: nil).minX,
                        "reading_window_x": surfaceHostView.convert(surfaceHostView.bounds, to: nil).minX + textView.contentOrigin.x,
                        "surface_top": window.frame.height - surfaceHostView.convert(surfaceHostView.bounds, to: nil).maxY,
                        "sidebar_width": sidebar ? (sidebarContainer?.frame.width ?? 0) : 0,
                        "clip_height": scroll.contentView.bounds.height,
                        "document_height": textView.frame.height,
                        "scroll_inset_top": scroll.contentInsets.top,
                        "scroll_inset_bottom": scroll.contentInsets.bottom,
                        "scroller_hidden": scroll.verticalScroller?.isHidden ?? true,
                        "scroller_knob": scroll.verticalScroller?.knobProportion ?? 0,
                        "reading_y": textView.contentOrigin.y,
                        "reading_x": textView.contentOrigin.x,
                        "reading_width": scroll.contentView.bounds.width - textView.contentOrigin.x * 2,
                        "scroll_y": scroll.contentView.bounds.origin.y, "scroll_fraction": captureFraction,
                        "source_revision": bridge.revision, "status": "captured-uncompared"])
                }
            }
        }
        guard digest(try Data(contentsOf: executable, options: .mappedIfSafe)) == executableDigest else {
            throw Failure(message: "Application executable changed during visual acceptance capture")
        }
        let manifest = try JSONSerialization.data(withJSONObject: ["schema_version": 3, "cases": cases,
            "app_sha256": executableDigest, "source_sha256": sourceDigest,
            "source_filename": URL(fileURLWithPath: bridge.path).lastPathComponent,
            "font_resources_sha256": fontDigests,
            "os_version": ProcessInfo.processInfo.operatingSystemVersionString,
            "capture_started_at": captureStartedAt,
            "capture_finished_at": ISO8601DateFormatter().string(from: Date()),
            "capture_method": "ScreenCaptureKit/SCScreenshotManager",
            "visual_acceptance_passed": false, "note": "Yu regression baseline; review clipping, spacing and interaction separately"], options: [.prettyPrinted, .sortedKeys])
        try manifest.write(to: directory.appendingPathComponent("manifest.json"), options: .atomic)
    }

    func runRenderRegressionSelfCheck(resources: Bool, reopened: Bool = false) async throws {
        struct Failure: LocalizedError {
            let message: String
            var errorDescription: String? { message }
        }
        func require(_ condition: Bool, _ message: String) throws {
            if !condition { throw Failure(message: message) }
        }
        func submit() async throws -> NativeMacosRenderHostSurfaceSnapshot {
            let deadline = Date().addingTimeInterval(20)
            repeat {
                if let frame = try surfaceCoordinator.submitNow(), frame.submitted { return frame }
                try await Task.sleep(nanoseconds: 10_000_000)
            } while Date() < deadline
            throw Failure(message: "Timed out waiting for scripted frame")
        }
        guard let scroll = documentScrollView, let document = scroll.documentView,
              let window = view.window else { throw Failure(message: "Missing real window") }
        func scrollTo(_ y: CGFloat) {
            scroll.contentView.setBoundsOrigin(NSPoint(x: 0, y: max(0, y)))
            scroll.reflectScrolledClipView(scroll.contentView)
        }
        let first = try await submit()
        try require(abs(scroll.contentView.bounds.minY) < 1, "Document did not open at top")
        try require(first.commandCount > 0, "Empty first frame")
        print("Yu render regression environment: window=\(window.frame.size) scale=\(window.backingScaleFactor) max_fps=\(window.screen?.maximumFramesPerSecond ?? 0)")
        if reopened {
            print("Yu render regression self-check: reopened=true top=true submitted=true")
            return
        }
        if resources {
            try require(first.resourceRefreshPending && !first.resourceRetryPending,
                        "Fixture did not start with in-flight resources")
            try require(first.imageRequestCount > 0 && first.imageResourceCount == 0,
                        "Image was not observed as pending")
            // Small movements remain in the original coverage while both workers
            // are delayed. A full publication here would hide the polling bug.
            for y in [CGFloat(20), 40, 20, 0] {
                scrollTo(y)
                let frame = try await submit()
                try require(frame.frameSerial == first.frameSerial && frame.presentationReused,
                            "Pending resources rebuilt a covered scroll frame")
            }
            let deadline = Date().addingTimeInterval(40)
            var previousHeight = first.contentHeight
            var heightChanges = 0
            var ready: NativeMacosRenderHostSurfaceSnapshot?
            // Observe only the coordinator's last submitted snapshot. No submit,
            // resource query or bounds event is allowed to wake the idle window.
            repeat {
                try await Task.sleep(nanoseconds: 30_000_000)
                if let frame = surfaceCoordinator.lastSnapshot {
                    if abs(frame.contentHeight - previousHeight) > 0.5 {
                        heightChanges += 1
                        previousHeight = frame.contentHeight
                    }
                    if !frame.resourceRefreshPending && frame.imageResourceCount > 0 {
                        ready = frame
                        break
                    }
                }
            } while Date() < deadline
            guard let ready else { throw Failure(message: "Idle completion notification did not publish resources") }
            try require(ready.imageFailureCount == 0 && ready.frameSerial > first.frameSerial,
                        "Completed resources did not replace placeholder publication")
            // 这条断言要求 fixture 图片的内在高度**高过正文行高**（行高 =
            // line_height × 1.6 ≈ 32pt）：占位与就绪都占同一行时几何本就不
            // 变，计数无从判读。fixture 的 latency.png 因此取 64px 高。
            try require(heightChanges == 1, "Image geometry changed \(heightChanges) times instead of once")
            try await Task.sleep(nanoseconds: 800_000_000)
            try require(surfaceCoordinator.lastSnapshot?.frameSerial == ready.frameSerial,
                        "Settled idle resources kept publishing frames")
            print("Yu render regression self-check: resources=true pending_retained=true idle_completion=true height_changes=\(heightChanges) images=\(ready.imageResourceCount)")
            return
        }
        try require(bridge.source.utf8.count >= 100_000, "Long fixture is less than 100 KB")
        try require(first.contentHeight > scroll.contentView.bounds.height * 30,
                    "Long fixture has insufficient scroll extent")
        guard let divider = surfaceCoordinator.tableResizeAccessibilityDividers().first else {
            throw Failure(message: "Submitted first frame has no table accessibility geometry")
        }
        let sourceBeforeResize = bridge.source
        let hoverPoint = NSPoint(x: divider.rect.minX, y: divider.rect.midY)
        for _ in 0..<100 {
            try require(surfaceCoordinator.tableResizeHover(at: hoverPoint),
                        "Submitted divider did not produce column hover")
        }
        try require(!surfaceCoordinator.tableResizeHover(at:
                        NSPoint(x: hoverPoint.x + 10, y: hoverPoint.y))
                    && !surfaceCoordinator.tableResizeHover(at:
                        NSPoint(x: hoverPoint.x, y: divider.rect.maxY + 1)),
                    "Hover escaped divider tolerance or vertical table extent")
        try require(surfaceCoordinator.lastSnapshot?.frameSerial == first.frameSerial
                    && !surfaceCoordinator.tableResizeActiveForSelfCheck,
                    "Read-only hover opened a gesture or changed publication")
        try require(divider.revision == bridge.revision && divider.rect.height > 0,
                    "Invalid submitted table accessibility geometry")
        try require(surfaceCoordinator.adjustTableResizeAccessibility(divider, direction: 1),
                    "Table accessibility increment failed")
        try require(surfaceCoordinator.tableResizeAccessibilityDividers().isEmpty,
                    "Unsubmitted table resize exposed stale accessibility geometry")
        try require(!surfaceCoordinator.tableResizeHover(at: hoverPoint),
                    "Unsubmitted resize exposed stale hover geometry")
        _ = try await submit()
        guard let adjusted = surfaceCoordinator.tableResizeAccessibilityDividers().first else {
            throw Failure(message: "Resized frame lost table accessibility geometry")
        }
        try require(abs(adjusted.rect.midX - divider.rect.midX - divider.adjustStep) < 0.5
                    && adjusted.tableSourceRange == divider.tableSourceRange
                    && bridge.source == sourceBeforeResize,
                    "Table accessibility geometry did not follow submitted resize")
        print("Yu render regression self-check: ax_frame_resize=true")
        try require(surfaceCoordinator.tableResizeHover(at:
                        NSPoint(x: adjusted.rect.minX, y: adjusted.rect.midY)),
                    "Hover did not follow resized publication")
        print("Yu render regression self-check: hover_frame=true")
        let original = window.frame
        var generation = first.surfaceGeneration
        for step in 1...12 {
            if step % 3 == 0 {
                var frame = original
                frame.size.width += step % 2 == 0 ? 96 : -80
                frame.size.height += 32
                window.setFrame(frame, display: true)
                window.contentView?.layoutSubtreeIfNeeded()
            }
            let range = max(0, document.frame.height - scroll.contentView.bounds.height)
            scrollTo(range * CGFloat(step) / 12)
            let frame = try await submit()
            try require(frame.commandCount > 0 && frame.revision == bridge.revision,
                        "Long scroll submitted an empty or stale frame")
            let expectedWidth = scroll.contentView.bounds.width - 2 * textView.contentOrigin.x
            try require(abs(CGFloat(surfaceCoordinator.visualDecorationGeometry()?.maxWidth ?? 0) - expectedWidth) < 1,
                        "Render geometry kept a stale pre-resize content width: step=\(step) expected=\(expectedWidth) actual=\(surfaceCoordinator.visualDecorationGeometry()?.maxWidth ?? 0) surface=\(surfaceHostView.bounds.width) clip=\(scroll.contentView.bounds.width) inset=\(textView.contentOrigin.x)")
            let extentTheme = NativeTheme.spec(dark: document.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua)
            let expectedExtent = max(frame.contentHeight + CGFloat(extentTheme.top + extentTheme.bottom), scroll.contentView.bounds.height)
            try require(abs(document.frame.height - expectedExtent) < 1,
                        "Scroll extent disagrees with accepted publication")
            if step % 3 == 0 {
                try require(frame.surfaceGeneration > generation, "Resize did not advance surface generation")
            }
            generation = frame.surfaceGeneration
        }
        window.setFrame(original, display: true)
        window.contentView?.layoutSubtreeIfNeeded()
        let tail = (bridge.source as NSString).range(of: "YU_END_OF_DOCUMENT")
        try require(tail.location != NSNotFound, "Missing final-line marker")
        textView.navigate(toSource: NSRange(location: tail.location, length: 0))
        surfaceCoordinator.revealCaretIfNeeded()
        let last = try await submit()
        let caret = try bridge.shapedCaretScrollRequest(
            revision: bridge.revision, size: Float(textView.font?.pointSize ?? 16),
            maxWidth: Float(max(textView.bounds.width - 2 * textView.contentOrigin.x, 1)),
            scrollY: Float(scroll.contentView.bounds.minY),
            viewportHeight: Float(scroll.contentView.bounds.height))
        // A fractional Rust target may round to a backing pixel in AppKit.
        // Test visibility itself, not exact equality with that scroll target.
        let finalLineVisible = caret.caretPoint.y >= scroll.contentView.bounds.minY - 0.5
            && caret.caretPoint.y + caret.caretHeight <= scroll.contentView.bounds.maxY + 0.5
            && last.caretDecorationCount > 0
        let finalLineFailure = "Final line cannot be brought into the viewport: carets=\(last.caretDecorationCount) needsScroll=\(caret.needsScroll) caretY=\(caret.caretPoint.y) currentY=\(scroll.contentView.bounds.minY) targetY=\(caret.targetScrollY) height=\(last.contentHeight) viewport=\(scroll.contentView.bounds.height)"
        // A later scroll wins over a still-pending navigation. Use bounds
        // events without manufacturing live-scroll performance samples.
        scrollTo(0)
        surfaceCoordinator.revealCaretIfNeeded()
        scrollTo(0)
        _ = try await submit()
        try await Task.sleep(nanoseconds: 100_000_000)
        try require(abs(scroll.contentView.bounds.minY) < 1,
                    "Pending caret navigation pulled back a later scroll")
        surfaceCoordinator.detach()
        try require(!surfaceCoordinator.hasCurrentFrame(), "Detach kept old frame current")
        try require(surfaceCoordinator.tableResizeAccessibilityDividers().isEmpty,
                    "Detach kept old table accessibility geometry")
        try require(!surfaceCoordinator.tableResizeHover(at: hoverPoint),
                    "Detach kept old hover geometry")
        let rebound = try await submit()
        try require(rebound.frameSerial > last.frameSerial, "Rebind used old publication")
        print("Yu render regression self-check: long=true steps=12 resize=4 final_line=\(finalLineVisible) scroll_cancels_navigation=true rebind=true bytes=\(bridge.source.utf8.count)")
        try require(finalLineVisible, finalLineFailure)
    }

    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        if settingsWindowIsKey { return menuItem.action == #selector(closeFromMenu(_:)) }
        if menuItem.action == #selector(editImagePropertiesFromMenu(_:)) { return textView.canEditImage() }
        if menuItem.action == #selector(insertImageFromMenu(_:)) { return textView.isEditable }
        let state = bridge.state
        if menuItem.action == #selector(saveFromMenu(_:)) {
            return state.dirty || persistence.isUntitled || bridge.composition.active
        }
        if menuItem.action == #selector(reloadFromMenu(_:)) {
            return !state.dirty && state.disk != .unchanged
        }
        if menuItem.action == #selector(zoomInFromMenu(_:)) { return readingZoom < 3 }
        if menuItem.action == #selector(zoomOutFromMenu(_:)) { return readingZoom > 0.5 }
        if menuItem.action == #selector(resetZoomFromMenu(_:)) { return abs(readingZoom - 1) > 0.001 }
        if menuItem.action == #selector(toggleOutlineFromMenu(_:)) {
            menuItem.state = outlineIsVisible ? .on : .off
            return true
        }
        if menuItem.action == #selector(toggleSearchFromMenu(_:)) {
            menuItem.state = searchIsVisible ? .on : .off
            return true
        }
        if menuItem.action == #selector(toggleSourceMode(_:)) {
            menuItem.state = bridge.sourceMode ? .on : .off
            return !textView.hasMarkedText()
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
        persistence.suspend()
        do {
            try textView.finishCompositionForFileOperation()
            try? persistence.flushRecovery()
            let request = try bridge.requestClose()
            switch request.result {
            case 0: return true
            case 1:
                if persistence.isPristineUntitled {
                    try bridge.resolveClose(UInt8(YU_STORAGE_CLOSE_RESOLVE_DISCARD))
                    return true
                }
                return prompt(request)
            default:
                persistence.cancelClose()
                return false
            }
        } catch {
            persistence.cancelClose()
            show(error)
            return false
        }
    }

    func cancelCloseRequest() { persistence.cancelClose() }

    func finalizeCloseRequest() throws { try persistence.finalizeClose() }

    private func prompt(_ request: YuStorageCloseRequest) -> Bool {
        let conflict = request.close_state >= 3
        let alert = NSAlert()
        alert.alertStyle = conflict ? .warning : .informational
        alert.messageText = conflict ? "文件已被外部修改" : "要保存对“\(view.window?.title ?? "未命名")”的更改吗？"
        alert.informativeText = conflict
            ? "磁盘版本不会被覆盖。可以将本地内容保存为副本，或丢弃本地修改。"
            : "可以保存、丢弃这次修改，或取消关闭。"
        alert.addButton(withTitle: conflict ? "保存副本…" : "保存")
        alert.addButton(withTitle: "不保存")
        alert.addButton(withTitle: "取消")
        let response = closeAlertDecision?(alert) ?? alert.runModal()
        do {
            if response == .alertFirstButtonReturn {
                let saved = conflict ? chooseSaveDestination() : saveDocument()
                guard saved else { persistence.cancelClose(); return false }
                try bridge.resolveClose(UInt8(YU_STORAGE_CLOSE_RESOLVE_SAVE))
                return true
            }
            if response == .alertSecondButtonReturn {
                try bridge.resolveClose(UInt8(YU_STORAGE_CLOSE_RESOLVE_DISCARD))
                return true
            }
            persistence.cancelClose()
            return false
        } catch {
            persistence.cancelClose()
            show(error)
            return false
        }
    }

    private func startFileWatcher() {
        guard fileWatcher == nil, !persistence.isUntitled else { return }
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
        guard !persistence.suspended, !persistence.isUntitled else { return }
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
                "文件已在其他应用中修改。本地更改仍保留，请保存副本或关闭窗口处理冲突。"
            alert.addButton(withTitle: "知道了")
        } else {
            alert.informativeText =
                "当前没有本地未保存修改，可以重新加载磁盘上的版本。"
            alert.addButton(withTitle: "重新加载")
            alert.addButton(withTitle: "稍后")
        }
        let response = externalAlertDecision?(alert) ?? alert.runModal()
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
        statusLabel.stringValue = persistence.notice ?? ""
        statusLabel.toolTip = persistence.notice ?? status
        view.window?.isDocumentEdited = state.dirty
        statusLabel.setAccessibilityValue(status)
        statusDetailLabel.stringValue = "\((bridge.source as NSString).length) 字符"
        statusDetailLabel.setAccessibilityValue(statusDetailLabel.stringValue)
    }

    private func show(_ error: Error) {
        if let fileErrorPresenter { fileErrorPresenter(error); return }
        let alert = NSAlert(error: error)
        alert.runModal()
    }

}

final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    private var settingsController: NativeSettingsWindowController?

    @objc private func showSettings(_ sender: Any?) {
        if settingsController == nil {
            let settings = NativeSettingsWindowController()
            if forceDarkMode || darkModeSelfCheck { settings.window?.appearance = NSAppearance(named: .darkAqua) }
            settings.onChange = { [weak self] in
                guard let self else { return }
                for document in self.documents.values { document.persistence.documentChanged() }
                self.installMainMenu(for: self.controller)
            }
            settingsController = settings
        }
        settingsController?.showWindow(sender)
        settingsController?.window?.makeKeyAndOrderFront(sender)
        NSApp.activate(ignoringOtherApps: true)
    }
    private var window: NSWindow?
    private var controller: DocumentViewController?
    private var launchSelfCheck = false
    private var darkModeSelfCheck = false
    private var forceDarkMode = false
    private var renderRegression = false
    private var resourceRegression = false
    private var documents: [NSWindow: DocumentViewController] = [:]
    private var consideredRecoveryFiles = Set<URL>()
    private var recoveryAlertDecision: ((NSAlert) -> NSApplication.ModalResponse)?
    private var documentErrorPresenter: ((Error) -> Void)?
    private var configureDocumentForCheck: ((DocumentViewController) -> Void)?
    private var lifecycleSelfCheck: String? {
        ["--document-lifecycle-self-check", "--document-recovery-writer-self-check",
         "--document-recovery-reader-self-check", "--document-recent-reader-self-check"]
            .first { CommandLine.arguments.contains($0) }
    }
    private var isolatedLifecycleCheck: Bool {
        lifecycleSelfCheck != nil && Bundle.main.bundleIdentifier?.hasPrefix("io.github.xiaodou997.yu.lifecycle-check.") == true
            && ProcessInfo.processInfo.environment["YU_DOCUMENT_STATE_DIR"] != nil
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let path: String
        renderRegression = CommandLine.arguments.contains("--render-regression-self-check")
        resourceRegression = CommandLine.arguments.contains("--resource-latency-self-check")
        launchSelfCheck = CommandLine.arguments.contains("--launch-window-self-check") || CommandLine.arguments.contains("--layout-coordinator-self-check") || CommandLine.arguments.contains("--window-state-self-check") || CommandLine.arguments.contains("--idle-resource-self-check") || CommandLine.arguments.contains("--presentation-latency-self-check") || CommandLine.arguments.contains("--zoom-latency-self-check") || CommandLine.arguments.contains("--redraw-latency-self-check") || renderRegression || resourceRegression || lifecycleSelfCheck != nil
        darkModeSelfCheck = CommandLine.arguments.contains("--dark-mode-self-check")
        // 冒烟/截图用的显式外观开关：默认跟随系统，不参与 self-check。
        forceDarkMode = CommandLine.arguments.contains("--dark-mode")
        if let argument = CommandLine.arguments.dropFirst().first(where: { !$0.hasPrefix("-") }) {
            path = URL(fileURLWithPath: argument).path
        } else {
            recoverPendingDocuments()
            if let restored = documents.keys.first {
                restored.makeKeyAndOrderFront(nil)
                return
            }
            newDocument(nil)
            return
        }

        if !launchSelfCheck && ProcessInfo.processInfo.environment["YU_VISUAL_CAPTURE_DIR"] == nil {
            _ = openDocument(at: URL(fileURLWithPath: path))
            recoverPendingDocuments()
            if documents.isEmpty { newDocument(nil) }
            return
        }
        do {
            let bridge = try StorageBridge(path: path)
            let window = presentDocument(bridge: bridge)
            guard let controller = documents[window] else { throw CocoaError(.coderInvalidValue) }
            if !launchSelfCheck && ProcessInfo.processInfo.environment["YU_VISUAL_CAPTURE_DIR"] == nil {
                DispatchQueue.main.async { [weak self] in self?.recoverPendingDocuments() }
            }
            if let output = ProcessInfo.processInfo.environment["YU_VISUAL_CAPTURE_DIR"] {
                Task { @MainActor in
                    do {
                        try await controller.captureVisualAcceptance(to: URL(fileURLWithPath: output))
                        print("Yu visual capture: actual screenshots written; Yu baseline review pending")
                        NSApp.terminate(nil)
                    } catch {
                        fputs("Yu visual capture failed: \(error)\n", stderr)
                        exit(EXIT_FAILURE)
                    }
                }
            }
            print("Yu document host opened path=\(bridge.path) revision=\(bridge.revision)")
            if launchSelfCheck {
                // Give AppKit one complete appearance/layout turn. This is a
                // real window smoke test: the native host must be visible
                // before asynchronous surface preparation is exercised.
                DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(500)) {
                    guard window.isVisible else {
                        fputs("Yu launch self-check failed: window is not visible\n", stderr)
                        exit(EXIT_FAILURE)
                    }
                    if self.darkModeSelfCheck,
                       window.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) != .darkAqua {
                        fputs("Yu dark-mode self-check failed: window is not Dark Aqua\n", stderr)
                        exit(EXIT_FAILURE)
                    }
                    if self.darkModeSelfCheck {
                        // 视觉 token 必须是真动态：darkAqua 下解析出来仍要深、
                        // aqua 下仍要浅，否则侧栏/rail/状态栏在深色模式下保持
                        // 浅色（cgColor/layer 把颜色快照死的那类回归，截图才
                        // 看得见，自动化必须替人眼先挡一道）。
                        let dark = NSAppearance(named: .darkAqua)!
                        let light = NSAppearance(named: .aqua)!
                        func luma(_ token: NSColor, _ appearance: NSAppearance) -> Double {
                            Double(
                                YuVisualTokens.luminanceForSelfCheck(of: token, appearance: appearance)
                                    ?? 0
                            )
                        }
                        guard luma(YuVisualTokens.canvas, dark) < 0.5,
                              luma(YuVisualTokens.accentSoft, dark) < 0.6,
                              luma(YuVisualTokens.canvas, light) > 0.9 else {
                            fputs(
                                "Yu dark-mode self-check failed: visual tokens did not resolve for appearance\n",
                                stderr
                            )
                            exit(EXIT_FAILURE)
                        }
                    }
                    let saveItem = NSMenuItem(title: "保存", action: #selector(DocumentViewController.saveFromMenu(_:)), keyEquivalent: "s")
                    let reloadItem = NSMenuItem(title: "重新加载", action: #selector(DocumentViewController.reloadFromMenu(_:)), keyEquivalent: "")
                    guard window.toolbar != nil,
                          window.titleVisibility == .visible,
                          window.representedURL?.standardizedFileURL == URL(fileURLWithPath: bridge.path).standardizedFileURL,
                          !controller.validateMenuItem(saveItem),
                          !controller.validateMenuItem(reloadItem) else {
                        let state = bridge.state
                        fputs("Yu window self-check failed: clean document chrome or menu validation toolbar=\(window.toolbar != nil) dirty=\(state.dirty) revision=\(state.revision) saved=\(state.savedRevision) disk=\(state.disk) save=\(controller.validateMenuItem(saveItem)) reload=\(controller.validateMenuItem(reloadItem))\n", stderr)
                        exit(EXIT_FAILURE)
                    }
                    print("Yu launch self-check: window appeared and remained stable")
                    Task { @MainActor in
                        do {
                            if let mode = self.lifecycleSelfCheck {
                                try await self.runDocumentLifecycleCheck(mode: mode)
                                if mode == "--document-recovery-writer-self-check" { return }
                            } else if CommandLine.arguments.contains("--window-state-self-check") {
                                try await controller.runWindowStateSelfCheck()
                            } else if CommandLine.arguments.contains("--idle-resource-self-check") {
                                try await controller.runIdleResourceSelfCheck()
                            } else if CommandLine.arguments.contains("--presentation-latency-self-check") || CommandLine.arguments.contains("--zoom-latency-self-check") || CommandLine.arguments.contains("--redraw-latency-self-check") {
                                try await controller.runPresentationLatencySelfCheck(zoom: CommandLine.arguments.contains("--zoom-latency-self-check"), redraw: CommandLine.arguments.contains("--redraw-latency-self-check"))
                            } else if self.renderRegression || self.resourceRegression {
                                var longRegressionError: Error?
                                do {
                                    try await controller.runRenderRegressionSelfCheck(resources: self.resourceRegression)
                                } catch {
                                    if !self.renderRegression { throw error }
                                    longRegressionError = error
                                }
                                if self.renderRegression {
                                    // Exercise the actual window-close delegate, then a
                                    // fresh session/window (not just layer reattachment).
                                    window.performClose(nil)
                                    let next = DocumentViewController(bridge: try StorageBridge(path: path))
                                    let reopened = NSWindow(contentViewController: next)
                                    reopened.setContentSize(NSSize(width: 900, height: 620))
                                    reopened.delegate = self
                                    reopened.isReleasedWhenClosed = false
                                    self.controller = next
                                    self.window = reopened
                                    reopened.makeKeyAndOrderFront(nil)
                                    next.focusDocument()
                                    try await Task.sleep(nanoseconds: 500_000_000)
                                    try await next.runRenderRegressionSelfCheck(resources: false, reopened: true)
                                }
                                if let error = longRegressionError { throw error }
                            } else if CommandLine.arguments.contains("--layout-coordinator-self-check") {
                                try await controller.runLayoutCoordinatorSelfCheck()
                            } else {
                                try await controller.runFrameSchedulingSelfCheck()
                            }
                        } catch {
                            fputs("Yu frame scheduling self-check failed: \(error)\n", stderr)
                            exit(EXIT_FAILURE)
                        }
                        NSApp.terminate(nil)
                    }
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

    private func identity(_ url: URL) -> URL { url.standardizedFileURL.resolvingSymlinksInPath() }

    private func existingWindow(for url: URL) -> NSWindow? {
        documents.first { identity($0.value.documentURL) == identity(url) }?.key
    }

    @discardableResult
    private func presentDocument(bridge: StorageBridge, recovered: Bool = false) -> NSWindow {
        let controller = DocumentViewController(bridge: bridge, recovered: recovered)
        configureDocumentForCheck?(controller)
        let window = NSWindow(contentViewController: controller)
        window.styleMask = [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView]
        window.setContentSize(NSSize(width: 900, height: 620))
        window.titleVisibility = .visible
        window.title = controller.persistence.isUntitled ? "未命名" : controller.documentURL.lastPathComponent
        window.representedURL = controller.persistence.isUntitled ? nil : controller.documentURL
        window.toolbarStyle = .unified
        // AppKit owns chrome appearance independently from Github/Night.
        // A transparent titlebar over a dark reading canvas otherwise leaves
        // system light-appearance titles and controls without readable contrast.
        window.titlebarAppearsTransparent = false
        window.titlebarSeparatorStyle = .none
        window.backgroundColor = .windowBackgroundColor
        if darkModeSelfCheck || forceDarkMode { window.appearance = NSAppearance(named: .darkAqua) }
        window.delegate = self
        window.isReleasedWhenClosed = false
        window.isRestorable = !launchSelfCheck && !controller.persistence.isUntitled
            && ProcessInfo.processInfo.environment["YU_VISUAL_CAPTURE_DIR"] == nil
        window.restorationClass = NativeWindowRestorer.self
        window.identifier = NSUserInterfaceItemIdentifier(bridge.path)
        controller.onOpenDocument = { [weak self] next in _ = self?.openDocument(at: next) }
        controller.onValidateSaveDestination = { [weak self, weak window, weak controller] destination in
            guard let self, let controller else { throw CocoaError(.userCancelled) }
            if let existing = self.existingWindow(for: destination), existing !== window {
                throw NSError(domain: "Yu.Document", code: 1,
                    userInfo: [NSLocalizedDescriptionKey: "这个文件已在另一个 Yu 窗口中打开，请选择其他位置。"])
            }
            if self.identity(destination) != self.identity(controller.documentURL),
               try self.pendingRecovery(for: destination) != nil {
                throw NSError(domain: "Yu.Document", code: 2,
                    userInfo: [NSLocalizedDescriptionKey: "这个位置有未处理的恢复副本。请先从“恢复未保存的文档”处理，或选择其他位置。"])
            }
        }
        controller.onDocumentURLChange = { [weak self, weak window, weak controller] in
            guard let self, let window, let controller else { return }
            window.isRestorable = !self.launchSelfCheck && ProcessInfo.processInfo.environment["YU_VISUAL_CAPTURE_DIR"] == nil
            self.noteRecentDocument(controller.documentURL)
        }
        documents[window] = controller
        self.window = window
        self.controller = controller
        if !controller.persistence.isUntitled { noteRecentDocument(controller.documentURL) }
        window.center()
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        installMainMenu(for: controller)
        controller.focusDocument()
        return window
    }

    private func noteRecentDocument(_ url: URL) {
        guard (!launchSelfCheck || isolatedLifecycleCheck), ProcessInfo.processInfo.environment["YU_VISUAL_CAPTURE_DIR"] == nil else { return }
        NSDocumentController.shared.noteNewRecentDocumentURL(url)
        installMainMenu(for: controller)
    }

    private func pendingRecovery(for url: URL) throws -> URL? {
        let locations = NativeDocumentLocations.current
        let exact = try StorageBridge.recoveryURL(for: identity(url), in: locations.recovery)
        if FileManager.default.fileExists(atPath: exact.path) { return exact }
        // Older user-facing aliases can refer to the same canonical file.
        for record in try locations.recoveryFiles() {
            if let target = try? StorageBridge.recoveryTarget(at: record), identity(target) == identity(url) {
                return record
            }
        }
        return nil
    }

    private func recoveryResponse(for target: URL) -> NSApplication.ModalResponse {
        let alert = NSAlert()
        alert.messageText = "恢复未保存的文档？"
        alert.informativeText = "\(NativeDocumentLocations.current.isDraft(target) ? "未命名文档" : target.path)\n恢复不会立即覆盖磁盘文件；确认内容后请手动保存。"
        alert.addButton(withTitle: "恢复")
        alert.addButton(withTitle: "丢弃恢复副本")
        alert.addButton(withTitle: "稍后")
        return recoveryAlertDecision?(alert) ?? alert.runModal()
    }

    @discardableResult
    func openDocument(at url: URL) -> NSWindow? {
        if let existing = existingWindow(for: url) {
            existing.makeKeyAndOrderFront(nil)
            documents[existing]?.focusDocument()
            return existing
        }
        do {
            if let record = try pendingRecovery(for: url) {
                consideredRecoveryFiles.insert(record)
                if let target = try? StorageBridge.recoveryTarget(at: record), identity(target) == identity(url) {
                    switch recoveryResponse(for: target) {
                    case .alertFirstButtonReturn:
                        return presentDocument(bridge: try StorageBridge(path: record.path, mode: .recovery), recovered: true)
                    case .alertSecondButtonReturn:
                        try FileManager.default.removeItem(at: record)
                    default: return nil // Do not overwrite a deferred checkpoint with a fresh session.
                    }
                } else {
                    let alert = NSAlert()
                    alert.messageText = "这个文档的恢复副本无法读取"
                    alert.informativeText = "可以保留损坏副本用于后续检查，再打开磁盘文件。副本不会被删除。"
                    alert.addButton(withTitle: "保留副本并打开")
                    alert.addButton(withTitle: "取消")
                    guard (recoveryAlertDecision?(alert) ?? alert.runModal()) == .alertFirstButtonReturn else { return nil }
                    let archive = NativeDocumentLocations.current.recovery.appendingPathComponent("Invalid", isDirectory: true)
                    try FileManager.default.createDirectory(at: archive, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
                    try FileManager.default.moveItem(at: record, to: archive.appendingPathComponent(UUID().uuidString + "-" + record.lastPathComponent))
                }
            }
            return presentDocument(bridge: try StorageBridge(path: identity(url).path))
        } catch {
            if let documentErrorPresenter { documentErrorPresenter(error) }
            else { NSAlert(error: error).runModal() }
            return nil
        }
    }

    @objc private func newDocument(_ sender: Any?) {
        do {
            let draft = try NativeDocumentLocations.current.newDraftURL()
            presentDocument(bridge: try StorageBridge(path: draft.path, mode: .untitled))
        } catch { NSAlert(error: error).runModal() }
    }

    @objc private func openRecentDocument(_ sender: NSMenuItem) {
        guard let url = sender.representedObject as? URL else { return }
        _ = openDocument(at: url)
    }

    @objc private func clearRecentDocuments(_ sender: Any?) {
        NSDocumentController.shared.clearRecentDocuments(sender)
        installMainMenu(for: controller)
    }

    @objc private func toggleAutosave(_ sender: Any?) {
        NativeDocumentPersistence.autosaveEnabled.toggle()
        for controller in documents.values { controller.persistence.documentChanged() }
        installMainMenu(for: controller)
    }

    @objc private func recoverDocuments(_ sender: Any?) {
        consideredRecoveryFiles.removeAll()
        recoverPendingDocuments()
    }

    private func recoverPendingDocuments() {
        guard (!launchSelfCheck || isolatedLifecycleCheck), ProcessInfo.processInfo.environment["YU_VISUAL_CAPTURE_DIR"] == nil else { return }
        do {
            for record in try NativeDocumentLocations.current.recoveryFiles() where !consideredRecoveryFiles.contains(record) {
                consideredRecoveryFiles.insert(record)
                do {
                    let target = try StorageBridge.recoveryTarget(at: record)
                    if let existing = existingWindow(for: target), documents[existing]?.persistence.bridge.state.dirty == true {
                        let alert = NSAlert()
                        alert.messageText = "恢复副本已保留"
                        alert.informativeText = "“\(existing.title)”当前有未保存修改。请先处理该窗口，再从“文件 → 恢复未保存的文档”打开恢复副本。"
                        _ = recoveryAlertDecision?(alert) ?? alert.runModal()
                        continue
                    }
                    switch recoveryResponse(for: target) {
                    case .alertFirstButtonReturn:
                        // Read successfully before replacing a clean window.
                        let bridge = try StorageBridge(path: record.path, mode: .recovery)
                        if let existing = existingWindow(for: target) {
                            documents[existing]?.persistence.suspend()
                            existing.close()
                        }
                        let window = presentDocument(bridge: bridge, recovered: true)
                        try documents[window]?.persistence.flushRecovery()
                    case .alertSecondButtonReturn:
                        try FileManager.default.removeItem(at: record)
                    default: break
                    }
                } catch {
                    let alert = NSAlert(error: error)
                    alert.messageText = "无法读取恢复副本"
                    alert.informativeText = "恢复文件已保留：\(record.path)\n\(error.localizedDescription)"
                    if let documentErrorPresenter { documentErrorPresenter(error) }
                    else { alert.runModal() }
                }
            }
        } catch {
            if let documentErrorPresenter { documentErrorPresenter(error) }
            else { NSAlert(error: error).runModal() }
        }
    }

    func application(_ application: NSApplication, open urls: [URL]) {
        for url in urls { _ = openDocument(at: url) }
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag { newDocument(nil) }
        return true
    }

    func applicationWillResignActive(_ notification: Notification) {
        guard !launchSelfCheck else { return }
        for controller in documents.values where !controller.persistence.suspended {
            controller.persistence.checkpoint(allowAutosave: false)
        }
    }

    @objc private func openFromMenu(_ sender: Any?) {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = true
        guard panel.runModal() == .OK else { return }
        for url in panel.urls { openDocument(at: url) }
    }

    func windowDidBecomeKey(_ notification: Notification) {
        guard let window = notification.object as? NSWindow,
              let controller = documents[window] else { return }
        self.window = window
        self.controller = controller
        installMainMenu(for: controller)
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        guard let controller = documents[sender] else { return true }
        guard controller.requestClose() else { return false }
        do { try controller.finalizeCloseRequest() }
        catch {
            controller.cancelCloseRequest()
            NSAlert(error: error).runModal()
            return false
        }
        controller.detachSurfaceHost()
        return true
    }

    func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool { true }

    func window(_ window: NSWindow, willEncodeRestorableState state: NSCoder) {
        documents[window]?.encodeWindowState(to: state)
    }

    func window(_ window: NSWindow, didDecodeRestorableState state: NSCoder) {
        documents[window]?.restoreWindowState(from: state)
    }

    func windowWillClose(_ notification: Notification) {
        guard let window = notification.object as? NSWindow else { return }
        let closed = documents.removeValue(forKey: window)
        closed?.persistence.suspend()
        closed?.detachSurfaceHost()
        if self.window === window {
            self.window = documents.keys.first
            self.controller = self.window.flatMap { documents[$0] }
            installMainMenu(for: self.controller)
        }
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        let controllers = Array(documents.values)
        for controller in controllers {
            controller.persistence.suspend()
            try? controller.persistence.flushRecovery()
        }
        for controller in controllers {
            if !controller.requestClose() {
                for item in controllers { item.cancelCloseRequest() }
                return .terminateCancel
            }
        }
        do {
            for controller in controllers { try controller.finalizeCloseRequest() }
        } catch {
            for controller in controllers { controller.cancelCloseRequest() }
            NSAlert(error: error).runModal()
            return .terminateCancel
        }
        for controller in controllers { controller.detachSurfaceHost() }
        return .terminateNow
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }

    private func installMainMenu(for controller: DocumentViewController?) {
        let mainMenu = NSMenu()

        let appMenuItem = NSMenuItem()
        let appMenu = NSMenu(title: "Yu")
        appMenu.addItem(
            withTitle: "关于 Yu",
            action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)),
            keyEquivalent: ""
        )
        let settings = NSMenuItem(title: "设置…", action: #selector(showSettings(_:)), keyEquivalent: ",")
        settings.target = self
        appMenu.addItem(settings)
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
        fileMenu.addItem(withTitle: "新建", action: #selector(newDocument(_:)), keyEquivalent: "n").target = self
        fileMenu.addItem(withTitle: "打开…", action: #selector(openFromMenu(_:)), keyEquivalent: "o").target = self
        let save = NSMenuItem(
            title: "保存",
            action: #selector(DocumentViewController.saveFromMenu(_:)),
            keyEquivalent: "s"
        )
        save.target = controller
        fileMenu.addItem(save)
        let saveAs = NSMenuItem(title: "另存为…", action: #selector(DocumentViewController.saveAsFromMenu(_:)), keyEquivalent: "s")
        saveAs.keyEquivalentModifierMask = [.command, .shift]
        saveAs.target = controller
        fileMenu.addItem(saveAs)
        let insertImage = NSMenuItem(title: "插入图片…", action: #selector(DocumentViewController.insertImageFromMenu(_:)), keyEquivalent: "")
        insertImage.target = controller
        fileMenu.addItem(insertImage)
        let imageProperties = NSMenuItem(title: "图片属性…", action: #selector(DocumentViewController.editImagePropertiesFromMenu(_:)), keyEquivalent: "")
        imageProperties.target = controller
        fileMenu.addItem(imageProperties)
        let recent = NSMenu(title: "最近打开")
        for url in NSDocumentController.shared.recentDocumentURLs {
            let item = NSMenuItem(title: url.lastPathComponent, action: #selector(openRecentDocument(_:)), keyEquivalent: "")
            item.representedObject = url
            item.toolTip = url.path
            item.target = self
            recent.addItem(item)
        }
        recent.addItem(.separator())
        recent.addItem(withTitle: "清除菜单", action: #selector(clearRecentDocuments(_:)), keyEquivalent: "").target = self
        let recentItem = NSMenuItem(title: "最近打开", action: nil, keyEquivalent: "")
        recentItem.submenu = recent
        fileMenu.addItem(recentItem)
        let autosave = NSMenuItem(title: "自动保存", action: #selector(toggleAutosave(_:)), keyEquivalent: "")
        autosave.target = self
        autosave.state = NativeDocumentPersistence.autosaveEnabled ? .on : .off
        fileMenu.addItem(autosave)
        fileMenu.addItem(withTitle: "恢复未保存的文档…", action: #selector(recoverDocuments(_:)), keyEquivalent: "").target = self
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
            action: #selector(NSWindow.performClose(_:)),
            keyEquivalent: "w"
        )
        close.target = nil
        fileMenu.addItem(close)
        fileMenuItem.submenu = fileMenu
        mainMenu.addItem(fileMenuItem)

        let editMenuItem = NSMenuItem()
        let editMenu = NSMenu(title: "编辑")
        // Standard nil-target actions follow the current native responder,
        // including remote file panels, search fields and settings. Yu's own
        // NSTextInputClient handles the same selectors through Rust below.
        let editItems: [(String, Selector, String)] = [
            ("撤销", NSSelectorFromString("undo:"), "z"),
            ("重做", NSSelectorFromString("redo:"), "Z"),
            ("剪切", #selector(DocumentTextView.cut(_:)), "x"),
            ("复制", #selector(DocumentTextView.copy(_:)), "c"),
            ("粘贴", #selector(DocumentTextView.paste(_:)), "v"),
            ("全选", #selector(DocumentTextView.selectAll(_:)), "a"),
        ]
        for (title, action, keyEquivalent) in editItems.prefix(2) {
            let item = NSMenuItem(title: title, action: action, keyEquivalent: keyEquivalent)
            item.target = nil
            editMenu.addItem(item)
        }
        editMenu.addItem(.separator())
        for (title, action, keyEquivalent) in editItems.dropFirst(2) {
            let item = NSMenuItem(title: title, action: action, keyEquivalent: keyEquivalent)
            item.target = nil
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

        let tableMenuItem = NSMenuItem()
        tableMenuItem.submenu = controller?.makeTableMenu() ?? NSMenu(title: "表格")
        mainMenu.addItem(tableMenuItem)

        let viewMenuItem = NSMenuItem()
        let viewMenu = NSMenu(title: "显示")
        let sourceMode = NSMenuItem(title: "源码模式", action: #selector(DocumentViewController.toggleSourceMode(_:)), keyEquivalent: "m")
        sourceMode.keyEquivalentModifierMask = [.command, .shift]
        sourceMode.target = controller
        viewMenu.addItem(sourceMode)
        let disclosure = NSMenuItem(title: "展开／折叠当前摘要", action: #selector(DocumentTextView.toggleDisclosureFromMenu(_:)), keyEquivalent: "d")
        disclosure.keyEquivalentModifierMask = [.command, .option, .control]
        viewMenu.addItem(disclosure)
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
        viewMenu.addItem(.separator())
        for (title, action, key) in [
            ("放大", #selector(DocumentViewController.zoomInFromMenu(_:)), "+"),
            ("缩小", #selector(DocumentViewController.zoomOutFromMenu(_:)), "-"),
            ("实际大小", #selector(DocumentViewController.resetZoomFromMenu(_:)), "0")
        ] {
            let item = NSMenuItem(title: title, action: action, keyEquivalent: key)
            item.target = controller
            viewMenu.addItem(item)
        }
        viewMenuItem.submenu = viewMenu
        mainMenu.addItem(viewMenuItem)

        NSApp.mainMenu = mainMenu
    }
}

/// Restore native UI state, never a second serialized document.
final class NativeWindowRestorer: NSObject, NSWindowRestoration {
    static func restoreWindow(withIdentifier identifier: NSUserInterfaceItemIdentifier,
                              state: NSCoder, completionHandler: @escaping (NSWindow?, Error?) -> Void) {
        // Automation has explicit fixtures and must not inherit or overwrite
        // the user's saved windows (including mode and scroll position).
        guard !CommandLine.arguments.contains(where: { $0.hasSuffix("-self-check") }),
              ProcessInfo.processInfo.environment["YU_VISUAL_CAPTURE_DIR"] == nil else {
            completionHandler(nil, nil)
            return
        }
        guard let path = state.decodeObject(of: NSString.self, forKey: "Yu.documentPath") as String?,
              let delegate = NSApp.delegate as? AppDelegate else {
            completionHandler(nil, CocoaError(.fileReadCorruptFile))
            return
        }
        guard FileManager.default.fileExists(atPath: path),
              let window = delegate.openDocument(at: URL(fileURLWithPath: path)) else {
            completionHandler(nil, CocoaError(.fileReadNoSuchFile))
            return
        }
        completionHandler(window, nil)
    }
}

@MainActor
private enum NativeWindowCapture {
    static func image(_ window: NSWindow) async throws -> CGImage {
        let content = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: false)
        guard let target = content.windows.first(where: { $0.windowID == CGWindowID(window.windowNumber) }) else {
            throw CocoaError(.fileReadUnknown)
        }
        let filter = SCContentFilter(desktopIndependentWindow: target)
        let configuration = SCStreamConfiguration()
        configuration.width = Int((window.frame.width * window.backingScaleFactor).rounded())
        configuration.height = Int((window.frame.height * window.backingScaleFactor).rounded())
        configuration.showsCursor = false
        configuration.capturesAudio = false
        configuration.ignoreShadowsSingleWindow = true
        return try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: configuration)
    }
}

// Runs only in a separately identified test bundle, with an isolated state
// directory. Native panels receive injected user decisions; file/session/menu
// actions and the autosave scheduler are the production implementations.
extension AppDelegate {
    @MainActor
    private func runDocumentLifecycleCheck(mode: String) async throws {
        func require(_ value: Bool, _ message: String) throws {
            if !value { throw NSError(domain: "Yu.DocumentCheck", code: 1,
                userInfo: [NSLocalizedDescriptionKey: message]) }
        }
        try require(isolatedLifecycleCheck, "Use the isolated lifecycle test runner")
        guard let initialWindow = window, let initial = documents[initialWindow],
              let resultPath = ProcessInfo.processInfo.environment["YU_DOCUMENT_CHECK_RESULT"] else {
            throw CocoaError(.coderInvalidValue)
        }
        let locations = NativeDocumentLocations.current
        try locations.prepare()
        let files = locations.root.appendingPathComponent("Files", isDirectory: true)
        try FileManager.default.createDirectory(at: files, withIntermediateDirectories: true)
        var errors = [String]()
        configureDocumentForCheck = { controller in
            controller.closeAlertDecision = { _ in .alertThirdButtonReturn }
            controller.externalAlertDecision = { _ in .alertSecondButtonReturn }
            controller.fileErrorPresenter = { errors.append($0.localizedDescription) }
        }
        configureDocumentForCheck?(initial)
        documentErrorPresenter = { errors.append($0.localizedDescription) }
        NativeDocumentPersistence.autosaveEnabled = false
        var checks = [String]()
        func record(_ name: String) { checks.append(name); print("Yu file lifecycle: \(name)"); fflush(stdout) }
        func insert(_ text: String, into controller: DocumentViewController) {
            controller.withFileInputForSelfCheck {
                $0.insertText(text, replacementRange: NSRange(location: NSNotFound, length: 0))
            }
        }
        func fixture(_ name: String, _ text: String) throws -> (NSWindow, DocumentViewController) {
            let url = files.appendingPathComponent(name)
            try Data(text.utf8).write(to: url, options: .atomic)
            let next = presentDocument(bridge: try StorageBridge(path: url.path))
            return (next, documents[next]!)
        }
        func finishResult(_ extra: [String: Any] = [:]) throws {
            var result: [String: Any] = ["mode": mode, "checks": checks, "passed": true,
                "bundle_id": Bundle.main.bundleIdentifier ?? "", "state_root": locations.root.path]
            for (key, value) in extra { result[key] = value }
            try JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted, .sortedKeys])
                .write(to: URL(fileURLWithPath: resultPath), options: .atomic)
        }

        if mode == "--document-recent-reader-self-check" {
            let recent = NSDocumentController.shared.recentDocumentURLs.map(identity)
            try require(recent.contains(identity(files.appendingPathComponent("close-conflict-copy.md"))), "Recent Save As destination did not survive process exit")
            record("recent document persisted across processes")
            clearRecentDocuments(nil)
            try require(NSDocumentController.shared.recentDocumentURLs.isEmpty, "Clear Recent Documents did not clear native storage")
            record("native recent-document clearing")
            try finishResult()
            return
        }

        if mode == "--document-recovery-writer-self-check" {
            try require(initial.persistence.bridge.source == "原稿\r\n", "Incorrect crash fixture")
            insert("本地新增🙂\r\n", into: initial)
            try initial.persistence.flushRecovery()
            newDocument(nil)
            guard let draftWindow = window, let draft = documents[draftWindow] else { throw CocoaError(.coderInvalidValue) }
            insert("未命名恢复中文🙂\n", into: draft)
            try draft.persistence.flushRecovery()
            let (_, deleted) = try fixture("deleted.md", "将被删除\r\n")
            insert("本地", into: deleted)
            try deleted.persistence.flushRecovery()
            try require(!FileManager.default.fileExists(atPath: draft.documentURL.path), "Untitled checkpoint created a user document")
            record("three crash checkpoints persisted before SIGKILL")
            try finishResult(["named_path": initial.documentURL.path, "draft_path": draft.documentURL.path,
                "deleted_path": deleted.documentURL.path])
            // The runner kills this live process after observing the atomic
            // ready record. Do not terminate gracefully or clear recovery here.
            return
        }

        if mode == "--document-recovery-reader-self-check" {
            let recordsBefore = try locations.recoveryFiles()
            try require(recordsBefore.count == 4, "Expected three valid records and one corrupt record")
            recoveryAlertDecision = { _ in .alertThirdButtonReturn }
            recoverDocuments(nil)
            try require(documents.count == 1 && (try locations.recoveryFiles()).count == 4, "Later must retain recovery without opening documents")
            record("defer recovery preserves every record")
            recoveryAlertDecision = { _ in .alertFirstButtonReturn }
            recoverDocuments(nil)
            let restored = documents.values.filter { $0.persistence.needsRecoveryReview }
            try require(restored.count == 3, "Valid recovery was blocked by another corrupt record")
            for controller in restored {
                let bridge = controller.persistence.bridge
                try require(bridge.state.dirty, "Recovered source must remain unsaved")
                controller.persistence.checkpoint(allowAutosave: true)
                if controller.persistence.isUntitled {
                    try require(bridge.source == "未命名恢复中文🙂\n", "Untitled recovery source mismatch")
                    try require(!FileManager.default.fileExists(atPath: controller.documentURL.path), "Recovered draft autosaved to an internal file")
                } else if controller.documentURL.lastPathComponent == "deleted.md" {
                    try require(bridge.source == "本地将被删除\r\n" && bridge.state.disk == .missing, "Deleted-file recovery mismatch")
                    try require(!FileManager.default.fileExists(atPath: controller.documentURL.path), "Recovery recreated a deleted original")
                } else {
                    try require(bridge.source == "本地新增🙂\r\n原稿\r\n" && bridge.state.bom, "Named recovery lost source or BOM")
                    try require(bridge.state.disk == .changed, "Recovery lost pre-crash disk fingerprint")
                    try require(try Data(contentsOf: controller.documentURL) == Data("外部版本\r\n".utf8), "Recovery overwrote external changes")
                }
            }
            try require(!errors.isEmpty, "Corrupt recovery was not reported")
            record("SIGKILL recovery: named, untitled, deleted, BOM and external conflict")
            record("recovered documents require manual save; corrupt record retained")
            if let draft = restored.first(where: { $0.persistence.isUntitled }) {
                let destination = files.appendingPathComponent("recovered-draft.md")
                try draft.saveDocumentAs(to: destination, replaceExisting: false)
                try require(try Data(contentsOf: destination) == Data("未命名恢复中文🙂\n".utf8), "Recovered draft Save As mismatch")
                try require(!draft.persistence.needsRecoveryReview, "Manual save did not resume normal persistence")
            }
            let (discardWindow, discard) = try fixture("discard-recovery.md", "磁盘保留")
            insert("恢复副本", into: discard)
            try discard.persistence.flushRecovery()
            let discardRecord = try discard.persistence.bridge.recoveryURL(in: locations.recovery)
            discard.persistence.suspend()
            discardWindow.close() // Simulate an orphaned checkpoint, not user discard.
            recoveryAlertDecision = { alert in
                alert.informativeText.contains("discard-recovery.md") ? .alertSecondButtonReturn : .alertThirdButtonReturn
            }
            recoverDocuments(nil)
            try require(!FileManager.default.fileExists(atPath: discardRecord.path), "Discard did not remove recovery envelope")
            try require(try Data(contentsOf: files.appendingPathComponent("discard-recovery.md")) == Data("磁盘保留".utf8), "Discard changed the original document")
            record("explicit discard removes only the selected recovery record")
            for controller in documents.values { controller.closeAlertDecision = { _ in .alertSecondButtonReturn } }
            try finishResult(["reported_recovery_errors": errors.count])
            return
        }

        // New, initial save cancellation, overwrite consent and history.
        newDocument(nil)
        guard let draftWindow = window, let draft = documents[draftWindow] else { throw CocoaError(.coderInvalidValue) }
        let draftPath = draft.documentURL
        try require(draftWindow.title == "未命名" && draftWindow.representedURL == nil, "New-document chrome is not untitled")
        insert("# 草稿\r\n正文🙂", into: draft)
        draft.savePanelDecision = { _ in nil }
        try require(!draft.saveDocument() && draft.persistence.isUntitled, "Cancelled first save lost untitled state")
        try require(!FileManager.default.fileExists(atPath: draftPath.path), "New created a backing Markdown file")
        let first = files.appendingPathComponent("first.md")
        draft.savePanelDecision = { _ in first }
        try require(draft.saveDocument(), "First save failed")
        let expected = Data("# 草稿\r\n正文🙂".utf8)
        try require(try Data(contentsOf: first) == expected, "First save bytes mismatch")
        try require(draftWindow.representedURL == first && draftWindow.title == "first.md", "First save did not retarget window")
        record("new, cancelled Save panel, first save and document identity")
        let renamed = files.appendingPathComponent("renamed.md")
        try Data("existing target".utf8).write(to: renamed)
        let revision = draft.persistence.bridge.revision
        do { try draft.saveDocumentAs(to: renamed, replaceExisting: false); throw CocoaError(.coderInvalidValue) }
        catch BridgeError.operation(let status) { try require(status == StorageStatus.externalChange, "Unexpected overwrite refusal") }
        try require(draft.documentURL == first, "Rejected overwrite changed window path")
        draft.savePanelDecision = { _ in renamed }
        draft.saveAsFromMenu(nil)
        try require(draft.documentURL == renamed && draft.persistence.bridge.revision == revision, "Save As changed revision or failed identity change")
        try require(try Data(contentsOf: first) == expected && Data(contentsOf: renamed) == expected, "Save As changed original or output bytes")
        draft.withFileInputForSelfCheck { $0.performUndo() }
        try require(draft.persistence.bridge.source.isEmpty, "Save As discarded undo history")
        draft.withFileInputForSelfCheck { $0.performRedo() }
        try require(draft.persistence.bridge.source == "# 草稿\r\n正文🙂", "Save As discarded redo history")
        try require(draft.saveDocument(), "Save after redo failed")
        record("Save As overwrite consent, original bytes and shared undo/redo")

        let originalURL = initial.documentURL
        let originalBytes = try Data(contentsOf: originalURL)
        insert("新增", into: initial)
        try initial.saveDocumentAs(to: files.appendingPathComponent("bom-copy.md"), replaceExisting: false)
        try require(try Data(contentsOf: originalURL) == originalBytes, "BOM Save As modified original")
        try require(try Data(contentsOf: initial.documentURL) == Data([0xef, 0xbb, 0xbf]) + Data("新增原稿\r\n".utf8), "BOM/CRLF was not preserved")
        let alias = files.appendingPathComponent("alias.md")
        try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: renamed)
        let count = documents.count
        try require(openDocument(at: alias) === draftWindow && documents.count == count, "Alias opened a duplicate session")
        do { try initial.saveDocumentAs(to: renamed, replaceExisting: true); throw CocoaError(.coderInvalidValue) }
        catch let error as NSError { try require(error.domain == "Yu.Document", "Save As did not reject another window's target") }
        draftWindow.makeKeyAndOrderFront(nil)
        let fileMenu = NSApp.mainMenu?.items.first(where: { $0.submenu?.title == "文件" })?.submenu
        try require((fileMenu?.items.first(where: { $0.title == "保存" })?.target as? DocumentViewController) === draft, "Save menu targets the wrong window")
        let recentMenu = fileMenu?.items.first(where: { $0.title == "最近打开" })?.submenu
        guard let recentItem = recentMenu?.items.first(where: { ($0.representedObject as? URL).map(identity) == identity(renamed) }) else { throw CocoaError(.coderInvalidValue) }
        openRecentDocument(recentItem)
        try require(documents.count == count, "Open Recent duplicated a window")
        record("BOM/CRLF, canonical-path deduplication, active menu and Open Recent")

        // A deferred checkpoint must survive both Open and Save As attempts.
        let (deferredWindow, deferred) = try fixture("deferred.md", "磁盘原文")
        insert("待恢复", into: deferred)
        try deferred.persistence.flushRecovery()
        let deferredURL = deferred.documentURL
        let deferredRecord = try deferred.persistence.bridge.recoveryURL(in: locations.recovery)
        let deferredBytes = try Data(contentsOf: deferredRecord)
        deferred.persistence.suspend()
        deferredWindow.close()
        recoveryAlertDecision = { _ in .alertThirdButtonReturn }
        try require(openDocument(at: deferredURL) == nil, "Later opened a session that could replace recovery")
        do { try initial.saveDocumentAs(to: deferredURL, replaceExisting: true); throw CocoaError(.coderInvalidValue) }
        catch let error as NSError { try require(error.domain == "Yu.Document" && error.code == 2, "Save As did not protect a pending checkpoint") }
        try require(try Data(contentsOf: deferredRecord) == deferredBytes, "Deferred checkpoint was changed")
        try require(try Data(contentsOf: deferredURL) == Data("磁盘原文".utf8), "Deferred target was overwritten")
        recoveryAlertDecision = { _ in .alertFirstButtonReturn }
        guard let restoredWindow = openDocument(at: deferredURL), let restored = documents[restoredWindow] else { throw CocoaError(.coderInvalidValue) }
        try require(restored.persistence.needsRecoveryReview && restored.persistence.bridge.source == "待恢复磁盘原文", "Open did not restore pending source")
        restored.closeAlertDecision = { _ in .alertSecondButtonReturn }
        restoredWindow.performClose(nil)
        record("deferred recovery survives Open and Save As, then restores original edits")

        // Corrupt records are archived byte-for-byte before a new session may checkpoint.
        let corruptBytes = Data("invalid recovery envelope".utf8)
        try corruptBytes.write(to: deferredRecord, options: .atomic)
        guard let cleanWindow = openDocument(at: deferredURL), let clean = documents[cleanWindow] else { throw CocoaError(.coderInvalidValue) }
        try require(clean.persistence.bridge.source == "磁盘原文", "Corrupt recovery replaced disk source")
        let archive = locations.recovery.appendingPathComponent("Invalid", isDirectory: true)
        let archived = try FileManager.default.contentsOfDirectory(at: archive, includingPropertiesForKeys: nil)
        try require(try archived.contains { try Data(contentsOf: $0) == corruptBytes }, "Corrupt recovery was not preserved")
        cleanWindow.performClose(nil)
        recoveryAlertDecision = nil
        record("corrupt checkpoint retained in archive before opening disk document")

        // Exercise the real debounce, then conflicts and marked-text protection.
        let (_, automatic) = try fixture("automatic.md", "基线")
        NativeDocumentPersistence.autosaveEnabled = true
        insert("自动保存", into: automatic)
        let deadline = Date().addingTimeInterval(4)
        while automatic.persistence.bridge.state.dirty && Date() < deadline { try await Task.sleep(nanoseconds: 20_000_000) }
        try require(!automatic.persistence.bridge.state.dirty, "Debounced autosave did not run")
        try require(try Data(contentsOf: automatic.documentURL) == Data("自动保存基线".utf8), "Autosave bytes mismatch")
        NativeDocumentPersistence.autosaveEnabled = false
        insert("保留", into: automatic)
        try await Task.sleep(nanoseconds: 1_200_000_000)
        try require(automatic.persistence.bridge.state.dirty, "Disabled autosave wrote the document")
        try require(try Data(contentsOf: automatic.documentURL) == Data("自动保存基线".utf8), "Disabled autosave changed disk")
        let recoveryPath = try automatic.persistence.bridge.recoveryURL(in: locations.recovery)
        try require(FileManager.default.fileExists(atPath: recoveryPath.path), "Disabled autosave stopped crash checkpoints")
        try Data("external".utf8).write(to: automatic.documentURL, options: .atomic)
        automatic.persistence.checkpoint(allowAutosave: true)
        try require(automatic.persistence.bridge.state.dirty && automatic.persistence.notice != nil, "Conflict did not retain dirty state and notice")
        try require(try Data(contentsOf: automatic.documentURL) == Data("external".utf8), "Autosave overwrote external data")
        try automatic.saveDocumentAs(to: files.appendingPathComponent("conflict-copy.md"), replaceExisting: false)
        record("real autosave debounce, disabled-save recovery and external-change protection")

        let (_, composition) = try fixture("composition.md", "base")
        insert("已提交", into: composition)
        composition.withFileInputForSelfCheck { $0.setMarkedText("候选", selectedRange: NSRange(location: 2, length: 0), replacementRange: NSRange(location: NSNotFound, length: 0)) }
        composition.persistence.checkpoint(allowAutosave: true)
        try require(composition.persistence.bridge.composition.active, "Autosave committed the IME overlay")
        try require(try Data(contentsOf: composition.documentURL) == Data("base".utf8), "Autosave wrote during preedit")
        let compositionRecord = try composition.persistence.bridge.recoveryURL(in: locations.recovery)
        let backup = try StorageBridge(path: compositionRecord.path, mode: .recovery)
        try require(backup.source == "已提交base", "Recovery included uncommitted preedit")
        try require(composition.saveDocument(), "Explicit composition Save failed")
        try require(composition.persistence.bridge.source == "已提交候选base", "Explicit save did not commit visible preedit once")
        composition.withFileInputForSelfCheck { $0.performUndo() }
        try require(composition.persistence.bridge.source == "已提交base", "Composition save lost atomic undo")
        composition.withFileInputForSelfCheck { $0.performRedo() }
        try require(composition.saveDocument(), "Composition redo Save failed")
        record("autosave leaves preedit alone; explicit Save commits once with undo")

        insert("未保存", into: draft)
        try draft.persistence.flushRecovery()
        let beforeFailedSave = draft.documentURL
        do { try draft.saveDocumentAs(to: files.appendingPathComponent("missing/sub/copy.md"), replaceExisting: false); throw CocoaError(.coderInvalidValue) }
        catch BridgeError.operation { }
        try require(draft.documentURL == beforeFailedSave && draft.persistence.bridge.state.dirty, "Failed Save As lost identity or edits")
        try require(FileManager.default.fileExists(atPath: (try draft.persistence.bridge.recoveryURL(in: locations.recovery)).path), "Failed save lost recovery")
        try require(draft.saveDocument(), "Save failed after recoverable destination error")
        record("failed Save As retains source, identity, dirty state and checkpoint")

        // Exercise Save / Cancel / Save Copy through the actual window delegate.
        let (closingWindow, closing) = try fixture("closing.md", "原文")
        insert("保存关闭", into: closing)
        closing.closeAlertDecision = { _ in .alertThirdButtonReturn }
        closingWindow.performClose(nil)
        try require(documents[closingWindow] != nil && closing.persistence.bridge.state.closeState == 0, "Cancel closed the window or left a pending close")
        closing.closeAlertDecision = { _ in .alertFirstButtonReturn }
        closingWindow.performClose(nil)
        try require(documents[closingWindow] == nil, "Save did not close the window")
        try require(try Data(contentsOf: closing.documentURL) == Data("保存关闭原文".utf8), "Save on close lost edits")
        newDocument(nil)
        let unsavedWindow = window!
        let unsaved = documents[unsavedWindow]!
        insert("首次关闭保存", into: unsaved)
        unsaved.closeAlertDecision = { _ in .alertFirstButtonReturn }
        unsaved.savePanelDecision = { _ in nil }
        unsavedWindow.performClose(nil)
        try require(documents[unsavedWindow] != nil && unsaved.persistence.bridge.state.closeState == 0, "Cancelled first Save closed the draft")
        let closeDestination = files.appendingPathComponent("saved-on-close.md")
        unsaved.savePanelDecision = { _ in closeDestination }
        unsavedWindow.performClose(nil)
        try require(documents[unsavedWindow] == nil && (try Data(contentsOf: closeDestination)) == Data("首次关闭保存".utf8), "First Save on close failed")
        let (conflictWindow, conflict) = try fixture("close-conflict.md", "基线")
        insert("本地", into: conflict)
        try Data("外部内容".utf8).write(to: conflict.documentURL, options: .atomic)
        let conflictOriginal = conflict.documentURL
        let conflictCopy = files.appendingPathComponent("close-conflict-copy.md")
        conflict.closeAlertDecision = { _ in .alertFirstButtonReturn }
        conflict.savePanelDecision = { _ in conflictCopy }
        conflictWindow.performClose(nil)
        try require(documents[conflictWindow] == nil, "Save Copy did not resolve close conflict")
        try require(try Data(contentsOf: conflictOriginal) == Data("外部内容".utf8) && Data(contentsOf: conflictCopy) == Data("本地基线".utf8), "Conflict close changed external bytes or lost local source")
        record("window close Cancel, Save, first-save cancellation and conflict Save Copy")

        newDocument(nil)
        let quitA = documents[window!]!
        insert("退出甲", into: quitA)
        try quitA.persistence.flushRecovery()
        newDocument(nil)
        let quitB = documents[window!]!
        insert("退出乙", into: quitB)
        try quitB.persistence.flushRecovery()
        let recordA = try quitA.persistence.bridge.recoveryURL(in: locations.recovery)
        let recordB = try quitB.persistence.bridge.recoveryURL(in: locations.recovery)
        var decisions = 0
        for controller in documents.values {
            controller.closeAlertDecision = { _ in
                decisions += 1
                return decisions == 1 ? .alertSecondButtonReturn : .alertThirdButtonReturn
            }
        }
        try require(applicationShouldTerminate(NSApp) == .terminateCancel && decisions == 2, "Aggregate quit cancellation failed")
        try require(documents.values.allSatisfy { $0.persistence.bridge.state.closeState == 0 }, "Cancelled quit left a closed session")
        try require(FileManager.default.fileExists(atPath: recordA.path) && FileManager.default.fileExists(atPath: recordB.path), "Provisional discard deleted recovery before quit was approved")
        record("multi-window quit cancellation rolls back all close states and keeps backups")
        for controller in documents.values { controller.closeAlertDecision = { _ in .alertSecondButtonReturn } }
        for window in Array(documents.keys) { window.performClose(nil) }
        try require(documents.isEmpty && !applicationShouldTerminateAfterLastWindowClosed(NSApp), "Last-window close terminated the application")
        try require(!FileManager.default.fileExists(atPath: recordA.path) && !FileManager.default.fileExists(atPath: recordB.path), "Approved discard left recoverable drafts")
        newDocument(nil)
        try require(documents.count == 1 && documents.values.first?.persistence.isPristineUntitled == true, "New failed after closing all windows")
        record("approved discard cleanup and New with no existing windows")
        try require(errors.isEmpty, "Unexpected file errors: \(errors)")
        try finishResult(["recent_target": conflictCopy.path])
    }
}
