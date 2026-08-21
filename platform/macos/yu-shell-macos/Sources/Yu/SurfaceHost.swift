import AppKit
import Darwin
import Foundation
import UniformTypeIdentifiers
import YuStorageFFI

// Metal surface 的 AppKit 宿主与帧提交调度。
//
// 「这一帧和屏幕上那一帧是不是同一帧」由 Rust 判断（`macosFrameIsCurrent`），
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
    /// selection and scrolling on the TextKit mirror underneath it.
    func setNativeContentVisible(_ visible: Bool) {
        if nativeContentVisible == visible {
            // NSView defaults to visible before the first successful submit;
            // keep the fallback state deterministic even on the first bind.
            isHidden = !visible
            return
        }
        nativeContentVisible = visible
        isHidden = !visible
    }

    /// Let hit testing continue to the source TextKit view below the overlay.
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
}
struct TableResizePointerSession: Equatable {
    let revision: UInt64
    let kind: UInt8
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
    private static let maxImageRefreshAttempts = 40
    private static let imageRefreshDelay: DispatchTimeInterval = .milliseconds(50)

    /// 一帧里只有 AppKit 知道的那部分。
    ///
    /// 这里刻意不含 Revision、composition generation 或 selection：它们是 Rust
    /// 的状态，平台把它们复制过来只会多出一份可能过期的副本。
    private struct FrameGeometry {
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
    private var contentWidth: CGFloat?
    private(set) var lastSnapshot: NativeMacosRenderHostSurfaceSnapshot?
    private var submitScheduled = false
    private var scheduleToken: UInt64 = 0
    private var imageRefreshTask: DispatchWorkItem?
    private var imageRefreshAttempts = 0
    private var imageRefreshNeeded = false
    private var tableResizePointerState = TableResizePointerState()

    var onError: ((Error) -> Void)?
    var onSurfaceStateChange: (() -> Void)?
    private(set) var isAttached = false

    init(bridge: StorageBridge, fontSize: CGFloat = 16.0) {
        self.bridge = bridge
        self.fontSize = max(fontSize, 1.0)
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
        self.surfaceView = surfaceView
        self.scrollView = scrollView
        self.fontSize = max(fontSize, 1.0)
        contentWidth = nil
        lastSnapshot = nil
        imageRefreshNeeded = false
        tableResizePointerState.reset()
        isAttached = false
        surfaceView.setNativeContentVisible(false)
    }

    func setFontSize(_ fontSize: CGFloat) {
        let next = max(fontSize, 1.0)
        guard abs(self.fontSize - next) > 0.001 else { return }
        self.fontSize = next
        scheduleSubmit()
    }

    /// Publishes the text content width used by the source TextKit mirror.
    /// The transparent surface may span the full clip viewport, while native
    /// text insets reduce the actual wrapping width. Keeping this value in
    /// the coordinator makes metrics, shaped hit-testing and render layout
    /// share one width contract.
    func setContentWidth(_ width: CGFloat) {
        let next = max(width, 1.0)
        if let current = contentWidth, abs(current - next) <= 0.5 {
            return
        }
        contentWidth = next
        scheduleSubmit()
    }

    /// 屏幕上那一帧是否就是当前状态该有的那一帧。
    ///
    /// 判断整个交给 Rust：编辑状态、composition、selection 与表格 resize 覆盖
    /// 都在那边，平台只递上自己知道的几何。可见的旧帧不算数——编辑、滚动、
    /// 缩放或改变 backing scale 之后，替换帧真正到达 surface 之前都必须判为
    /// 「不是当前帧」。
    func hasCurrentFrame() -> Bool {
        guard isAttached, let geometry = currentFrameGeometry else {
            return false
        }
        return frameIsCurrent(geometry)
    }

    /// 把平台几何递给 Rust 的适配器。`FrameGeometry` 是本协调器的内部形状，
    /// 不应该出现在 bridge 的签名里。
    private func frameIsCurrent(_ geometry: FrameGeometry) -> Bool {
        bridge.macosFrameIsCurrent(
            size: Float(geometry.size),
            maxWidth: Float(geometry.maxWidth),
            scrollY: Float(geometry.scrollY),
            viewportHeight: Float(geometry.viewportHeight),
            surfaceWidth: Double(geometry.surfaceWidth),
            surfaceHeight: Double(geometry.surfaceHeight),
            scale: Double(geometry.scale)
        )
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
        max(contentWidth ?? surfaceView.bounds.width, 1.0)
    }

