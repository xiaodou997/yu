import AppKit
import Darwin
import Foundation
import QuartzCore
import os
import UniformTypeIdentifiers
import YuStorageFFI

// Metal surface 的 AppKit 宿主与帧提交调度。
//
// 「这一帧和屏幕上那一帧是不是同一帧」由 Rust 判断（`frameIsCurrent`），
// 平台只负责把 AppKit 才知道的几何递过去。metrics 计算与资源刷新判断仍在
// 本文件，属于 S1 帧调度迁移的后续步骤，见
// docs/architecture/overview-v2.md 第 8 节。

/// A real product-window surface host. Rust still owns the native layer and
/// GPU resources; this view only reports AppKit window/geometry lifecycle and
/// never becomes the canonical document model.
final class MacosSurfaceHostView: NSView {
    var onWindowStateChange: ((Bool) -> Void)?
    var onGeometryChange: (() -> Void)?
    private(set) var nativeContentVisible = false

    /// The Rust surface is a visual projection only. Keep AppKit input,
    /// selection and scrolling on the NSTextInputClient underneath it.
    func setNativeContentVisible(_ visible: Bool) {
        nativeContentVisible = visible
        // Synchronize initial visibility, but avoid dirtying the AppKit layer
        // tree again for every unchanged Metal frame.
        if isHidden != !visible { isHidden = !visible }
    }

    /// Let hit testing continue to the native input view below the surface.
    override func hitTest(_ point: NSPoint) -> NSView? {
        nil
    }

    override func viewWillMove(toWindow newWindow: NSWindow?) {
        if newWindow == nil {
            setNativeContentVisible(false)
            onWindowStateChange?(false)
        }
        super.viewWillMove(toWindow: newWindow)
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window == nil {
            setNativeContentVisible(false)
        }
        onWindowStateChange?(window != nil)
        onGeometryChange?()
    }

    override func layout() {
        super.layout()
        onGeometryChange?()
    }

    /// 切换深浅色。
    ///
    /// **文档区不会自己跟着变**：侧栏那几个面板用的是 AppKit 的语义色，外观
    /// 一换就自动重绘；文档区是 Rust 画的一帧，而换外观**既不推进 Revision
    /// 也不改几何**——不主动重提交的话，`frameIsCurrent` 会判它与屏幕上那一帧
    /// 等价而整段跳过。表现是「面板变深了，正文还是白的」，一直到你碰一下
    /// 文档为止。
    ///
    /// 走 `onGeometryChange` 这条既有的路而不是另开一条：它的语义本来就是
    /// 「平台这一侧有什么变了，重新问一次 Rust」，而外观正是那样一件事。
    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        onGeometryChange?()
    }
}
struct TableResizePointerSession: Equatable {
    let revision: UInt64
    let kind: UInt8
}

struct ScrollSchedulerMetrics: Equatable {
    fileprivate(set) var requested: UInt64 = 0
    fileprivate(set) var coalesced: UInt64 = 0
    fileprivate(set) var followUps: UInt64 = 0
    fileprivate(set) var overBudget: UInt64 = 0
    fileprivate(set) var busy: UInt64 = 0
}
/// Keeps the native pointer route explicit and headless-testable. Rust owns
/// the geometry preview; this state only answers whether subsequent mouse
/// events belong to the active divider gesture and when a revision invalidates
/// that route.
struct TableResizePointerState {
    private(set) var session: TableResizePointerSession?

    var isActive: Bool { session != nil }

    @discardableResult
    mutating func begin(revision: UInt64, kind: UInt8) -> Bool {
        guard kind == YU_STORAGE_TABLE_RESIZE_COLUMN
            || kind == YU_STORAGE_TABLE_RESIZE_ROW else {
            return false
        }
        session = TableResizePointerSession(revision: revision, kind: kind)
        return true
    }

    func acceptsUpdate(revision: UInt64) -> Bool {
        session?.revision == revision
    }

    @discardableResult
    mutating func finish(revision: UInt64) -> Bool {
        guard acceptsUpdate(revision: revision) else { return false }
        session = nil
        return true
    }

    @discardableResult
    mutating func cancel(revision: UInt64) -> Bool {
        guard acceptsUpdate(revision: revision) else { return false }
        session = nil
        return true
    }

    mutating func reset() {
        session = nil
    }
}
/// Coordinates a persistent Rust surface with one product `NSView`.
///
/// The coordinator is deliberately an AppKit lifecycle adapter, not a second
/// editor or renderer. It converts window/layout/scroll/revision changes into
/// the already validated synchronous FFI submit protocol and detaches before
/// the view leaves its window.
final class MacosSurfaceHostCoordinator {
    private static let timingEnabled = ProcessInfo.processInfo.environment["YU_RENDER_TIMING"] != nil
    private(set) var caretRevealGeneration: UInt64 = 0
    private var pendingBoundsEventTime: CFTimeInterval?

    private func recordMetric(_ event: String, fields: String = "") {
        guard Self.timingEnabled else { return }
        let identity = surfaceView?.layer.map {
            "0x" + String(UInt(bitPattern: Unmanaged.passUnretained($0).toOpaque()), radix: 16)
        } ?? "unknown"
        fputs("yu-render-metric event=\(event) surface=\(identity) time_s=\(CACurrentMediaTime()) \(fields)\n", stdout)
        fflush(stdout)
    }

    func noteBoundsEvent() {
        if !isApplyingCaretReveal && !isApplyingContentExtent {
            caretRevealGeneration &+= 1
            pendingCaretReveal = nil
        }
        guard Self.timingEnabled else { return }
        pendingBoundsEventTime = pendingBoundsEventTime ?? CACurrentMediaTime()
    }

    private static let maxImageRefreshAttempts = 40
    private static let imageRefreshInitialDelayMilliseconds = 20
    private static let imageRefreshMaximumDelayMilliseconds = 250
    private static let renderLog = Logger(
        subsystem: "io.github.xiaodou.yu",
        category: "render"
    )

    /// 一帧里只有 AppKit 知道的那部分。
    ///
    /// 这里刻意不含 Revision、composition generation 或 selection：它们是 Rust
    /// 的状态，平台把它们复制过来只会多出一份可能过期的副本。
    private struct FrameGeometry: Equatable {
        let size: CGFloat
        let maxWidth: CGFloat
        let scrollY: CGFloat
        let viewportHeight: CGFloat
        let surfaceWidth: CGFloat
        let surfaceHeight: CGFloat
        let scale: CGFloat
    }

    private let bridge: StorageBridge
    private weak var surfaceView: MacosSurfaceHostView?
    private weak var scrollView: NSScrollView?
    private var fontSize: CGFloat
    private var horizontalContentInset: CGFloat = 0
    private struct ReadingAnchor {
        let revision: UInt64
        let source: UInt64
        let affinity: UInt8
        let offset: CGFloat
        let scrollY: CGFloat
    }
    private var readingAnchor: ReadingAnchor?
    private var pendingRestoredPosition: (source: UInt64, offset: CGFloat, affinity: UInt8)?

    func encodeReadingPosition(to coder: NSCoder) {
        guard let geometry = currentFrameGeometry else { return }
        retainReadingAnchor(geometry)
        coder.encode(Int64(readingAnchor?.source ?? 0), forKey: "Yu.readingSource")
        coder.encode(Double(readingAnchor?.offset ?? 0), forKey: "Yu.readingOffset")
        coder.encode(Int(readingAnchor?.affinity ?? 0), forKey: "Yu.readingAffinity")
    }

    func retainPositionForPresentationChange() {
        if let geometry = currentFrameGeometry { retainReadingAnchor(geometry) }
    }

