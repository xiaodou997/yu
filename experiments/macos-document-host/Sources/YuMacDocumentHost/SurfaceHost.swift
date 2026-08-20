import AppKit
import Darwin
import Foundation
import UniformTypeIdentifiers
import YuStorageFFI

// Metal surface 的 AppKit 宿主与帧提交调度。
//
// 注：帧调度决策目前仍在 Swift 侧，每个决策点都要一次 FFI 查询去取
// Rust 的状态。S1 的后续工作是把这部分移入 Rust，见
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

    private struct MetricsKey: Equatable {
        let revision: UInt64
        let size: Double
        let maxWidth: Double
    }

    private struct Metrics {
        let key: MetricsKey
        let lineHeight: Float
        let defaultAdvance: Float
    }

    private struct SubmitKey: Equatable {
        let revision: UInt64
        let compositionGeneration: UInt64
        let size: Double
        let maxWidth: Double
        let scrollY: Double
        let viewportHeight: Double
        let surfaceWidth: Double
        let surfaceHeight: Double
        let scale: Double
    }

    private let bridge: StorageBridge
    private weak var surfaceView: MacosSurfaceHostView?
    private weak var scrollView: NSScrollView?
    private var fontSize: CGFloat
    private var contentWidth: CGFloat?
    private var metrics: Metrics?
    private var lastSubmitKey: SubmitKey?
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
        metrics = nil
        lastSubmitKey = nil
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
        metrics = nil
        lastSubmitKey = nil
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
        metrics = nil
        lastSubmitKey = nil
        scheduleSubmit()
    }

    /// Returns true only when the Metal surface has accepted the current Rust
    /// revision, transient composition generation and complete submit
    /// geometry. A visible old frame is deliberately insufficient: source
    /// glyphs must remain available until the replacement publication has
    /// reached the native surface after an edit, scroll, resize or scale
    /// change.
    func hasCurrentPublication(revision: UInt64, compositionGeneration: UInt64) -> Bool {
        guard isAttached,
              let snapshot = lastSnapshot,
              let currentKey = currentSubmitKey else {
            return false
        }
        return snapshot.submitted
            && snapshot.revision == revision
            && snapshot.compositionGeneration == compositionGeneration
            && currentKey.revision == revision
            && currentKey.compositionGeneration == compositionGeneration
            && lastSubmitKey == currentKey
    }

    private var currentSubmitKey: SubmitKey? {
        guard let surfaceView,
              let scrollView,
              let window = surfaceView.window,
              surfaceView.bounds.width > 0.0,
              surfaceView.bounds.height > 0.0 else {
            return nil
        }
        let viewportBounds = scrollView.contentView.bounds
        return SubmitKey(
            revision: bridge.state.revision,
            compositionGeneration: bridge.composition.generation,
            size: Double(max(fontSize, 1.0)),
            maxWidth: Double(layoutWidth(for: surfaceView)),
            scrollY: Double(max(viewportBounds.origin.y, 0.0)),
            viewportHeight: Double(max(viewportBounds.height, 1.0)),
            surfaceWidth: Double(max(surfaceView.bounds.width, 1.0)),
            surfaceHeight: Double(max(surfaceView.bounds.height, 1.0)),
            scale: Double(max(window.backingScaleFactor, 1.0))
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

    /// Selection/caret movement does not advance the canonical source
    /// Revision, but it does change retained editor-decoration geometry.
    /// Revoke the current visual publication before scheduling its replacement
    /// so equal primitive counts cannot make an old caret look current.
    func invalidateEditorDecorationPublication() {
        lastSubmitKey = nil
        lastSnapshot = nil
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
            self.lastSubmitKey = nil
            do {
                _ = try self.submitNow()
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

    /// Publishes CoreText metrics before a vertical editor command enters the
    /// Rust command path. This keeps the command's shaped line wrapping and
    /// the subsequent caret reveal on one Revision/width contract even when a
    /// key arrives before the next asynchronous surface submit.
    func prepareForEditorCommand() {
        guard let surfaceView,
              surfaceView.bounds.width > 0.0 else {
            return
        }
        let revision = bridge.state.revision
        let size = max(fontSize, 1.0)
        let maxWidth = layoutWidth(for: surfaceView)
        do {
            _ = try ensureMetrics(
                revision: revision,
                size: size,
                maxWidth: maxWidth
            )
        } catch {
            onError?(error)
        }
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
            let hit = try bridge.macosTableResizeHitTestAtDocumentPoint(
                revision: revision,
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
              hasCurrentPublication(
                  revision: revision,
                  compositionGeneration: bridge.composition.generation
              ) else {
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
              let geometry = visualDecorationGeometry() else {
            return false
        }
        let step = max(
            CGFloat(8.0),
            min(CGFloat(16.0), CGFloat(geometry.lineHeight) * 0.5)
        )
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
            let hit = try bridge.macosTableResizeHitTestAtDocumentPoint(
                revision: revision,
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
            let begun = try bridge.macosTableResizeBeginAtDocumentPoint(
                revision: revision,
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
            lastSubmitKey = nil
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
            _ = try bridge.tableResizeUpdate(
                revision: revision,
                pointerPosition: pointerPosition
            )
            lastSubmitKey = nil
            scheduleSubmit()
            return true
        } catch BridgeError.operation(let status)
            where status == StorageStatus.staleRevision
                || status == StorageStatus.tableResizeNotActive {
            tableResizePointerState.reset()
            lastSubmitKey = nil
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
            _ = try bridge.tableResizeFinish(revision: revision)
            _ = tableResizePointerState.finish(revision: revision)
            lastSubmitKey = nil
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
            try bridge.tableResizeCancel(revision: revision)
            _ = tableResizePointerState.cancel(revision: revision)
            lastSubmitKey = nil
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

    /// Returns the same revision-bound CoreText metrics and viewport inputs
    /// used by the Rust render host. Visual decorations use this accessor so
    /// their geometry query cannot silently drift to TextKit's independent
    /// wrapping width or line height.
    func visualDecorationGeometry() -> (
        size: Float,
        maxWidth: Float,
        scrollY: Float,
        viewportHeight: Float,
        lineHeight: Float
    )? {
        guard let surfaceView,
              let scrollView,
              surfaceView.bounds.width > 0.0,
              surfaceView.bounds.height > 0.0 else {
            return nil
        }
        let revision = bridge.state.revision
        let size = max(fontSize, 1.0)
        let maxWidth = layoutWidth(for: surfaceView)
        let viewportBounds = scrollView.contentView.bounds
        let viewportHeight = max(viewportBounds.height, 1.0)
        let scrollY = max(viewportBounds.origin.y, 0.0)
        do {
            let metrics = try ensureMetrics(
                revision: revision,
                size: size,
                maxWidth: maxWidth
            )
            return (
                Float(size),
                Float(maxWidth),
                Float(scrollY),
                Float(viewportHeight),
                metrics.lineHeight
            )
        } catch {
            return nil
        }
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
            let metrics = try ensureMetrics(
                revision: revision,
                size: size,
                maxWidth: maxWidth
            )
            let request = try bridge.macosShapedCaretScrollRequest(
                revision: revision,
                size: Float(size),
                maxWidth: Float(maxWidth),
                scrollY: Float(currentScrollY),
                viewportHeight: Float(viewportHeight),
                margin: max(metrics.lineHeight, 4.0)
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
            lastSubmitKey = nil
            scheduleSubmit()
        } catch {
            // Caret reveal is an enhancement to the source TextKit view. The
            // source mirror remains interactive if shaped metrics are stale,
            // unavailable, or temporarily racing a document edit.
        }
    }

    /// 扫描当前 viewport 的图片与嵌入资源，决定是否需要安排一次刷新。
    ///
    /// 这里不再做 retained coverage 判断：Rust surface 是唯一渲染路径，
    /// 资源未就绪时由 Rust 绘制 placeholder，不存在回退 TextKit 的分支
    /// （不变量 I5）。资源状态只影响「要不要再取一次」，不影响「谁来画」。
    private func updateResourceRefreshState(
        snapshot: NativeMacosRenderHostSurfaceSnapshot,
        revision: UInt64,
        size: Float,
        maxWidth: Float,
        scrollY: Float,
        viewportHeight: Float
    ) {
        imageRefreshNeeded = false
        guard snapshot.commandCount > 0 else { return }
        do {
            let (viewport, blocks) = try bridge.macosShapedViewportBlocks(
                revision: revision,
                size: size,
                maxWidth: maxWidth,
                scrollY: scrollY,
                viewportHeight: viewportHeight
            )
            guard viewport.revision == revision else { return }
            let visibleBlockIndexes = Set(blocks.map(\.blockIndex))
            let images = try bridge.macosVisualImages(revision: revision)
            for image in images where visibleBlockIndexes.contains(image.blockIndex) {
                if retainedResourceNeedsRefresh(
                    state: imageResourceCoverageState(image.resourceStatus),
                    resourceFingerprint: image.resourceFingerprint
                ) {
                    imageRefreshNeeded = true
                }
            }
            guard blocks.contains(where: {
                $0.kind == UInt8(YU_STORAGE_PROJECTION_BLOCK_FENCED_CODE)
            }) else { return }
            let embeddedResources = try bridge.macosVisualEmbeddedResources(
                revision: revision
            )
            for resource in embeddedResources
                where visibleBlockIndexes.contains(resource.blockIndex) {
                if retainedResourceNeedsRefresh(
                    state: embeddedResourceCoverageState(resource.resourceStatus),
                    resourceFingerprint: resource.resourceFingerprint
                ) {
                    imageRefreshNeeded = true
                }
            }
        } catch {
            // 资源查询失败不影响绘制；下一帧会重试。
        }
    }

    @discardableResult
    func submitNow() throws -> NativeMacosRenderHostSurfaceSnapshot? {
        guard let surfaceView,
              let scrollView,
              let window = surfaceView.window,
              surfaceView.bounds.width > 0.0,
              surfaceView.bounds.height > 0.0 else {
            return nil
        }

        let revision = bridge.state.revision
        let size = max(fontSize, 1.0)
        let surfaceWidth = max(surfaceView.bounds.width, 1.0)
        let surfaceHeight = max(surfaceView.bounds.height, 1.0)
        let viewportBounds = scrollView.contentView.bounds
        let viewportHeight = max(viewportBounds.height, 1.0)
        let scrollY = max(viewportBounds.origin.y, 0.0)
        let maxWidth = layoutWidth(for: surfaceView)
        let scale = max(window.backingScaleFactor, 1.0)
        // Composition updates do not advance the canonical Revision. Include
        // the Rust-owned generation in the submit key so every marked-text
        // update/cancel publishes a fresh transient glyph scene instead of
        // reusing the previous frame by geometry alone.
        let compositionGeneration = bridge.composition.generation
        let key = SubmitKey(
            revision: revision,
            compositionGeneration: compositionGeneration,
            size: Double(size),
            maxWidth: Double(maxWidth),
            scrollY: Double(scrollY),
            viewportHeight: Double(viewportHeight),
            surfaceWidth: Double(surfaceWidth),
            surfaceHeight: Double(surfaceHeight),
            scale: Double(scale)
        )
        if isAttached, key == lastSubmitKey {
            // A same-key submit can be reached after the controller has
            // rejected an empty plan. Do not briefly re-show that blank
            // surface; source TextKit remains the canonical visible fallback
            // until a publication with actual draw commands exists.
            // Rust surface 是唯一渲染路径：attach 之后一直可见，
            // 内容由 retained frame 决定，不由 coverage 决定（不变量 I5）。
            surfaceView.setNativeContentVisible(true)
            scheduleImageResourceRefresh()
            return lastSnapshot
        }

        _ = try ensureMetrics(
            revision: revision,
            size: size,
            maxWidth: maxWidth
        )
        let rawView = Unmanaged.passUnretained(surfaceView).toOpaque()
        let snapshot = try bridge.macosRenderHostSurfaceSubmit(
            revision: revision,
            size: Float(size),
            maxWidth: Float(maxWidth),
            scrollY: Float(scrollY),
            viewportHeight: Float(viewportHeight),
            surfaceWidth: Double(surfaceWidth),
            surfaceHeight: Double(surfaceHeight),
            scale: Double(scale),
            view: rawView
        )
        isAttached = true
        lastSubmitKey = key
        lastSnapshot = snapshot
        updateResourceRefreshState(
            snapshot: snapshot,
            revision: revision,
            size: Float(size),
            maxWidth: Float(maxWidth),
            scrollY: Float(scrollY),
            viewportHeight: Float(viewportHeight)
        )
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
        lastSubmitKey = nil
        lastSnapshot = nil
        imageRefreshNeeded = false
        metrics = nil
        surfaceView?.setNativeContentVisible(false)
        onSurfaceStateChange?()
    }

    private func clearTableResizeState() {
        try? bridge.tableResizeCancel(revision: bridge.state.revision)
        tableResizePointerState.reset()
        lastSubmitKey = nil
    }

    private func ensureMetrics(
        revision: UInt64,
        size: CGFloat,
        maxWidth: CGFloat
    ) throws -> Metrics {
        let key = MetricsKey(
            revision: revision,
            size: Double(size),
            maxWidth: Double(maxWidth)
        )
        if let metrics, metrics.key == key {
            return metrics
        }
        let layout = try bridge.macosFontMetrics(
            revision: revision,
            size: Float(size),
            maxWidth: Float(maxWidth)
        )
        guard layout.revision == revision,
              abs(layout.size - size) <= 0.001,
              layout.lineHeight.isFinite,
              layout.lineHeight > 0.0,
              layout.defaultAdvance.isFinite,
              layout.defaultAdvance > 0.0 else {
            throw BridgeError.operation(StorageStatus.invalidViewport)
        }
        let next = Metrics(
            key: key,
            lineHeight: Float(layout.lineHeight),
            defaultAdvance: Float(layout.defaultAdvance)
        )
        try bridge.setViewportConfig(
            revision: revision,
            maxWidth: Float(maxWidth),
            lineHeight: next.lineHeight,
            defaultAdvance: next.defaultAdvance,
            estimatedBlockHeight: next.lineHeight,
            overscan: 0.0
        )
        metrics = next
        return next
    }
}