    func scheduleSubmit() {
        imageRefreshAttempts = 0
        guard !submitScheduled else { return }
        submitScheduled = true
        let token = scheduleToken
        DispatchQueue.main.async { [weak self] in
            guard let self, self.scheduleToken == token else { return }
            self.submitScheduled = false
            do {
                _ = try self.submitNow()
            } catch {
                self.clearTableResizeState()
                self.imageRefreshNeeded = false
                self.cancelImageResourceRefresh()
                self.surfaceView?.setNativeContentVisible(false)
                self.onSurfaceStateChange?()
                self.onError?(error)
            }
        }
    }

    private func cancelImageResourceRefresh() {
        imageRefreshTask?.cancel()
        imageRefreshTask = nil
        imageRefreshAttempts = 0
    }

    /// Polls only while the current viewport has an image resource that is
    /// pending, failed, or otherwise unproven. The Rust worker is deliberately
    /// asynchronous and has no callback into AppKit, so a short bounded poll
    /// is the smallest safe bridge. Every attempt invalidates the submit key
    /// to force Rust to drain worker results and republish the frame.
    private func scheduleImageResourceRefresh() {
        guard imageRefreshNeeded,
              isAttached,
              imageRefreshTask == nil,
              imageRefreshAttempts < Self.maxImageRefreshAttempts else {
            return
        }
        let token = scheduleToken
        imageRefreshAttempts += 1
        let task = DispatchWorkItem { [weak self] in
            guard let self,
                  self.scheduleToken == token,
                  self.isAttached else {
                return
            }
            self.imageRefreshTask = nil
            do {
                // 强制提交：几何与编辑状态都没变，Rust 会判为「当前帧」，
                // 但这次提交的目的正是让它去收割 worker 的结果并重新发布。
                // 这个 force 是资源刷新判断仍留在平台侧的直接后果，随该判断
                // 一起移入 Rust 后即可消失。
                _ = try self.submitNow(force: true)
            } catch {
                self.imageRefreshNeeded = false
                self.cancelImageResourceRefresh()
                self.surfaceView?.setNativeContentVisible(false)
                self.onSurfaceStateChange?()
                self.onError?(error)
            }
        }
        imageRefreshTask = task
        DispatchQueue.main.asyncAfter(deadline: .now() + Self.imageRefreshDelay, execute: task)
    }

    /// Resolves the current document-space point against the same shaped
    /// table geometry used by the pointer begin path. Hover is intentionally
    /// read-only: it never opens a Rust resize session and silently falls
    /// back to the normal arrow when metrics or the Revision are stale.
    func tableResizeHover(at point: NSPoint) -> Bool {
        if let session = tableResizePointerState.session,
           session.revision == bridge.state.revision {
            return session.kind == YU_STORAGE_TABLE_RESIZE_COLUMN
        }
        guard !bridge.composition.active,
              point.x.isFinite,
              point.y.isFinite,
              let geometry = visualDecorationGeometry() else {
            return false
        }
        let revision = bridge.state.revision
        let tolerance = Float(max(CGFloat(6.0), fontSize * 0.4))
        do {
            let hit = try bridge.macosTableResizeAtDocumentPoint(
                revision: revision,
                action: UInt8(YU_STORAGE_TABLE_RESIZE_PROBE),
                size: geometry.size,
                maxWidth: geometry.maxWidth,
                point: CGPoint(x: point.x, y: point.y),
                tolerance: tolerance
            )
            return hit.kind == UInt8(YU_STORAGE_TABLE_RESIZE_COLUMN)
        } catch {
            return false
        }
    }