    func restoreReadingPosition(from coder: NSCoder) {
        guard coder.containsValue(forKey: "Yu.readingSource") else { return }
        let source = coder.decodeInt64(forKey: "Yu.readingSource")
        let offset = coder.decodeDouble(forKey: "Yu.readingOffset")
        guard source >= 0, source <= (bridge.source as NSString).length, offset.isFinite else { return }
        let affinity = coder.containsValue(forKey: "Yu.readingAffinity") ? coder.decodeInteger(forKey: "Yu.readingAffinity") : 0
        guard (0...1).contains(affinity) else { return }
        pendingRestoredPosition = (UInt64(source), CGFloat(offset), UInt8(affinity))
        scheduleSubmit()
    }

    func releaseRebuildableCaches() {
        displayLinkPacer.stop()
        try? bridge.trimRenderCaches()
        lastSnapshot = nil
        // Keep the current contents visible until activity requests a new frame.
        // The next frame must rebuild from the same source and geometry.
    }

    private func retainReadingAnchor(_ geometry: FrameGeometry) {
        // Rewrapping changes the first character of the top visual line. Keep
        // the original source anchor throughout resize/progressive updates;
        // recapture only after an actual camera move or source revision change.
        if let anchor = readingAnchor, anchor.revision == bridge.revision,
           !isLiveScrolling, pendingCaretReveal == nil,
           abs(anchor.scrollY - geometry.scrollY) < 0.5 { return }

        guard !isLiveScrolling, pendingCaretReveal == nil, geometry.scrollY > 0,
              let hit = try? bridge.projectionHitTest(revision: bridge.revision,
                point: CGPoint(x: 0, y: max(geometry.scrollY - CGFloat(NativeTheme.spec(resolved: currentAppearance).top), 0)),
                size: Float(geometry.size), maxWidth: Float(geometry.maxWidth)) else {
            readingAnchor = nil
            return
        }
        readingAnchor = ReadingAnchor(revision: bridge.revision, source: hit.sourceUTF16,
            affinity: hit.affinity, offset: geometry.scrollY - CGFloat(NativeTheme.spec(resolved: currentAppearance).top) - hit.point.y,
            scrollY: geometry.scrollY)
    }

    private func restoreReadingAnchor(_ geometry: FrameGeometry) -> Bool {
        guard let anchor = readingAnchor, anchor.revision == bridge.revision,
              !isLiveScrolling, pendingCaretReveal == nil,
              abs(anchor.scrollY - geometry.scrollY) < 0.5,
              let scrollView,
              let caret = try? bridge.sourceCaret(revision: anchor.revision, sourceUTF16: anchor.source,
                affinity: anchor.affinity, size: Float(geometry.size), maxWidth: Float(geometry.maxWidth)) else { return false }
        let maximum = max((scrollView.documentView?.bounds.height ?? 0) - geometry.viewportHeight, 0)
        let target = min(max(caret.point.y + anchor.offset + CGFloat(NativeTheme.spec(resolved: currentAppearance).top), 0), maximum)
        guard abs(target - geometry.scrollY) > 0.5 else { return false }
        recordMetric("anchor_restore", fields: "source=\(anchor.source) caret=\(caret.point.y) offset=\(anchor.offset) scroll=\(geometry.scrollY) target=\(target)")
        scrollView.contentView.setBoundsOrigin(CGPoint(x: scrollView.contentView.bounds.origin.x, y: target))
        scrollView.reflectScrolledClipView(scrollView.contentView)
        readingAnchor = ReadingAnchor(revision: anchor.revision, source: anchor.source,
            affinity: anchor.affinity, offset: anchor.offset, scrollY: target)
        return true
    }

    private(set) var lastSnapshot: NativeMacosRenderHostSurfaceSnapshot?
    private var frameWakeGate = FrameWakeGate()
    private var pendingSubmitIntent = FrameSubmitIntent()
    private var presentationRetry: DispatchWorkItem?
    private var layoutRefinement: DispatchWorkItem?
    private var frameWorkReadyObserver: NSObjectProtocol?
    private var calendarObservers: [(NotificationCenter, NSObjectProtocol)] = []
    private let calendarWakeup = RenderCalendarWakeup()
    private var calendarSuspended = false
    private var occlusionObserver: NSObjectProtocol?
    private var resourceCompletionObserver: NSObjectProtocol?
    private var scheduleToken: UInt64 = 0
    /// Latest content request; distinct from the pending wake-up ticket so
    /// continuous input cannot postpone an already scheduled frame.
    private var submitRequestGeneration: UInt64 = 0
    private var imageRefreshTask: DispatchWorkItem?
    private var imageRefreshAttempts = 0
    private var imageRefreshNeeded = false
    private var tableResizePointerState = TableResizePointerState()
    private var liveScrollDepth = 0
    private var pendingSubmitWorkItem: DispatchWorkItem?
    /// Main-thread reentrancy guard. FFI submission is currently synchronous;
    /// keeping this explicit prevents a future display-linked callback from
    /// accidentally starting a second submission before the first completes.
    private var submitInFlight = false
    private var displayLinkWakeRequested = false
    private lazy var displayLinkPacer = DisplayLinkPacer { [weak self] in
        guard let self, self.isLiveScrolling, self.displayLinkWakeRequested else { return }
        self.displayLinkWakeRequested = false
        self.enqueueSubmit(immediate: true, force: false, resetRefreshBudget: false)
    }
    private(set) var scrollMetrics = ScrollSchedulerMetrics()
    private var liveSubmitDurationsMilliseconds: [Double] = []
    private var appliedContentHeight: CGFloat?
    private var isApplyingContentExtent = false
    private var pendingTypewriterReveal = false
    private var pendingCaretReveal: NativeSelectionEndpoints?
    private var isApplyingCaretReveal = false
    private(set) var lastSubmitDurationMilliseconds: Double = 0.0

    var onError: ((Error) -> Void)?
    var onPresentationStorageError: ((Error) -> Void)?
    var onSurfaceStateChange: (() -> Void)?
    private(set) var isAttached = false