    /// Resolves a primary click against the exact Rust scene publication that
    /// is currently visible. The coordinator returns metadata only; canonical
    /// mutation stays in `DocumentTextView`'s existing command path.
    func taskCheckboxHit(at point: NSPoint) -> NativeTaskCheckboxHit? {
        let revision = bridge.state.revision
        guard !bridge.composition.active,
              point.x.isFinite,
              point.y.isFinite,
              hasCurrentFrame() else {
            return nil
        }
        do {
            let hit = try bridge.macosTaskCheckboxHitTest(
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

    /// Returns the visible, read-only divider descriptors from the same
    /// document-space CoreText layout used by hover and begin. Callers project
    /// these into ephemeral native Accessibility elements; the descriptors
    /// never retain a Rust layout or open a resize gesture.
    func tableResizeAccessibilityDividers() -> [NativeTableResizeAccessibilityDivider] {
        guard !bridge.composition.active,
              let geometry = visualDecorationGeometry() else {
            return []
        }
        do {
            return try bridge.macosTableResizeAccessibilityDividers(
                revision: bridge.state.revision,
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
        guard descriptor.revision == bridge.state.revision,
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
        guard descriptor.revision == bridge.state.revision,
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
        let revision = bridge.state.revision
        let tolerance = Float(max(CGFloat(6.0), fontSize * 0.4))
        do {
            let hit = try bridge.macosTableResizeAtDocumentPoint(
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
            let begun = try bridge.macosTableResizeAtDocumentPoint(
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
        let revision = bridge.state.revision
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
        let revision = bridge.state.revision
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
        let revision = bridge.state.revision
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
    func revealCaretIfNeeded() {
        guard let surfaceView,
              let scrollView,
              surfaceView.bounds.width > 0.0,
              surfaceView.bounds.height > 0.0 else {
            return
        }
        let revision = bridge.state.revision
        let size = max(fontSize, 1.0)
        let maxWidth = layoutWidth(for: surfaceView)
        let viewportBounds = scrollView.contentView.bounds
        let viewportHeight = max(viewportBounds.height, 1.0)
        let currentScrollY = max(viewportBounds.origin.y, 0.0)
        do {
            let request = try bridge.macosShapedCaretScrollRequest(
                revision: revision,
                size: Float(size),
                maxWidth: Float(maxWidth),
                scrollY: Float(currentScrollY),
                viewportHeight: Float(viewportHeight)
            )
            guard request.revision == revision,
                  request.currentScrollY.isFinite,
                  request.targetScrollY.isFinite,
                  request.targetScrollY >= 0.0,
                  request.needsScroll else {
                return
            }
            let nativeMaxScrollY = max(
                (scrollView.documentView?.bounds.height ?? 0.0) - viewportHeight,
                0.0
            )
            let targetScrollY = min(max(request.targetScrollY, 0.0), nativeMaxScrollY)
            guard abs(targetScrollY - currentScrollY) > 0.5 else { return }
            var origin = viewportBounds.origin
            origin.y = targetScrollY
            scrollView.contentView.setBoundsOrigin(origin)
            scrollView.reflectScrolledClipView(scrollView.contentView)
            scheduleSubmit()
        } catch {
            // Caret reveal is an enhancement to the source TextKit view. The
            // source mirror remains interactive if shaped metrics are stale,
            // unavailable, or temporarily racing a document edit.
        }
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
        let target = max(contentHeight, scrollView.contentView.bounds.height)
        if let textView = documentView as? NSTextView {
            textView.minSize = NSSize(width: 0.0, height: target)
            textView.maxSize = NSSize(
                width: CGFloat.greatestFiniteMagnitude,
                height: target
            )
        }
        guard abs(documentView.frame.height - target) > 0.5 else { return }
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
    }

    /// 提交一帧。
    ///
    /// `force` 只为资源刷新轮询而存在：那条路径需要一次真实提交去收割
    /// 异步 worker 的结果，而此时几何与编辑状态都没有变化，Rust 会正确地
    /// 判定「与屏幕上的帧等价」。资源刷新判断移入 Rust 后这个参数即可删除。
    @discardableResult
    func submitNow(force: Bool = false) throws -> NativeMacosRenderHostSurfaceSnapshot? {
        guard let surfaceView,
              let geometry = currentFrameGeometry else {
            return nil
        }

        if !force, isAttached, frameIsCurrent(geometry) {
            // Rust surface 是唯一渲染路径：attach 之后一直可见，
            // 内容由 retained frame 决定，不由 coverage 决定（不变量 I5）。
            surfaceView.setNativeContentVisible(true)
            scheduleImageResourceRefresh()
            return lastSnapshot
        }

        let revision = bridge.state.revision
        let rawView = Unmanaged.passUnretained(surfaceView).toOpaque()
        let snapshot = try bridge.macosRenderHostSurfaceSubmit(
            revision: revision,
            size: Float(geometry.size),
            maxWidth: Float(geometry.maxWidth),
            scrollY: Float(geometry.scrollY),
            viewportHeight: Float(geometry.viewportHeight),
            surfaceWidth: Double(geometry.surfaceWidth),
            surfaceHeight: Double(geometry.surfaceHeight),
            scale: Double(geometry.scale),
            view: rawView
        )
        isAttached = true
        lastSnapshot = snapshot
        applyContentHeight(snapshot.contentHeight)
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
        return snapshot
    }

    func detach() {
        scheduleToken &+= 1
        submitScheduled = false
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
            revision: bridge.state.revision,
            action: UInt8(YU_STORAGE_TABLE_RESIZE_CANCEL)
        )
        tableResizePointerState.reset()
    }

}