    init(bridge: StorageBridge, fontSize: CGFloat = 16.0) {
        self.bridge = bridge
        self.fontSize = max(fontSize, 1.0)
        let calendarEvents: [(NotificationCenter, Notification.Name)] = [
            (.default, .NSCalendarDayChanged),
            (.default, .NSSystemTimeZoneDidChange),
            (.default, .NSSystemClockDidChange),
            (.default, NSApplication.didBecomeActiveNotification),
            (.default, NSApplication.didResignActiveNotification),
            (NSWorkspace.shared.notificationCenter, NSWorkspace.didWakeNotification),
            (NSWorkspace.shared.notificationCenter, NSWorkspace.willSleepNotification),
            (NSWorkspace.shared.notificationCenter, NSWorkspace.sessionDidBecomeActiveNotification),
        ]
        for (center, name) in calendarEvents {
            let observer = center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                guard let self else { return }
                if name == NSWorkspace.willSleepNotification { self.calendarSuspended = true }
                if name == NSWorkspace.didWakeNotification || name == NSWorkspace.sessionDidBecomeActiveNotification { self.calendarSuspended = false }
                self.calendarEnvironmentDidChange()
            }
            calendarObservers.append((center, observer))
        }
        occlusionObserver = NotificationCenter.default.addObserver(
            forName: NSWindow.didChangeOcclusionStateNotification, object: nil, queue: .main
        ) { [weak self] notification in
            guard let self, let window = notification.object as? NSWindow,
                  window === self.surfaceView?.window else { return }
            self.calendarWakeup.cancel()
            self.layoutRefinement?.cancel()
            self.layoutRefinement = nil
            if self.isAttached, window.occlusionState.contains(.visible) {
                self.bridge.invalidateRenderCalendar()
                self.enqueueSubmit(immediate: false, force: true, resetRefreshBudget: true)
            }
        }
        frameWorkReadyObserver = NotificationCenter.default.addObserver(
            forName: Notification.Name("YuRenderWorkReady"), object: nil, queue: .main
        ) { [weak self] _ in
            guard let self, self.isAttached, self.presentationRetry != nil else { return }
            self.presentationRetry?.cancel()
            self.presentationRetry = nil
            self.recordMetric("work_ready_notification")
            self.enqueueSubmit(immediate: !self.isLiveScrolling, force: false, resetRefreshBudget: false)
        }
        resourceCompletionObserver = NotificationCenter.default.addObserver(
            forName: Notification.Name("YuRenderResourceCompleted"), object: nil, queue: .main
        ) { [weak self] _ in
            guard let self, self.isAttached, self.imageRefreshNeeded else { return }
            self.recordMetric("resource_notification")
            self.cancelImageResourceRefresh()
            self.enqueueSubmit(immediate: false, force: true, resetRefreshBudget: true)
        }
    }

    deinit {
        for (center, observer) in calendarObservers { center.removeObserver(observer) }
        if let occlusionObserver {
            NotificationCenter.default.removeObserver(occlusionObserver)
        }
        if let frameWorkReadyObserver {
            NotificationCenter.default.removeObserver(frameWorkReadyObserver)
        }
        if let resourceCompletionObserver {
            NotificationCenter.default.removeObserver(resourceCompletionObserver)
        }
    }

    private func calendarEnvironmentDidChange() {
        // Invalidate even while hidden; the next visible frame must resample.
        bridge.invalidateRenderCalendar()
        calendarWakeup.cancel()
        guard isAttached, !calendarSuspended, NSApplication.shared.isActive,
              surfaceView?.window?.occlusionState.contains(.visible) == true else { return }
        enqueueSubmit(immediate: false, force: true, resetRefreshBudget: true)
    }

    private func scheduleCalendarBoundaryRefresh() {
        guard isAttached, !calendarSuspended, NSApplication.shared.isActive,
              surfaceView?.window?.occlusionState.contains(.visible) == true,
              let boundary = bridge.nextRenderCalendarBoundary else {
            calendarWakeup.cancel()
            return
        }
        calendarWakeup.schedule(boundary: boundary) { [weak self] in
            self?.calendarEnvironmentDidChange()
        }
    }

    func bind(
        surfaceView: MacosSurfaceHostView,
        scrollView: NSScrollView,
        fontSize: CGFloat
    ) {
        if isAttached {
            detach()
        }
        cancelImageResourceRefresh()
        scheduleToken &+= 1
        submitRequestGeneration &+= 1
        self.surfaceView = surfaceView
        frameWakeGate.invalidate()
        pendingSubmitIntent = FrameSubmitIntent()
        self.scrollView = scrollView
        self.fontSize = max(fontSize, 1.0)
        horizontalContentInset = 0
        lastSnapshot = nil
        imageRefreshNeeded = false
        tableResizePointerState.reset()
        liveScrollDepth = 0
        pendingSubmitWorkItem?.cancel()
        pendingSubmitWorkItem = nil
        submitInFlight = false
        appliedContentHeight = nil
        isApplyingContentExtent = false
        isAttached = false
        surfaceView.setNativeContentVisible(false)
    }

    var isLiveScrolling: Bool { liveScrollDepth > 0 }

    /// AppKit emits several bounds changes for one trackpad gesture.  Keep the
    /// state explicit so the scheduler can use a frame-paced, latest-only path
    /// during the gesture without changing caret or edit submission semantics.
    func beginLiveScroll() {
        caretRevealGeneration &+= 1
        readingAnchor = nil
        pendingCaretReveal = nil
        if liveScrollDepth == 0 {
            recordMetric("live_begin", fields: "max_fps=\(surfaceView?.window?.screen?.maximumFramesPerSecond ?? 0) scale=\(surfaceView?.window?.backingScaleFactor ?? 0) width=\(surfaceView?.window?.frame.width ?? 0) height=\(surfaceView?.window?.frame.height ?? 0)")
            scrollMetrics = ScrollSchedulerMetrics()
            liveSubmitDurationsMilliseconds.removeAll(keepingCapacity: true)
            if let surfaceView { displayLinkPacer.start(view: surfaceView) }
        }
        liveScrollDepth += 1
    }

    func endLiveScroll() {
        liveScrollDepth = max(liveScrollDepth - 1, 0)
        guard !isLiveScrolling else { return }
        recordMetric("live_end")
        displayLinkPacer.stop()
        displayLinkWakeRequested = false
        let metrics = scrollMetrics
        Self.renderLog.debug("live scroll settled requests=\(metrics.requested, privacy: .public) coalesced=\(metrics.coalesced, privacy: .public)")
        Self.renderLog.debug("live scroll followUps=\(metrics.followUps, privacy: .public) overBudget=\(metrics.overBudget, privacy: .public)")
        Self.renderLog.debug("live scroll renderBusy=\(metrics.busy, privacy: .public)")
        if !liveSubmitDurationsMilliseconds.isEmpty {
            let sorted = liveSubmitDurationsMilliseconds.sorted()
            let percentile: (Double) -> Double = { fraction in
                let index = min(
                    sorted.count - 1,
                    Int((Double(sorted.count - 1) * fraction).rounded())
                )
                return sorted[index]
            }
            Self.renderLog.info(
                "live scroll submit-attempt timing samples=\(sorted.count, privacy: .public) p50=\(percentile(0.50), privacy: .public)ms p95=\(percentile(0.95), privacy: .public)ms p99=\(percentile(0.99), privacy: .public)ms"
            )
        }
        scheduleSubmit(immediate: true)
    }

    func setFontSize(_ fontSize: CGFloat) {
        let next = max(fontSize, 1.0)
        guard abs(self.fontSize - next) > 0.001 else { return }
        self.fontSize = next
        scheduleSubmit()
    }

    /// Retain the text inset, not a width captured during an AppKit layout
    /// callback. Every query derives wrapping width from the current surface,
    /// even while NSTextView is still resizing to match its clip view.
    func setHorizontalContentInset(_ inset: CGFloat) {
        let next = max(inset, 0.0)
        if abs(horizontalContentInset - next) <= 0.5 {
            return
        }
        horizontalContentInset = next
        scheduleSubmit()
    }

    /// 屏幕上那一帧是否就是当前状态该有的那一帧。
    ///
    /// 判断整个交给 Rust：编辑状态、composition、selection 与表格 resize 覆盖
    /// 都在那边，平台只递上自己知道的几何。可见的旧帧不算数——编辑、滚动、
    /// 缩放或改变 backing scale 之后，替换帧真正到达 surface 之前都必须判为
    /// 「不是当前帧」。
    func hasCurrentFrame(requirePresented: Bool = false) -> Bool {
        guard isAttached, let geometry = currentFrameGeometry else {
            return false
        }
        return frameIsCurrent(geometry, requirePresented: requirePresented)
    }

    func currentPresentationTime() -> Double? {
        guard isAttached, let geometry = currentFrameGeometry else { return nil }
        return bridge.framePresentationTime(size: Float(geometry.size), maxWidth: Float(geometry.maxWidth),
            scrollY: Float(geometry.scrollY), viewportHeight: Float(geometry.viewportHeight),
            surfaceWidth: Double(geometry.surfaceWidth), surfaceHeight: Double(geometry.surfaceHeight),
            scale: Double(geometry.scale), appearance: currentAppearance)
    }

    /// 把平台几何递给 Rust 的适配器。`FrameGeometry` 是本协调器的内部形状，
    /// 不应该出现在 bridge 的签名里。
    private func frameIsCurrent(_ geometry: FrameGeometry, requirePresented: Bool = false) -> Bool {
        bridge.frameIsCurrent(
            size: Float(geometry.size),
            maxWidth: Float(geometry.maxWidth),
            scrollY: Float(geometry.scrollY),
            viewportHeight: Float(geometry.viewportHeight),
            surfaceWidth: Double(geometry.surfaceWidth),
            surfaceHeight: Double(geometry.surfaceHeight),
            scale: Double(geometry.scale),
            appearance: currentAppearance,
            requirePresented: requirePresented
        )
    }

    /// 当前系统外观，送给 Rust 的那一个字节。
    ///
    /// **只有平台知道这件事**，而选出哪一套颜色是 `yu-workspace` 的事：进去的
    /// 是一个事实，出来的是一整套颜色。平台这一侧不许出现任何颜色字面量，
    /// 否则第二端要把同一套配色再挑一遍，而两端挑出来的一定会漂开。
    ///
    /// 取 surface view 的 `effectiveAppearance` 而不是 `NSApp` 的：窗口可以
    /// 单独指定外观（`NSWindow.appearance`），按 app 取会在那种窗口上画错。
    private var currentAppearance: UInt8 {
        guard let surfaceView else { return NativeTheme.resolved(dark: false) }
        let match = surfaceView.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua])
        return NativeTheme.resolved(dark: match == .darkAqua)
    }

    private var currentFrameGeometry: FrameGeometry? {
        guard let surfaceView,
              let scrollView,
              let window = surfaceView.window,
              surfaceView.bounds.width > 0.0,
              surfaceView.bounds.height > 0.0 else {
            return nil
        }
        let viewportBounds = scrollView.contentView.bounds
        return FrameGeometry(
            size: max(fontSize, 1.0),
            maxWidth: layoutWidth(for: surfaceView),
            scrollY: max(viewportBounds.origin.y, 0.0),
            viewportHeight: max(viewportBounds.height, 1.0),
            surfaceWidth: max(surfaceView.bounds.width, 1.0),
            surfaceHeight: max(surfaceView.bounds.height, 1.0),
            scale: max(window.backingScaleFactor, 1.0)
        )
    }

    private func layoutWidth(for surfaceView: MacosSurfaceHostView) -> CGFloat {
        max(surfaceView.bounds.width - horizontalContentInset, 1.0)
    }

    /// Match the coalescing cadence to the active display. A fixed 16ms delay
    /// artificially caps ProMotion windows at 60Hz; the bounds callback still
    /// remains latest-only, so this only changes when we sample the latest
    /// presentation state.
    private var liveScrollFrameInterval: DispatchTimeInterval {
        let frames = max(surfaceView?.window?.screen?.maximumFramesPerSecond ?? 60, 1)
        let seconds = 1.0 / Double(frames)
        let nanoseconds = Int((seconds * 1_000_000_000.0).rounded())
        return .nanoseconds(max(1, nanoseconds))
    }

    func scheduleSubmit(immediate: Bool = false) {
        enqueueSubmit(immediate: immediate, force: false, resetRefreshBudget: true)
    }

    private func enqueueSubmit(immediate: Bool, force: Bool, resetRefreshBudget: Bool) {
        if resetRefreshBudget {
            imageRefreshAttempts = 0
            layoutRefinement?.cancel()
            layoutRefinement = nil
        }
        pendingSubmitIntent.merge(force: force)
        scrollMetrics.requested &+= 1
        if pendingSubmitWorkItem != nil {
            scrollMetrics.coalesced &+= 1
        }
        submitRequestGeneration &+= 1
        // While submitting, only record dirty state. Completion schedules a
        // single follow-up; never spin a nested main-loop wake-up here.
        guard !submitInFlight else { return }
        guard let ticket = frameWakeGate.request(immediate: immediate) else { return }
        pendingSubmitWorkItem?.cancel()
        let token = scheduleToken
        let delay: DispatchTimeInterval
        if isLiveScrolling {
            displayLinkWakeRequested = true
        }
        if immediate || !isLiveScrolling {
            delay = .milliseconds(0)
        } else {
            // Keep the main thread from attempting a synchronous Rust/Metal
            // submission for every fractional trackpad bounds update.  The
            // latest bounds are read when this work item runs.
            delay = liveScrollFrameInterval
        }
        let workItem = DispatchWorkItem { [weak self] in
            guard let self,
                  self.scheduleToken == token,
                  self.frameWakeGate.consume(ticket) else { return }
            let requestGeneration = self.submitRequestGeneration
            self.pendingSubmitWorkItem = nil
            guard !self.submitInFlight else {
                // The active submission will schedule a latest-only follow-up
                // once it has returned to the main run loop.
                self.enqueueSubmit(immediate: false, force: false, resetRefreshBudget: false)
                return
            }
            self.submitInFlight = true
            defer {
                self.submitInFlight = false
                if self.scheduleToken == token,
                   self.submitRequestGeneration != requestGeneration,
                   self.isAttached {
                    self.scrollMetrics.followUps &+= 1
                    self.enqueueSubmit(immediate: !self.isLiveScrolling, force: false, resetRefreshBudget: false)
                }
            }
            do {
                _ = try self.submitNow(force: self.pendingSubmitIntent.takeForce())
            } catch {
                self.clearTableResizeState()
                self.imageRefreshNeeded = false
                self.cancelImageResourceRefresh()
                self.surfaceView?.setNativeContentVisible(false)
                self.onSurfaceStateChange?()
                self.onError?(error)
            }
        }
        pendingSubmitWorkItem = workItem
        if !(isLiveScrolling && !immediate && displayLinkPacer.isRunning) {
            DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: workItem)
        }
    }

    private func cancelImageResourceRefresh() {
        imageRefreshTask?.cancel()
        imageRefreshTask = nil
        imageRefreshAttempts = 0
    }

    /// Distant measurements must not supersede the first visible publication
    /// before it reaches the display. Retain only one revision/serial-bound
    /// follow-up, and do no offscreen polling after occlusion or minimization.
    private func scheduleLayoutRefinement(serial: UInt64, delay: DispatchTimeInterval = .milliseconds(32)) {
        layoutRefinement?.cancel()
        let token = scheduleToken
        let work = DispatchWorkItem { [weak self] in
            guard let self, self.scheduleToken == token else { return }
            self.layoutRefinement = nil
            guard self.isAttached, !self.isLiveScrolling,
                  self.lastSnapshot?.frameSerial == serial,
                  self.lastSnapshot?.layoutPending == true,
                  self.surfaceView?.window?.occlusionState.contains(.visible) == true else { return }
            guard self.hasCurrentFrame(requirePresented: true) else {
                self.scheduleLayoutRefinement(serial: serial, delay: self.liveScrollFrameInterval)
                return
            }
            self.enqueueSubmit(immediate: false, force: true, resetRefreshBudget: false)
        }
        layoutRefinement = work
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: work)
    }

    /// Bounded backoff for failed resources. In-flight workers wake this host
    /// through completion notifications and do not need a polling timer.
    private func scheduleImageResourceRefresh() {
        guard imageRefreshNeeded,
              lastSnapshot?.resourceRetryPending == true,
              isAttached,
              imageRefreshTask == nil,
              imageRefreshAttempts < Self.maxImageRefreshAttempts else {
            return
        }
        let token = scheduleToken
        imageRefreshAttempts += 1
        let exponent = min(imageRefreshAttempts - 1, 4)
        let delayMilliseconds = min(
            Self.imageRefreshInitialDelayMilliseconds * (1 << exponent),
            Self.imageRefreshMaximumDelayMilliseconds
        )
        let task = DispatchWorkItem { [weak self] in
            guard let self,
                  self.scheduleToken == token,
                  self.isAttached else {
                return
            }
            self.imageRefreshTask = nil
            self.recordMetric("resource_retry")
            // Join the existing wake-up rather than bypassing the scheduler.
            // Polling must not replenish its own bounded retry budget.
            self.enqueueSubmit(immediate: false, force: true, resetRefreshBudget: false)
        }
        imageRefreshTask = task
        DispatchQueue.main.asyncAfter(
            deadline: .now() + .milliseconds(delayMilliseconds),
            execute: task
        )
    }

    /// Resolves the current document-space point against submitted table
    /// geometry. Hover is intentionally
    /// read-only: it never opens a Rust resize session and silently falls
    /// back to the normal arrow when metrics or the Revision are stale.
    func tableResizeHover(at point: NSPoint) -> Bool {
        let start = DispatchTime.now().uptimeNanoseconds
        defer {
            recordMetric("hover_query", fields: "duration_ms=\(Double(DispatchTime.now().uptimeNanoseconds - start) / 1_000_000)")
        }
        if surfaceView?.window != nil && !isAttached { return false }
        if let session = tableResizePointerState.session,
           session.revision == bridge.revision {
            return session.kind == YU_STORAGE_TABLE_RESIZE_COLUMN
        }
        guard !bridge.composition.active,
              point.x.isFinite,
              point.y.isFinite,
              let geometry = visualDecorationGeometry() else {
            return false
        }
        let revision = bridge.revision
        let tolerance = Float(max(CGFloat(6.0), fontSize * 0.4))
        do {
            return try bridge.tableResizeHover(
                revision: revision,
                size: geometry.size,
                maxWidth: geometry.maxWidth,
                scrollY: geometry.scrollY,
                viewportHeight: geometry.viewportHeight,
                point: CGPoint(x: point.x, y: point.y),
                tolerance: tolerance
            )
        } catch {
            return false
        }
    }

    /// Resolves a primary click against the exact Rust scene publication that
    /// is currently visible. The coordinator returns metadata only; canonical
    /// mutation stays in `DocumentTextView`'s existing command path.
    func taskCheckboxHit(at point: NSPoint) -> NativeTaskCheckboxHit? {
        let revision = bridge.revision
        guard !bridge.composition.active,
              point.x.isFinite,
              point.y.isFinite,
              hasCurrentFrame() else {
            return nil
        }
        do {
            let hit = try bridge.taskCheckboxHitTest(
                revision: revision,
                point: CGPoint(x: point.x, y: point.y)
            )
            guard hit.revision == revision,
                  hit.bounds.width > 0.0,
                  hit.bounds.height > 0.0 else {
                return nil
            }
            return hit
        } catch BridgeError.operation(let status)
            where status == StorageStatus.invalidSelection
                || status == StorageStatus.staleRevision {
            return nil
        } catch {
            // A missing/stale retained publication is an enhancement miss.
            // Preserve AppKit's ordinary source selection without surfacing a
            // modal error for a pointer query.
            return nil
        }
    }

    /// Returns read-only divider descriptors from the submitted frame's
    /// document-space table geometry. Callers project
    /// these into ephemeral native Accessibility elements; the descriptors
    /// never retain a Rust layout or open a resize gesture.
    func tableResizeAccessibilityDividers() -> [NativeTableResizeAccessibilityDivider] {
        // Initial window layout must not invoke the headless shaping fallback
        // before its production render host has been attached.
        // The controller is queried before it even has a window. No surface
        // means no published geometry, regardless of window attachment.
        guard isAttached else { return [] }
        guard !bridge.composition.active,
              let geometry = visualDecorationGeometry() else {
            return []
        }
        do {
            return try bridge.tableResizeAccessibilityDividers(
                revision: bridge.revision,
                size: geometry.size,
                maxWidth: geometry.maxWidth,
                scrollY: geometry.scrollY,
                viewportHeight: geometry.viewportHeight
            )
        } catch {
            return []
        }
    }

    /// Converts one document-space divider descriptor into a screen-space AX
    /// frame. The conversion is intentionally performed at query time so a
    /// scroll, window move, or surface detach cannot leave an element holding
    /// stale AppKit coordinates.
    func tableResizeAccessibilityFrame(
        for descriptor: NativeTableResizeAccessibilityDivider
    ) -> NSRect {
        guard descriptor.revision == bridge.revision,
              !bridge.composition.active,
              let surfaceView,
              let window = surfaceView.window,
              let geometry = visualDecorationGeometry() else {
            return .zero
        }
        let local = descriptor.rect.offsetBy(
            dx: 0.0,
            dy: -CGFloat(geometry.scrollY)
        )
        return window.convertToScreen(surfaceView.convert(local, to: nil))
    }

    /// Performs one VoiceOver increment/decrement as a Rust-owned transient
    /// resize. The effective divider descriptor is queried again after the
    /// action, so repeated actions accumulate through the session override
    /// without changing Markdown source or creating an editor transaction.
    @discardableResult
    func adjustTableResizeAccessibility(
        _ descriptor: NativeTableResizeAccessibilityDivider,
        direction: Int
    ) -> Bool {
        guard descriptor.revision == bridge.revision,
              descriptor.kind == UInt8(YU_STORAGE_TABLE_RESIZE_COLUMN),
              descriptor.columnCount >= 2,
              descriptor.index < descriptor.columnCount - 1,
              (direction == 1 || direction == -1),
              !bridge.composition.active,
              !tableResizePointerState.isActive,
              visualDecorationGeometry() != nil else {
            return false
        }
        // 步长由 Rust 随描述符一起给出——它是策略，不是平台信息。
        let step = descriptor.adjustStep
        let dividerPoint = NSPoint(
            x: descriptor.rect.midX,
            y: descriptor.rect.midY
        )
        guard beginTableResize(at: dividerPoint) else { return false }
        let updatedPoint = NSPoint(
            x: dividerPoint.x + CGFloat(direction) * step,
            y: dividerPoint.y
        )
        guard updateTableResize(at: updatedPoint),
              finishTableResize() else {
            _ = cancelTableResize()
            return false
        }
        return true
    }

    /// Attempts to start a CoreText-shaped table divider gesture at a
    /// document-space point. The hit-test is intentionally non-mutating and
    /// is followed by a matching Rust begin call so row gestures can choose
    /// their y-axis pointer coordinate before the preview is created.
    @discardableResult
    func beginTableResize(at point: NSPoint) -> Bool {
        guard !bridge.composition.active,
              point.x.isFinite,
              point.y.isFinite,
              let geometry = visualDecorationGeometry() else {
            return false
        }
        let revision = bridge.revision
        let tolerance = Float(max(CGFloat(6.0), fontSize * 0.4))
        do {
            let hit = try bridge.tableResizeAtDocumentPoint(
                revision: revision,
                action: UInt8(YU_STORAGE_TABLE_RESIZE_PROBE),
                size: geometry.size,
                maxWidth: geometry.maxWidth,
                point: CGPoint(x: point.x, y: point.y),
                tolerance: tolerance
            )
            // The retained render host currently consumes only column
            // overrides; keep row dividers on the normal selection path until
            // variable-row layout is published end-to-end.
            guard hit.kind == UInt8(YU_STORAGE_TABLE_RESIZE_COLUMN) else {
                return false
            }
            let pointerPosition = Float(point.x)
            let begun = try bridge.tableResizeAtDocumentPoint(
                revision: revision,
                action: UInt8(YU_STORAGE_TABLE_RESIZE_BEGIN),
                size: geometry.size,
                maxWidth: geometry.maxWidth,
                point: CGPoint(x: point.x, y: point.y),
                tolerance: tolerance,
                pointerPosition: pointerPosition
            )
            guard begun.revision == revision,
                  tableResizePointerState.begin(
                      revision: revision,
                      kind: begun.kind
                  ) else {
                return false
            }
            scheduleSubmit()
            return true
        } catch BridgeError.operation(let status)
            where status == StorageStatus.invalidSelection {
            // A click between table dividers belongs to normal selection.
            return false
        } catch {
            // A stale/temporarily unavailable shaped layout must never make
            // the source TextKit editor stop accepting pointer input.
            tableResizePointerState.reset()
            onError?(error)
            return false
        }
    }

    /// Forwards one drag sample to Rust and invalidates the retained surface
    /// even though the ordinary geometry submit key has not changed.
    @discardableResult
    func updateTableResize(at point: NSPoint) -> Bool {
        let revision = bridge.revision
        guard let session = tableResizePointerState.session,
              session.revision == revision,
              point.x.isFinite,
              point.y.isFinite else {
            return false
        }
        let pointerPosition = session.kind == YU_STORAGE_TABLE_RESIZE_COLUMN
            ? Float(point.x)
            : Float(point.y)
        do {
            _ = try bridge.tableResizeAction(
                revision: revision,
                action: UInt8(YU_STORAGE_TABLE_RESIZE_UPDATE),
                pointerPosition: pointerPosition
            )
            scheduleSubmit()
            return true
        } catch BridgeError.operation(let status)
            where status == StorageStatus.staleRevision
                || status == StorageStatus.tableResizeNotActive {
            tableResizePointerState.reset()
            return true
        } catch {
            tableResizePointerState.reset()
            onError?(error)
            return true
        }
    }

    /// Finishes the Rust gesture. The final preview remains attached to the
    /// current session until the next source revision or explicit reset, so
    /// the retained frame shows the committed divider immediately.
    @discardableResult
    func finishTableResize() -> Bool {
        let revision = bridge.revision
        guard tableResizePointerState.acceptsUpdate(revision: revision) else {
            return false
        }
        var finished = false
        do {
            _ = try bridge.tableResizeAction(
                revision: revision,
                action: UInt8(YU_STORAGE_TABLE_RESIZE_FINISH)
            )
            _ = tableResizePointerState.finish(revision: revision)
            scheduleSubmit()
            finished = true
            do {
                try bridge.persistTableWidths()
            } catch {
                onPresentationStorageError?(error)
            }
        } catch {
            tableResizePointerState.reset()
            onError?(error)
        }
        return finished
    }

    /// Cancels only an active pointer gesture. Document edits use
    /// `resetTableResizeAfterDocumentChange()` to also clear a finished
    /// preview that Rust intentionally keeps for the current frame.
    @discardableResult
    func cancelTableResize() -> Bool {
        let revision = bridge.revision
        guard tableResizePointerState.acceptsUpdate(revision: revision) else {
            return false
        }
        do {
            try bridge.tableResizeAction(
                revision: revision,
                action: UInt8(YU_STORAGE_TABLE_RESIZE_CANCEL)
            )
            _ = tableResizePointerState.cancel(revision: revision)
            scheduleSubmit()
        } catch {
            tableResizePointerState.reset()
            onError?(error)
        }
        return true
    }

    /// Clears both an active gesture and the finished preview when the
    /// canonical source revision changes. The FFI call is harmless when no
    /// gesture exists and is revision-bound to avoid clearing a newer edit.
    func resetTableResizeAfterDocumentChange() {
        clearTableResizeState()
    }

    var tableResizeActiveForSelfCheck: Bool {
        tableResizePointerState.isActive
    }

    /// 视觉装饰查询所用的 viewport 输入，与 Rust render host 完全同源。
    ///
    /// 这里只回答 AppKit 知道的量。行高与默认步进不在其中：它们由 Rust 在每个
    /// shaped 入口自行对齐，平台不再取回来又送回去。
    func visualDecorationGeometry() -> (
        size: Float,
        maxWidth: Float,
        scrollY: Float,
        viewportHeight: Float
    )? {
        guard let surfaceView,
              let scrollView,
              surfaceView.bounds.width > 0.0,
              surfaceView.bounds.height > 0.0 else {
            return nil
        }
        let viewportBounds = scrollView.contentView.bounds
        return (
            Float(max(fontSize, 1.0)),
            Float(layoutWidth(for: surfaceView)),
            Float(max(viewportBounds.origin.y, 0.0)),
            Float(max(viewportBounds.height, 1.0))
        )
    }

    /// Reveals the current Rust-owned caret using the same Revision-bound
    /// CoreText/shaped viewport contract as surface submission. This is a
    /// scroll-only adapter: it never asks TextKit for a caret and never lets
    /// AppKit invent document geometry. A stale or unavailable request is
    /// ignored so a transient surface race cannot interrupt editing.
    func revealCaretIfNeeded(typewriter: Bool = false, generation: UInt64? = nil) {
        guard !isLiveScrolling, generation == nil || generation == caretRevealGeneration else { return }
        let selection = bridge.selectionEndpoints
        let sameTarget = pendingCaretReveal.map {
            $0.revision == selection.revision && $0.focusUTF16 == selection.focusUTF16
                && $0.anchorUTF16 == selection.anchorUTF16 && $0.affinity == selection.affinity
        } ?? false
        pendingTypewriterReveal = NativeWritingPreferences.shared.typewriterMode
            && (typewriter || (sameTarget && pendingTypewriterReveal))
        pendingCaretReveal = selection
        _ = continueCaretReveal()
    }

    /// Deterministic event-order regression: an edit queues a main-thread
    /// reveal, then the user scrolls before that closure is delivered.
    func verifyDeferredCaretRevealForSelfCheck() {
        let queued = caretRevealGeneration
        noteBoundsEvent()
        revealCaretIfNeeded(typewriter: true, generation: queued)
        precondition(pendingCaretReveal == nil, "Deferred editing must not undo a newer user scroll")
        revealCaretIfNeeded(typewriter: true, generation: caretRevealGeneration)
        precondition(pendingCaretReveal != nil, "A current edit must retain its layout intent")
        beginLiveScroll()
        precondition(pendingCaretReveal == nil)
        revealCaretIfNeeded(typewriter: true, generation: caretRevealGeneration)
        precondition(pendingCaretReveal == nil, "Live scrolling must not acquire a caret-follow intent")
        endLiveScroll()
        let beforeDetach = caretRevealGeneration
        detach()
        revealCaretIfNeeded(typewriter: true, generation: beforeDetach)
        precondition(pendingCaretReveal == nil, "Detached views must reject queued navigation")
    }

    func refineCaretRevealIfNeeded() {
        _ = continueCaretReveal()
    }

    /// A navigation target can move while previously estimated blocks are
    /// measured. Keep that intent until an accepted frame contains the caret;
    /// ordinary user scrolling and detach cancel it immediately.
    @discardableResult
    private func continueCaretReveal() -> Bool {
        guard let pending = pendingCaretReveal else { return false }
        let current = bridge.selectionEndpoints
        guard pending.revision == current.revision,
              pending.focusUTF16 == current.focusUTF16,
              pending.anchorUTF16 == current.anchorUTF16,
              pending.affinity == current.affinity else {
            pendingCaretReveal = nil
            return false
        }
        guard let surfaceView,
              let scrollView,
              surfaceView.bounds.width > 0.0,
              surfaceView.bounds.height > 0.0 else {
            return false
        }
        let revision = bridge.revision
        let size = max(fontSize, 1.0)
        let maxWidth = layoutWidth(for: surfaceView)
        let viewportBounds = scrollView.contentView.bounds
        let viewportHeight = max(viewportBounds.height, 1.0)
        let currentScrollY = max(viewportBounds.origin.y, 0.0)
        do {
            let request = try bridge.shapedCaretScrollRequest(
                revision: revision,
                size: Float(size),
                maxWidth: Float(maxWidth),
                scrollY: Float(max(currentScrollY - CGFloat(NativeTheme.spec(resolved: currentAppearance).top), 0)),
                viewportHeight: Float(viewportHeight)
            )
            if Self.timingEnabled {
                print("yu-render-metric event=caret_reveal source=\(request.sourceUTF16) block=\(request.blockIndex) caret_y=\(request.caretPoint.y) height=\(request.caretHeight) scroll=\(request.currentScrollY) target=\(request.targetScrollY) width=\(maxWidth) extent=\(scrollView.documentView?.bounds.height ?? 0)")
            }
            guard request.revision == revision,
                  request.currentScrollY.isFinite,
                  request.targetScrollY.isFinite,
                  request.targetScrollY >= 0.0 else {
                pendingCaretReveal = nil
                return false
            }
            let centerForTyping = pendingTypewriterReveal && NativeWritingPreferences.shared.typewriterMode
            if !centerForTyping && !request.needsScroll {
                if hasCurrentFrame(), (lastSnapshot?.caretDecorationCount ?? 0) > 0 {
                    pendingCaretReveal = nil
                }
                return false
            }
            let nativeMaxScrollY = max(
                (scrollView.documentView?.bounds.height ?? 0.0) - viewportHeight,
                0.0
            )
            let top = CGFloat(NativeTheme.spec(resolved: currentAppearance).top)
            let desiredScrollY = centerForTyping
                ? NativeTypewriterGeometry.scrollTarget(caretY: request.caretPoint.y, caretHeight: request.caretHeight, top: top, viewportHeight: viewportHeight, maximumScroll: nativeMaxScrollY)
                : request.targetScrollY + top
            let targetScrollY = min(max(desiredScrollY, 0.0), nativeMaxScrollY)
            guard abs(targetScrollY - currentScrollY) > 0.5 else {
                // AppKit rounds clip origins to backing pixels. A subpixel
                // difference is settled once this frame contains the caret.
                if hasCurrentFrame(), (lastSnapshot?.caretDecorationCount ?? 0) > 0 {
                    pendingCaretReveal = nil
                }
                return false
            }
            var origin = viewportBounds.origin
            origin.y = targetScrollY
            isApplyingCaretReveal = true
            defer { isApplyingCaretReveal = false }
            scrollView.contentView.setBoundsOrigin(origin)
            scrollView.reflectScrolledClipView(scrollView.contentView)
            scheduleSubmit()
            return true
        } catch {
            // A stale query must not move the native viewport. The next
            // accepted frame retries the pending navigation intent.
        }
        return false
    }

    /// 让可滚动范围等于 Rust 这一帧渲染出来的内容高度。
    ///
    /// 滚动范围此前来自 document view 自己的 TextKit 排版——一套已经不再绘制
    /// 任何像素的布局（不变量 I5）。两套布局算出的高度并不相同：投影里标题更
    /// 大、块间有间距，因此实际内容比源码排版高得多，长文档的尾部根本滚不到，
    /// 而且没有任何报错。
    ///
    /// 用 minSize/maxSize 把高度钉死，而不是只 `setFrameSize`：`NSTextView`
    /// 在 `isVerticallyResizable` 下会按自己的排版把 frame 改回去。
    private func applyContentHeight(_ contentHeight: CGFloat) {
        guard let scrollView,
              let documentView = scrollView.documentView,
              contentHeight.isFinite,
              contentHeight > 0.0 else {
            return
        }
        // 内容比视口短时仍然占满视口，否则 clip view 会露出背景。
        let theme = NativeTheme.spec(resolved: currentAppearance)
        // Extra trailing space allows the last line to reach the writing position.
        // It is viewport padding, not source text or independent layout geometry.
        let trailingSpace = NativeWritingPreferences.shared.typewriterMode
            ? max(CGFloat(theme.bottom), scrollView.contentView.bounds.height / 2)
            : CGFloat(theme.bottom)
        let target = max(contentHeight + CGFloat(theme.top) + trailingSpace, scrollView.contentView.bounds.height)
        let extentChanged = appliedContentHeight.map {
            abs($0 - target) > 0.5
        } ?? true
        let frameChanged = abs(documentView.frame.height - target) > 0.5
        guard extentChanged || frameChanged else { return }
        guard !isApplyingContentExtent else { return }
        isApplyingContentExtent = true
        defer {
            isApplyingContentExtent = false
        }
        // 改变可滚动范围不得移动视口。AppKit 在 document view 变高时会自行调整
        // clip view 的 bounds origin——首帧就会把长文档直接滚到底部，用户打开
        // 文件看到的是最后一屏，而且没有任何报错。滚动位置是用户的状态，
        // 不是布局的副产品。
        let origin = scrollView.contentView.bounds.origin
        documentView.setFrameSize(
            NSSize(width: documentView.frame.width, height: target)
        )
        if scrollView.contentView.bounds.origin != origin {
            scrollView.contentView.setBoundsOrigin(origin)
            scrollView.reflectScrolledClipView(scrollView.contentView)
        }
        appliedContentHeight = target
    }

    /// 提交一帧。
    ///
    /// `force` 只为资源刷新轮询而存在：那条路径需要一次真实提交去收割
    /// 异步 worker 的结果，而此时几何与编辑状态都没有变化，Rust 会正确地
    /// 判定「与屏幕上的帧等价」。资源刷新判断移入 Rust 后这个参数即可删除。
    @discardableResult
    func submitNow(force: Bool = false) throws -> NativeMacosRenderHostSurfaceSnapshot? {
        defer { scheduleCalendarBoundaryRefresh() }
        let startedAt = DispatchTime.now().uptimeNanoseconds
        if let boundsTime = pendingBoundsEventTime {
            pendingBoundsEventTime = nil
            recordMetric("bounds_to_request", fields: "duration_ms=\((CACurrentMediaTime() - boundsTime) * 1000)")
        }
        defer {
            let elapsed = DispatchTime.now().uptimeNanoseconds - startedAt
            let milliseconds = Double(elapsed) / 1_000_000.0
            lastSubmitDurationMilliseconds = milliseconds
            recordMetric("submit_attempt", fields: "duration_ms=\(milliseconds)")
            if isLiveScrolling {
                if liveSubmitDurationsMilliseconds.count == 4096 {
                    liveSubmitDurationsMilliseconds.removeFirst()
                }
                liveSubmitDurationsMilliseconds.append(milliseconds)
            }
            if milliseconds > 16.7 {
                scrollMetrics.overBudget &+= 1
                Self.renderLog.warning(
                    "frame submit exceeded display budget: \(milliseconds, privacy: .public) ms liveScroll=\(self.isLiveScrolling, privacy: .public)"
                )
            }
        }
        guard let surfaceView,
              let geometry = currentFrameGeometry else {
            return nil
        }
        if let restored = pendingRestoredPosition {
            pendingRestoredPosition = nil
            readingAnchor = ReadingAnchor(revision: bridge.revision, source: restored.source,
                affinity: restored.affinity, offset: restored.offset, scrollY: geometry.scrollY)
            pendingCaretReveal = nil
        }

        // Calendar state is not part of the document revision or Swift geometry.
        // Check it before the retained-frame fast path, and invalidate that path
        // even if the subsequent native submission is busy or fails.
        if try bridge.updateRenderCalendar() { lastSnapshot = nil }
        if !force, isAttached, lastSnapshot?.layoutPending != true, frameIsCurrent(geometry) {
            // Rust surface 是唯一渲染路径：attach 之后一直可见，
            // 内容由 retained frame 决定，不由 coverage 决定（不变量 I5）。
            surfaceView.setNativeContentVisible(true)
            scheduleImageResourceRefresh()
            return lastSnapshot
        }

        let revision = bridge.revision
        let rawView = Unmanaged.passUnretained(surfaceView).toOpaque()
        let snapshot: NativeMacosRenderHostSurfaceSnapshot
        do {
            snapshot = try bridge.macosRenderHostSurfaceSubmit(
                revision: revision,
                size: Float(geometry.size),
                maxWidth: Float(geometry.maxWidth),
                scrollY: Float(geometry.scrollY),
                viewportHeight: Float(geometry.viewportHeight),
                surfaceWidth: Double(geometry.surfaceWidth),
                surfaceHeight: Double(geometry.surfaceHeight),
                scale: Double(geometry.scale),
                appearance: currentAppearance,
                view: rawView
            )
        } catch BridgeError.operation(let status) where status == YU_STORAGE_RENDER_BUSY {
            // The Rust surface exists even when its first presentation is busy.
            // Keep lifecycle ownership so closing the window still detaches it.
            isAttached = true
            scrollMetrics.busy &+= 1
            recordMetric("render_busy")
            pendingSubmitIntent.merge(force: force)
            if presentationRetry == nil {
                let token = scheduleToken
                let retry = DispatchWorkItem { [weak self] in
                    guard let self, self.scheduleToken == token else { return }
                    self.presentationRetry = nil
                    self.enqueueSubmit(immediate: false, force: false, resetRefreshBudget: false)
                }
                presentationRetry = retry
                DispatchQueue.main.asyncAfter(deadline: .now() + liveScrollFrameInterval, execute: retry)
            }
            return nil
        }
        presentationRetry?.cancel()
        presentationRetry = nil
        isAttached = true
        if lastSnapshot == nil {
            recordMetric("surface_ready", fields: "max_fps=\(surfaceView.window?.screen?.maximumFramesPerSecond ?? 0) scale=\(geometry.scale) width=\(surfaceView.window?.frame.width ?? 0) height=\(surfaceView.window?.frame.height ?? 0)")
        }
        lastSnapshot = snapshot
        layoutRefinement?.cancel()
        layoutRefinement = nil
        if snapshot.layoutPending && !isLiveScrolling {
            scheduleLayoutRefinement(serial: snapshot.frameSerial)
        }
        applyContentHeight(snapshot.contentHeight)
        if restoreReadingAnchor(geometry) {
            scheduleSubmit()
            return nil
        }
        // 「还有资源没落定吗」由 Rust 在提交这一帧时一并回答。平台此前要为此
        // 再查三次——可见 block、全部图片状态、全部内嵌资源状态——还得自己复制
        // 一份状态码语义表（不变量 I3）。
        imageRefreshNeeded = snapshot.resourceRefreshPending
        surfaceView.setNativeContentVisible(true)
        onSurfaceStateChange?()
        if imageRefreshNeeded {
            scheduleImageResourceRefresh()
        } else {
            cancelImageResourceRefresh()
        }
        if currentFrameGeometry != geometry {
            // Applying content extent may finish AppKit layout and resize the
            // clip/surface. This submission belongs to the previous geometry.
            recordMetric("geometry_changed_during_submit")
            scheduleSubmit()
            return nil
        }
        if continueCaretReveal() {
            // The submitted frame was valid for the old camera. A newly
            // refined navigation target must get its own latest presentation.
            return nil
        }
        retainReadingAnchor(geometry)
        return snapshot
    }

    /// Real-window regression for progressive completion and source anchoring.
    @MainActor
    func verifyLayoutCoordinatorForSelfCheck() async throws {
        guard let window = surfaceView?.window, let scrollView else { return }
        @MainActor func settle() async throws {
            let deadline = Date().addingTimeInterval(20)
            while Date() < deadline {
                if let frame = try submitNow(force: true), !frame.layoutPending { return }
                try await Task.sleep(nanoseconds: 8_000_000)
            }
            throw NSError(domain: "YuLayoutCoordinator", code: 1,
                userInfo: [NSLocalizedDescriptionKey: "Progressive layout did not finish"])
        }
        pendingCaretReveal = nil
        try await settle()
        let original = window.frame
        let maximum = max((scrollView.documentView?.bounds.height ?? 0) - scrollView.contentView.bounds.height, 0)
        guard maximum > 100 else { return }
        scrollView.contentView.setBoundsOrigin(CGPoint(x: 0, y: maximum * 0.5))
        scrollView.reflectScrolledClipView(scrollView.contentView)
        try await settle()
        guard let anchor = readingAnchor else {
            throw NSError(domain: "YuLayoutCoordinator", code: 2,
                userInfo: [NSLocalizedDescriptionKey: "Missing reading anchor"])
        }
        for delta in [-120.0, 80.0, 0.0] {
            var frame = original
            frame.size.width += delta
            window.setFrame(frame, display: true)
            window.contentView?.layoutSubtreeIfNeeded()
            try await settle()
            guard let geometry = currentFrameGeometry else { continue }
            let caret = try bridge.sourceCaret(revision: anchor.revision, sourceUTF16: anchor.source,
                affinity: anchor.affinity, size: Float(geometry.size), maxWidth: Float(geometry.maxWidth))
            let error = abs(caret.point.y + CGFloat(NativeTheme.spec(resolved: currentAppearance).top) + anchor.offset - geometry.scrollY)
            guard error <= 1.0 else {
                throw NSError(domain: "YuLayoutCoordinator", code: 3,
                    userInfo: [NSLocalizedDescriptionKey: "Reading anchor drifted \(error) pt"])
            }
        }
        print("Yu layout coordinator self-check: progressive=settled resizeAnchor=within-1pt widths=3")
    }

    func detach() {
        calendarWakeup.cancel()
        caretRevealGeneration &+= 1
        layoutRefinement?.cancel()
        layoutRefinement = nil
        readingAnchor = nil
        pendingCaretReveal = nil
        if isLiveScrolling { recordMetric("live_end") }
        pendingBoundsEventTime = nil
        presentationRetry?.cancel()
        presentationRetry = nil
        scheduleToken &+= 1
        submitRequestGeneration &+= 1
        frameWakeGate.invalidate()
        displayLinkPacer.stop()
        displayLinkWakeRequested = false
        pendingSubmitIntent = FrameSubmitIntent()
        pendingSubmitWorkItem?.cancel()
        pendingSubmitWorkItem = nil
        submitInFlight = false
        cancelImageResourceRefresh()
        clearTableResizeState()
        if isAttached {
            do {
                try bridge.macosRenderHostSurfaceDetach()
            } catch {
                onError?(error)
            }
        }
        isAttached = false
        lastSnapshot = nil
        imageRefreshNeeded = false
        surfaceView?.setNativeContentVisible(false)
        onSurfaceStateChange?()
    }

    private func clearTableResizeState() {
        _ = try? bridge.tableResizeAction(
            revision: bridge.revision,
            action: UInt8(YU_STORAGE_TABLE_RESIZE_CANCEL)
        )
        tableResizePointerState.reset()
    }

}
