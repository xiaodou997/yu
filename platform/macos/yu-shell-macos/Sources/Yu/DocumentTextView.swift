import AppKit
import Darwin
import Foundation
import UniformTypeIdentifiers
import YuStorageFFI

// `NSTextInputClient` 与 Accessibility 宿主。它不绘制任何像素——
// Rust surface 是唯一渲染路径（不变量 I5）——只负责把原生输入事件
// 转成 Rust command，并把 OS 的几何查询转给 Rust layout。

/// The native source mirror is deliberately a view cache, never a second
/// document model. Rust owns canonical source, revision, selection and
/// composition generation; this TextKit object only projects those values for
/// AppKit's NSTextInputClient callbacks. The visual pointer adapter asks
/// Rust's CoreText-shaped block layout for the visual boundary, then maps that
/// boundary back to canonical source ranges. The disposable TextKit visual
/// mirror remains a geometry and input/IME/accessibility host.
final class DocumentTextView: NSTextView {
    private enum Command {
        static let deleteBackward: UInt8 = 1
        static let deleteForward: UInt8 = 2
        static let moveLeft: UInt8 = 3
        static let moveRight: UInt8 = 4
        static let insertNewline: UInt8 = 5
        static let indentList: UInt8 = 6
        static let outdentList: UInt8 = 7
        static let undo: UInt8 = 8
        static let redo: UInt8 = 9
        static let toggleTask: UInt8 = 10
        static let moveWordLeft: UInt8 = 11
        static let moveWordRight: UInt8 = 12
        static let moveUp: UInt8 = 13
        static let moveDown: UInt8 = 14
        static let moveUpExtend: UInt8 = 15
        static let moveDownExtend: UInt8 = 16
    }


    private let bridge: StorageBridge
    private var canonicalSource: String
    private var canonicalRevision: UInt64
    private var semanticNodes: [NativeAccessibilitySemanticNode] = []
    private var semanticElements: [YuAccessibilitySemanticElement] = []
    private var tableResizeAccessibilityDescriptors:
        [NativeTableResizeAccessibilityDivider] = []
    private var tableResizeAccessibilityElements:
        [YuAccessibilityTableResizeElement] = []
    private var headingRotorDelegate: YuAccessibilityRotorDelegate!
    private var linkRotorDelegate: YuAccessibilityRotorDelegate!
    private var nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
    private var synchronizingSelection = false
    private var visualCompositionGeneration: UInt64?
    private var visualSelectionAnchor: Int?
    private var tableResizeTrackingArea: NSTrackingArea?
    private var tableResizeCursorActive = false
    private var taskCheckboxPointerConsumed = false
    var onDocumentChange: (() -> Void)?
    var onCaretChange: (() -> Void)?
    var onError: ((Error) -> Void)?
    var onTableResizeHover: ((NSPoint) -> Bool)?
    var onTaskCheckboxPress: ((NSPoint) -> Bool)?
    var onTableResizeBegin: ((NSPoint) -> Bool)?
    var onTableResizeUpdate: ((NSPoint) -> Bool)?
    var onTableResizeFinish: (() -> Bool)?
    var onTableResizeCancel: (() -> Bool)?
    var tableResizeAccessibilityProvider:
        (() -> [NativeTableResizeAccessibilityDivider])?
    var tableResizeAccessibilityFrameProvider:
        ((NativeTableResizeAccessibilityDivider) -> NSRect)?
    var onTableResizeAccessibilityAction:
        ((NativeTableResizeAccessibilityDivider, Int) -> Bool)?

    init(bridge: StorageBridge) {
        self.bridge = bridge
        canonicalSource = bridge.source
        canonicalRevision = bridge.state.revision
        // NSTextView's frame-only convenience initializer dynamically
        // dispatches to `init(frame:textContainer:)` on subclasses. Because
        // this view owns its bridge and is not storyboard-decoded, construct
        // the TextKit chain explicitly and call the designated initializer.
        // A nil text container can leave a source-backed mirror readable via
        // AX while providing no drawable storage/layout for the native view.
        let textStorage = NSTextStorage()
        let layoutManager = NSLayoutManager()
        let textContainer = NSTextContainer(
            size: NSSize(width: 900, height: CGFloat.greatestFiniteMagnitude)
        )
        layoutManager.addTextContainer(textContainer)
        textStorage.addLayoutManager(layoutManager)
        super.init(frame: .zero, textContainer: textContainer)
        isEditable = true
        isSelectable = true
        isRichText = false
        importsGraphics = false
        allowsUndo = false
        usesFindBar = true
        font = NSFont.systemFont(ofSize: 16)
        textColor = NSColor.textColor
        backgroundColor = NSColor.textBackgroundColor
        setAccessibilityElement(true)
        setAccessibilityRole(.textArea)
        setAccessibilityLabel("Yu Markdown 文档")
        setAccessibilityIdentifier("yu-document-text")
        headingRotorDelegate = YuAccessibilityRotorDelegate(owner: self, kind: .heading)
        linkRotorDelegate = YuAccessibilityRotorDelegate(owner: self, kind: .link)
        minSize = NSSize(width: 0, height: 0)
        maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        // 可滚动范围由 Rust 这一帧的内容高度决定（见
        // `MacosSurfaceHostCoordinator.applyContentHeight`）。这个视图不绘制
        // 任何像素，它自己的 TextKit 排版高度不该成为第二个滚动范围来源——
        // 两套高度不一致时长文档尾部滚不到，而且它是在窗口出现之后异步长出来
        // 的，会把视口一起拖走。
        isVerticallyResizable = false
        isHorizontallyResizable = false
        autoresizingMask = [.width]
        semanticNodes = bridge.accessibilitySemanticNodesIfAvailable ?? []
        rebuildSemanticAccessibilityTree()
        synchronizeProjection()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

    override func updateTrackingAreas() {
        if let tableResizeTrackingArea {
            removeTrackingArea(tableResizeTrackingArea)
        }
        let area = NSTrackingArea(
            rect: bounds,
            options: [
                .mouseEnteredAndExited,
                .mouseMoved,
                .activeInKeyWindow,
                .inVisibleRect
            ],
            owner: self,
            userInfo: nil
        )
        tableResizeTrackingArea = area
        addTrackingArea(area)
        super.updateTrackingAreas()
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        window?.acceptsMouseMovedEvents = true
        if window == nil {
            setTableResizeCursor(active: false)
        }
    }

    override func mouseMoved(with event: NSEvent) {
        let divider = onTableResizeHover?(visualPoint(for: event)) ?? false
        setTableResizeCursor(active: divider)
        super.mouseMoved(with: event)
    }

    override func mouseExited(with event: NSEvent) {
        setTableResizeCursor(active: false)
        super.mouseExited(with: event)
    }

    func refreshFromRust() {
        canonicalSource = bridge.source
        canonicalRevision = bridge.state.revision
        semanticNodes = bridge.accessibilitySemanticNodesIfAvailable ?? []
        nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
        synchronizeProjection()
        postAccessibilityRefresh()
    }





















    @discardableResult
    func applyVisualPointerSelectionForSelfCheck(
        at point: NSPoint,
        extending: Bool = false
    ) -> Bool {
        applyVisualPointerSelection(at: point, extending: extending)
    }


    @discardableResult
    func applyVisualSelectionForSelfCheck(_ visualRange: NSRange) -> Bool {
        applyVisualSelection(visualRange)
    }


    private func currentCompositionGeneration() -> UInt64? {
        guard bridge.composition.active else { return nil }
        return try? bridge.compositionProjection(revision: bridge.state.revision).generation
    }

    private func visualPoint(for event: NSEvent) -> NSPoint {
        let local = convert(event.locationInWindow, from: nil)
        return NSPoint(
            x: local.x - textContainerOrigin.x,
            y: local.y - textContainerOrigin.y
        )
    }

    private func setTableResizeCursor(active: Bool) {
        let next = active && window != nil
        guard tableResizeCursorActive != next else { return }
        tableResizeCursorActive = next
        (next ? NSCursor.resizeLeftRight : NSCursor.arrow).set()
    }

    /// Resolves a visual document point through the Rust CoreText-shaped
    /// block layout. TextKit remains the input/IME/accessibility host, but it
    /// must not guess glyph boundaries for production pointer selection.
    /// 命中测试完全由 Rust layout 完成。此处不再用 TextKit 布局出的
    /// visual 长度做上界校验——那等于用第二套布局系统验证第一套，
    /// 而第二套布局系统本身就是要消除的对象（不变量 I5、E1）。
    /// Rust 返回的 visualUTF16 已绑定同一 Revision，越界由 Rust 侧拒绝。
    private func shapedVisualOffset(at point: NSPoint) -> Int? {
        guard point.x.isFinite,
              point.y.isFinite,
              let (size, width) = visualLayoutMetrics(),
              let hit = try? bridge.macosProjectionHitTest(
                  revision: bridge.state.revision,
                  point: CGPoint(x: point.x, y: point.y),
                  size: size,
                  maxWidth: width
              ),
              hit.revision == bridge.state.revision,
              hit.point.x.isFinite,
              hit.point.y.isFinite,
              let visualOffset = Int(exactly: hit.visualUTF16),
              visualOffset >= 0 else {
            return nil
        }
        return visualOffset
    }

    @discardableResult
    private func applyVisualPointerSelection(
        at point: NSPoint,
        extending: Bool
    ) -> Bool {
        guard !bridge.composition.active else { return false }
        guard let visualOffset = shapedVisualOffset(at: point) else {
            // Rust 端对 Revision 与已发布的 viewport metrics 有意严格。
            // 几何过期时放弃本次指针选区，等下一帧重试。
            return false
        }
        if !extending || visualSelectionAnchor == nil {
            if extending {
                let endpoints = bridge.selectionEndpoints
                let sourceUTF16 = endpoints.anchorUTF16
                visualSelectionAnchor = visualUTF16ForSource(
                    sourceUTF16,
                    affinity: endpoints.affinity
                ) ?? visualOffset
            } else {
                visualSelectionAnchor = visualOffset
            }
        }
        guard let anchor = visualSelectionAnchor else { return false }
        let visualRange = NSRange(
            location: min(anchor, visualOffset),
            length: abs(visualOffset - anchor)
        )
        return applyVisualSelection(
            visualRange,
            anchorIsVisualStart: anchor <= visualOffset
        )
    }

    private func visualUTF16ForSource(
        _ sourceUTF16: UInt64,
        affinity: UInt8
    ) -> Int? {
        let revision = bridge.state.revision
        guard let caret = try? bridge.projectionCaret(
            revision: revision,
            sourceUTF16: sourceUTF16,
            affinity: affinity
        ),
              caret.revision == revision,
              let visualUTF16 = Int(exactly: caret.visualUTF16),
              visualUTF16 >= 0 else {
            return nil
        }
        return visualUTF16
    }

    @discardableResult
    private func applyVisualSelection(
        _ visualRange: NSRange,
        anchorIsVisualStart: Bool? = nil
    ) -> Bool {
        guard !bridge.composition.active,
              visualRange.location >= 0,
              visualRange.length >= 0 else {
            return false
        }
        do {
            let source = try bridge.projectionSourceSelection(
                revision: bridge.state.revision,
                visualRange: visualRange,
                affinity: 1
            )
            if let anchorIsVisualStart {
                let anchorUTF16 = anchorIsVisualStart
                    ? UInt64(source.sourceRange.location)
                    : UInt64(NSMaxRange(source.sourceRange))
                let focusUTF16 = anchorIsVisualStart
                    ? UInt64(NSMaxRange(source.sourceRange))
                    : UInt64(source.sourceRange.location)
                try bridge.setSelectionEndpoints(
                    anchorUTF16: anchorUTF16,
                    focusUTF16: focusUTF16,
                    affinity: source.affinity
                )
            } else {
                try bridge.setSelection(source.sourceRange, affinity: source.affinity)
            }
            canonicalRevision = bridge.state.revision
            synchronizingSelection = true
            super.setSelectedRange(source.sourceRange)
            synchronizingSelection = false
            postSelectionChanged()
            return true
        } catch {
            synchronizingSelection = false
            // A stale visual point is an expected race with source editing;
            // let the caller fall back to AppKit's source hit-test instead of
            // interrupting typing with a modal error.
            return false
        }
    }

    // These queries deliberately read a fresh Rust snapshot instead of
    // trusting TextKit's disposable projection. TextKit remains the source
    // mirror and AppKit fallback surface; source text, UTF-16 length,
    // selection and logical line ranges remain Revision-bound Rust data.
    override func accessibilityValue() -> String? {
        bridge.copySourceIfAvailable ?? canonicalSource
    }

    override func accessibilityNumberOfCharacters() -> Int {
        bridge.accessibilitySnapshotIfAvailable?.numberOfCharacters
            ?? (canonicalSource as NSString).length
    }

    override func accessibilitySelectedText() -> String? {
        guard let snapshot = bridge.accessibilitySnapshotIfAvailable else { return nil }
        return bridge.copySourceRangeIfAvailable(
            snapshot.selectedRange,
            revision: snapshot.revision
        )
    }

    override func accessibilitySelectedTextRange() -> NSRange {
        bridge.accessibilitySnapshotIfAvailable?.selectedRange ?? selectedRange()
    }

    override func setAccessibilitySelectedTextRange(_ range: NSRange) {
        do {
            if bridge.compositionIfAvailable?.active == true {
                try bridge.cancelComposition()
                nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
            }
            guard let snapshot = bridge.accessibilitySnapshotIfAvailable else { return }
            guard let valid = accessibilitySourceRange(range, snapshot: snapshot) else { return }
            try bridge.setSelection(valid, affinity: snapshot.affinity)
            synchronizeProjection()
            postSelectionChanged()
        } catch {
            onError?(error)
        }
    }

    override func accessibilitySelectedTextRanges() -> [NSValue]? {
        [NSValue(range: accessibilitySelectedTextRange())]
    }

    /// AppKit asks for these children through Objective-C Accessibility
    /// dispatch. The document TextKit element remains the editable source
    /// surface; semantic children and visible table splitters are stable,
    /// Revision-bound elements and never become a second text model.
    @objc var accessibilityChildren: [Any]? {
        semanticElements.map { $0 as Any } + tableResizeAccessibilityElements
            .map { $0 as Any }
    }

    @objc var accessibilityChildrenInNavigationOrder: [Any]? {
        semanticElements.map { $0 as Any } + tableResizeAccessibilityElements
            .map { $0 as Any }
    }

    /// Expose the same elements through AppKit's dedicated splitter
    /// attribute. VoiceOver may discover them either as document children or
    /// through this role-specific collection.
    @objc var accessibilitySplitters: [Any]? {
        tableResizeAccessibilityElements.map { $0 as Any }
    }

    /// Heading and Link rotors make the semantic tree discoverable without
    /// requiring VoiceOver to walk every paragraph and inline child. The
    /// delegates remain owned by the document view because AppKit retains
    /// rotor delegates weakly.
    @objc var accessibilityCustomRotors: [NSAccessibilityCustomRotor]? {
        [
            NSAccessibilityCustomRotor(
                rotorType: .heading,
                itemSearchDelegate: headingRotorDelegate
            ),
            NSAccessibilityCustomRotor(
                rotorType: .link,
                itemSearchDelegate: linkRotorDelegate
            ),
        ]
    }

    func accessibilityRotorResult(
        for kind: SemanticAccessibilityKind,
        parameters: NSAccessibilityCustomRotor.SearchParameters
    ) -> NSAccessibilityCustomRotor.ItemResult? {
        let candidates = flattenSemanticElements(semanticElements).filter { element in
            guard let elementKind = SemanticAccessibilityKind(rawValue: element.node.kind) else {
                return false
            }
            switch kind {
            case .heading:
                return elementKind == .heading
            case .link:
                return elementKind == .link
                    || elementKind == .autolink
                    || elementKind == .referenceLink
            default:
                return false
            }
        }.filter { element in
            let filter = parameters.filterString
            guard !filter.isEmpty else { return true }
            return element.accessibilityLabel?.localizedCaseInsensitiveContains(filter) == true
        }
        guard !candidates.isEmpty else { return nil }

        let current = parameters.currentItem?.targetElement as AnyObject?
        let currentIndex = current.flatMap { currentObject in
            candidates.firstIndex { $0 === currentObject }
        }
        let index: Int?
        switch parameters.searchDirection {
        case .next:
            index = currentIndex.map { $0 + 1 } ?? 0
        case .previous:
            index = currentIndex.map { $0 - 1 } ?? (candidates.count - 1)
        @unknown default:
            index = nil
        }
        guard let index, candidates.indices.contains(index) else { return nil }

        let element = candidates[index]
        let result = NSAccessibilityCustomRotor.ItemResult(targetElement: element)
        result.targetRange = element.node.sourceRange
        result.customLabel = element.accessibilityLabel
        return result
    }

    func toggleTaskAccessibilityNode(_ node: NativeAccessibilitySemanticNode) -> Bool {
        guard SemanticAccessibilityKind(rawValue: node.kind) == .taskListItem,
              node.revision == bridge.state.revision,
              let block = node.actionBlock else {
            return false
        }
        return toggleTask(block: block, revision: node.revision)
    }

    private func toggleTask(block: UInt64, revision: UInt64) -> Bool {
        guard revision == bridge.state.revision,
              !bridge.composition.active,
              bridge.commandAvailable(Command.toggleTask, block: block) else {
            return false
        }
        do {
            let result = try bridge.executeCommand(Command.toggleTask, block: block)
            guard result.changed else { return false }
            apply(result)
            synchronizeProjection()
            postAccessibilityRefresh()
            onDocumentChange?()
            return true
        } catch {
            onError?(error)
            return false
        }
    }

    func toggleTaskPointerHit(_ hit: NativeTaskCheckboxHit) -> Bool {
        toggleTask(block: hit.blockIndex, revision: hit.revision)
    }

    @discardableResult
    func pressTaskCheckboxForSelfCheck(at point: NSPoint) -> Bool {
        onTaskCheckboxPress?(point) ?? false
    }

    override func accessibilityString(for range: NSRange) -> String? {
        guard let snapshot = bridge.accessibilitySnapshotIfAvailable else { return nil }
        guard let valid = accessibilitySourceRange(range, snapshot: snapshot) else { return nil }
        return bridge.copySourceRangeIfAvailable(valid, revision: snapshot.revision)
    }

    override func accessibilityAttributedString(for range: NSRange) -> NSAttributedString? {
        guard let text = accessibilityString(for: range) else { return nil }
        return NSAttributedString(string: text)
    }

    override func accessibilityRange(forLine line: Int) -> NSRange {
        guard let snapshot = bridge.accessibilitySnapshotIfAvailable else {
            return NSRange(location: NSNotFound, length: 0)
        }
        guard line >= 0, line < snapshot.lineCount else {
            return NSRange(location: NSNotFound, length: 0)
        }
        return bridge.accessibilityLineRange(line, revision: snapshot.revision)?.range
            ?? NSRange(location: NSNotFound, length: 0)
    }

    override func accessibilityLine(for index: Int) -> Int {
        guard let snapshot = bridge.accessibilitySnapshotIfAvailable else { return NSNotFound }
        guard index >= 0, index <= snapshot.numberOfCharacters else { return NSNotFound }
        return bridge.accessibilityLine(for: index, revision: snapshot.revision) ?? NSNotFound
    }

    override func accessibilityInsertionPointLineNumber() -> Int {
        accessibilityLine(for: accessibilitySelectedTextRange().location)
    }

    override func accessibilityRange(for index: Int) -> NSRange {
        guard let snapshot = bridge.accessibilitySnapshotIfAvailable else {
            return NSRange(location: NSNotFound, length: 0)
        }
        guard index >= 0, index <= snapshot.numberOfCharacters else {
            return NSRange(location: NSNotFound, length: 0)
        }
        guard index < snapshot.numberOfCharacters else {
            return NSRange(location: index, length: 0)
        }
        let source = (bridge.copySourceIfAvailable ?? canonicalSource) as NSString
        return source.rangeOfComposedCharacterSequence(at: index)
    }

    override func setSelectedRange(_ charRange: NSRange) {
        let range = clampedRange(charRange, length: (string as NSString).length)
        let shouldSync = !synchronizingSelection
        synchronizingSelection = true
        super.setSelectedRange(range)
        synchronizingSelection = false
        guard shouldSync else { return }
        syncNativeSelectionToRust(range)
    }

    /// AppKit uses this plural entry point for mouse clicks, drag selection,
    /// and some TextKit accessibility paths. The Rust editor currently owns a
    /// single selection, so the first native range is the canonical one; the
    /// important part is that mouse selection cannot leave Rust at an older
    /// fixed line while the disposable TextKit mirror moves elsewhere.
    override func setSelectedRanges(
        _ ranges: [NSValue],
        affinity: NSSelectionAffinity,
        stillSelecting: Bool
    ) {
        let shouldSync = !synchronizingSelection
        synchronizingSelection = true
        super.setSelectedRanges(
            ranges,
            affinity: affinity,
            stillSelecting: stillSelecting
        )
        synchronizingSelection = false
        guard shouldSync else { return }
        let range = clampedRange(selectedRange(), length: (string as NSString).length)
        syncNativeSelectionToRust(range)
    }

    /// The visual pointer adapter resolves the click/drag point in the
    /// projected stream, then lets Rust convert that visual range into the
    /// canonical source selection. If the projected mirror is stale or
    /// unavailable, AppKit's source hit-test remains the safe fallback.
    override func mouseDown(with event: NSEvent) {
        if event.buttonNumber == 0,
           event.clickCount == 1,
           !event.modifierFlags.contains(.shift),
           onTaskCheckboxPress?(visualPoint(for: event)) == true {
            visualSelectionAnchor = nil
            taskCheckboxPointerConsumed = true
            return
        }
        taskCheckboxPointerConsumed = false
        if event.buttonNumber == 0,
           onTableResizeBegin?(visualPoint(for: event)) == true {
            visualSelectionAnchor = nil
            setTableResizeCursor(active: true)
            return
        }
        if applyVisualPointerSelection(
            at: visualPoint(for: event),
            extending: event.modifierFlags.contains(.shift)
        ) {
            return
        }
        visualSelectionAnchor = nil
        super.mouseDown(with: event)
    }

    override func mouseDragged(with event: NSEvent) {
        if taskCheckboxPointerConsumed {
            return
        }
        if onTableResizeUpdate?(visualPoint(for: event)) == true {
            setTableResizeCursor(active: true)
            return
        }
        if visualSelectionAnchor != nil,
           applyVisualPointerSelection(
               at: visualPoint(for: event),
               extending: true
           ) {
            return
        }
        super.mouseDragged(with: event)
    }

    override func mouseUp(with event: NSEvent) {
        if taskCheckboxPointerConsumed {
            taskCheckboxPointerConsumed = false
            return
        }
        if event.buttonNumber == 0,
           onTableResizeFinish?() == true {
            setTableResizeCursor(active: false)
            return
        }
        if visualSelectionAnchor != nil {
            visualSelectionAnchor = nil
            return
        }
        super.mouseUp(with: event)
    }

    /// Source delimiters may be hidden by the Rust projection. Draw the
    /// insertion point at the revision-bound visual caret while retaining the
    /// TextKit view as the input/IME/Accessibility owner.
    /// caret 由 Rust retained decoration 绘制。TextKit 不贡献像素。
    override func drawInsertionPoint(
        in rect: NSRect,
        color: NSColor,
        turnedOn: Bool
    ) {}

    /// Rust surface 是唯一渲染路径（不变量 I5）。本视图仍然是
    /// `NSTextInputClient` 与 Accessibility 的宿主，但不绘制任何像素：
    /// 没有 fallback 路径，也就不需要判断「该不该画」。
    override func draw(_ rect: NSRect) {}

    private func syncNativeSelectionToRust(_ range: NSRange) {
        guard !bridge.composition.active else { return }
        do {
            try bridge.setSelection(range)
            canonicalRevision = bridge.state.revision
            postSelectionChanged()
        } catch {
            onError?(error)
        }
    }

    override func insertText(_ insertString: Any, replacementRange: NSRange) {
        let text = stringValue(insertString)
        guard !text.isEmpty || bridge.composition.active else { return }
        do {
            if bridge.composition.active {
                try bridge.commitComposition(text)
                canonicalSource = bridge.source
                canonicalRevision = bridge.state.revision
            } else {
                let target = replacementRange.location == NSNotFound
                    ? bridge.selection.range
                    : replacementRange
                if target != bridge.selection.range {
                    try bridge.setSelection(target)
                }
                let result = try bridge.insertText(text)
                apply(result)
            }
            nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
            synchronizeProjection()
            postAccessibilityRefresh()
            onDocumentChange?()
        } catch {
            onError?(error)
        }
    }

    override func copy(_ sender: Any?) {
        do {
            try finishCompositionForClipboard()
            let revision = bridge.state.revision
            let text = bridge.copySelection()
            guard !text.isEmpty else { return }
            let html = try bridge.copySelectionHTML(revision: revision)
            try publishSourceToPasteboard(text, html: html)
        } catch {
            onError?(error)
        }
    }

    override func cut(_ sender: Any?) {
        do {
            try finishCompositionForClipboard()
            let revision = bridge.state.revision
            let selected = bridge.copySelection()
            guard !selected.isEmpty else { return }
            let html = try bridge.copySelectionHTML(revision: revision)
            try publishSourceToPasteboard(selected, html: html)
            guard bridge.commandAvailable(Command.deleteBackward) else { return }
            apply(try bridge.executeCommand(Command.deleteBackward))
            synchronizeProjection()
            postAccessibilityRefresh()
            onDocumentChange?()
        } catch {
            onError?(error)
        }
    }

    override func paste(_ sender: Any?) {
        do {
            try finishCompositionForClipboard()
            guard let text = try sourceFromPasteboard(), !text.isEmpty else {
                return
            }
            apply(try bridge.insertText(text))
            synchronizeProjection()
            postAccessibilityRefresh()
            onDocumentChange?()
        } catch {
            onError?(error)
        }
    }

    var hasSourceOnPasteboard: Bool {
        let pasteboard = NSPasteboard.general
        return pasteboard.string(forType: .yuMarkdown) != nil
            || pasteboard.string(forType: .string) != nil
            || pasteboard.string(forType: .yuHTML) != nil
    }

    /// Headless self-check hook for the native pasteboard adapter. Production
    /// paste still uses `NSPasteboard.general`; this overload only lets the
    /// command-line check exercise the same priority/fallback logic on a
    /// private pasteboard.
    func sourceFromPasteboardForSelfCheck(_ pasteboard: NSPasteboard) throws -> String? {
        try sourceFromPasteboard(pasteboard)
    }

    /// Runs the production copy payload against a private pasteboard so the
    /// workflow smoke test never changes the user's global clipboard.
    func copyToPasteboardForSelfCheck(_ pasteboard: NSPasteboard) throws {
        try finishCompositionForClipboard()
        let revision = bridge.state.revision
        let text = bridge.copySelection()
        guard !text.isEmpty else { return }
        let html = try bridge.copySelectionHTML(revision: revision)
        try publishSourceToPasteboard(text, html: html, to: pasteboard)
    }

    /// Runs the production paste transaction against a private pasteboard.
    /// The same Rust selection/insert path is used; AppKit notifications are
    /// intentionally omitted because this is a headless check.
    func pasteFromPasteboardForSelfCheck(_ pasteboard: NSPasteboard) throws {
        try finishCompositionForClipboard()
        guard let text = try sourceFromPasteboard(pasteboard), !text.isEmpty else {
            return
        }
        apply(try bridge.insertText(text))
        synchronizeProjection()
    }

    override func selectAll(_ sender: Any?) {
        do {
            try finishCompositionForClipboard()
            let length = canonicalSource.utf16.count
            try bridge.setSelection(NSRange(location: 0, length: length))
            synchronizeProjection()
            postSelectionChanged()
        } catch {
            onError?(error)
        }
    }

    override func setMarkedText(
        _ markedText: Any,
        selectedRange: NSRange,
        replacementRange: NSRange
    ) {
        let text = stringValue(markedText)
        do {
            let active = bridge.composition
            let target = active.active
                ? active.replacementRange
                : (replacementRange.location == NSNotFound ? bridge.selection.range : replacementRange)
            if active.active {
                try bridge.updateComposition(preedit: text, selection: selectedRange)
            } else {
                try bridge.beginComposition(
                    replacementRange: target,
                    preedit: text,
                    selection: selectedRange
                )
            }
            let current = bridge.composition
            nativeMarkedRange = NSRange(
                location: current.replacementRange.location,
                length: text.utf16.count
            )
            synchronizeProjection()
            postAccessibilityRefresh()
            onDocumentChange?()
        } catch {
            onError?(error)
        }
    }

    override func unmarkText() {
        // AppKit's unmark is a presentation transition. The Rust overlay must
        // stay alive because some input sources deliver insertText afterwards.
        synchronizeProjection()
        nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
    }

    override func hasMarkedText() -> Bool {
        bridge.composition.active && nativeMarkedRange.location != NSNotFound
    }

    override func markedRange() -> NSRange {
        return nativeMarkedRange
    }

    override func attributedSubstring(
        forProposedRange proposedRange: NSRange,
        actualRange: NSRangePointer?
    ) -> NSAttributedString? {
        let range = clampedRange(proposedRange, length: (string as NSString).length)
        actualRange?.pointee = range
        guard range.location != NSNotFound else { return nil }
        return textStorage?.attributedSubstring(from: range)
    }

    override func validAttributesForMarkedText() -> [NSAttributedString.Key] {
        [.font, .foregroundColor, .underlineStyle]
    }

    override func doCommand(by selector: Selector) {
        let name = NSStringFromSelector(selector)
        if name == "cancel:" || name == "cancelOperation:" {
            if onTableResizeCancel?() == true {
                setTableResizeCursor(active: false)
                return
            }
            do {
                if bridge.composition.active {
                    try bridge.cancelComposition()
                    nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
                    synchronizeProjection()
                    postAccessibilityRefresh()
                    onDocumentChange?()
                }
            } catch { onError?(error) }
            return
        }
        if name == "undo:" {
            routeCommand(Command.undo)
            return
        }
        if name == "redo:" {
            routeCommand(Command.redo)
            return
        }

        let command: UInt8?
        switch selector {
        case #selector(NSResponder.deleteBackward(_:)): command = Command.deleteBackward
        case #selector(NSResponder.deleteForward(_:)): command = Command.deleteForward
        case #selector(NSResponder.moveLeft(_:)): command = Command.moveLeft
        case #selector(NSResponder.moveRight(_:)): command = Command.moveRight
        case #selector(NSResponder.moveWordLeft(_:)): command = Command.moveWordLeft
        case #selector(NSResponder.moveWordRight(_:)): command = Command.moveWordRight
        case #selector(NSResponder.moveUp(_:)): command = Command.moveUp
        case #selector(NSResponder.moveDown(_:)): command = Command.moveDown
        case #selector(NSResponder.moveUpAndModifySelection(_:)): command = Command.moveUpExtend
        case #selector(NSResponder.moveDownAndModifySelection(_:)): command = Command.moveDownExtend
        case #selector(NSResponder.insertNewline(_:)): command = Command.insertNewline
        case #selector(NSResponder.insertTab(_:)): command = Command.indentList
        case #selector(NSResponder.insertBacktab(_:)): command = Command.outdentList
        default: command = nil
        }
        guard let command else { return }
        routeCommand(command)
    }

    /// AppKit normally turns Command-Z into an `undo:` selector, but that
    /// path is not guaranteed when TextKit's own undo manager is disabled.
    /// Catch the native key equivalent as a second, explicit bridge to the
    /// Rust history. The menu actions below call the same method, so there
    /// is still only one undo/redo implementation.
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        let modifiers = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        let isCommandZ = modifiers.contains(.command)
            && !modifiers.contains(.option)
            && !modifiers.contains(.control)
            && event.charactersIgnoringModifiers?.lowercased() == "z"
        guard isCommandZ else {
            return super.performKeyEquivalent(with: event)
        }
        let command = modifiers.contains(.shift) ? Command.redo : Command.undo
        return routeCommand(command)
    }

    @discardableResult
    private func routeCommand(_ command: UInt8) -> Bool {
        guard !bridge.composition.active else { return false }
        let isVertical = command == Command.moveUp
            || command == Command.moveDown
            || command == Command.moveUpExtend
            || command == Command.moveDownExtend
        guard bridge.commandAvailable(command) else { return false }
        do {
            let result: NativeCommandResult
            if isVertical, let (size, width) = visualLayoutMetrics() {
                do {
                    result = try bridge.executeShapedVerticalCommand(
                        command,
                        size: size,
                        maxWidth: width
                    )
                } catch BridgeError.operation(let status)
                    where status == StorageStatus.invalidViewport {
                    // A key can arrive before the first surface/layout
                    // publication. Preserve editing availability by falling
                    // back to Rust's ordinary metrics command; the next
                    // command will retry the shaped path after preparation.
                    result = try bridge.executeCommand(command)
                }
            } else {
                result = try bridge.executeCommand(command)
            }
            apply(result)
            synchronizeProjection()
            postAccessibilityRefresh()
            onDocumentChange?()
            return true
        } catch {
            onError?(error)
            return false
        }
    }

    /// IME 候选窗定位。
    ///
    /// 必须用 Rust 几何：NSTextView 的默认实现基于 TextKit 布局，而 TextKit
    /// 排的是 canonical source，屏幕上显示的却是 Rust 的投影结果（未聚焦的
    /// Markdown 语法被隐藏），两者的字符位置并不对应。沿用默认实现会让候选窗
    /// 偏离真实插入点，违反不变量 H3（OS 查询的 caret rect 必须与当前编辑
    /// 状态一致）。
    ///
    /// 几何不可用时回退到默认实现：候选窗位置略偏，好过不显示。
    override func firstRect(
        forCharacterRange range: NSRange,
        actualRange: NSRangePointer?
    ) -> NSRect {
        guard let caretRect = rustCaretRect(forSourceUTF16: range.location) else {
            return super.firstRect(forCharacterRange: range, actualRange: actualRange)
        }
        actualRange?.pointee = NSRange(location: range.location, length: 0)
        let viewRect = NSRect(
            x: caretRect.origin.x + textContainerOrigin.x,
            y: caretRect.origin.y + textContainerOrigin.y,
            width: max(caretRect.width, 1.0),
            height: caretRect.height
        )
        let windowRect = convert(viewRect, to: nil)
        return window?.convertToScreen(windowRect) ?? windowRect
    }

    /// 当前 Revision 下某个 source offset 的 caret 矩形，document-space。
    /// composition 期间使用同一 generation 绑定的 transient 布局，
    /// 否则 preedit 的候选窗会落在提交前的旧位置。
    private func rustCaretRect(forSourceUTF16 sourceUTF16: Int) -> NSRect? {
        guard sourceUTF16 >= 0,
              let (size, width) = visualLayoutMetrics() else {
            return nil
        }
        let revision = bridge.state.revision
        let offset = UInt64(sourceUTF16)
        if bridge.composition.active {
            guard let caret = try? bridge.macosCompositionShapedCaret(
                revision: revision,
                generation: bridge.composition.generation,
                sourceUTF16: offset,
                affinity: 0,
                size: size,
                maxWidth: width
            ), caret.revision == revision else {
                return nil
            }
            return NSRect(origin: caret.point, size: caret.size)
        }
        guard let caret = try? bridge.macosSourceCaret(
            revision: revision,
            sourceUTF16: offset,
            affinity: 0,
            size: size,
            maxWidth: width
        ), caret.revision == revision else {
            return nil
        }
        return NSRect(
            origin: caret.point,
            size: CGSize(width: 1.0, height: caret.height)
        )
    }

    private func visualLayoutMetrics() -> (Float, Float)? {
        guard let font, bounds.width.isFinite, bounds.width > 0.0 else { return nil }
        let width = max(bounds.width - 2.0 * textContainerOrigin.x, 1.0)
        guard width.isFinite, width > 0.0 else { return nil }
        return (Float(max(font.pointSize, 1.0)), Float(width))
    }

    /// Menu actions use these explicit entry points instead of NSTextView's
    /// undo manager. Rust remains the sole owner of history and revision.
    func performUndo() {
        _ = routeCommand(Command.undo)
    }

    func performRedo() {
        _ = routeCommand(Command.redo)
    }

    func canUndo() -> Bool {
        !bridge.composition.active && bridge.commandAvailable(Command.undo)
    }

    func canRedo() -> Bool {
        !bridge.composition.active && bridge.commandAvailable(Command.redo)
    }

    private func finishCompositionForClipboard() throws {
        guard bridge.composition.active else { return }
        try bridge.cancelComposition()
        nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
        synchronizeProjection()
    }

    private func publishSourceToPasteboard(
        _ source: String,
        html: String,
        to pasteboard: NSPasteboard = .general
    ) throws {
        pasteboard.clearContents()
        guard pasteboard.setString(source, forType: .string),
              pasteboard.setString(source, forType: .yuMarkdown),
              pasteboard.setString(html, forType: .yuHTML) else {
            throw BridgeError.clipboard
        }
    }

    private func sourceFromPasteboard(_ pasteboard: NSPasteboard = .general) throws -> String? {
        if let markdown = pasteboard.string(forType: .yuMarkdown) {
            return markdown
        }
        if let plain = pasteboard.string(forType: .string) {
            return plain
        }
        guard let html = pasteboard.string(forType: .yuHTML) else {
            return nil
        }
        return try bridge.importHTML(html)
    }

    private func apply(_ result: NativeCommandResult) {
        switch result.sourceSync {
        case 0:
            break
        case 1:
            guard let oldRange = result.oldSourceRange, let newRange = result.newSourceRange else {
                canonicalSource = bridge.source
                break
            }
            let inserted = bridge.copySourceRange(newRange, revision: result.revision)
            let mutable = NSMutableString(string: canonicalSource)
            if oldRange.location >= 0, NSMaxRange(oldRange) <= mutable.length {
                mutable.replaceCharacters(in: oldRange, with: inserted)
                canonicalSource = mutable as String
            } else {
                canonicalSource = bridge.source
            }
        default:
            canonicalSource = bridge.source
        }
        canonicalRevision = result.revision
        _ = result.changed
    }

    private func synchronizeProjection() {
        let active = bridge.composition
        let projected: String
        let selection: NSRange
        if active.active {
            let preedit = bridge.copyComposition(active)
            let mutable = NSMutableString(string: canonicalSource)
            if active.replacementRange.location >= 0,
               NSMaxRange(active.replacementRange) <= mutable.length {
                mutable.replaceCharacters(in: active.replacementRange, with: preedit)
            }
            projected = mutable as String
            nativeMarkedRange = NSRange(
                location: active.replacementRange.location,
                length: preedit.utf16.count
            )
            selection = NSRange(
                location: active.replacementRange.location + active.selection.location,
                length: active.selection.length
            )
        } else {
            projected = canonicalSource
            nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
            let rustSelection = bridge.selection
            selection = rustSelection.range
        }
        synchronizingSelection = true
        string = projected
        selectedRange = clampedRange(selection, length: (string as NSString).length)
        synchronizingSelection = false
        needsDisplay = true
    }

    private func stringValue(_ value: Any) -> String {
        if let attributed = value as? NSAttributedString { return attributed.string }
        if let string = value as? String { return string }
        return "\(value)"
    }

    private func clampedRange(_ range: NSRange, length: Int) -> NSRange {
        guard range.location != NSNotFound else { return NSRange(location: NSNotFound, length: 0) }
        let location = min(max(range.location, 0), length)
        let maximum = max(0, length - location)
        return NSRange(location: location, length: min(max(range.length, 0), maximum))
    }

    private func accessibilitySourceRange(
        _ range: NSRange,
        snapshot: NativeAccessibilitySnapshot
    ) -> NSRange? {
        guard range.location != NSNotFound,
              range.location >= 0,
              range.length >= 0,
              NSMaxRange(range) <= snapshot.numberOfCharacters else {
            return nil
        }
        let source = (bridge.copySourceIfAvailable ?? canonicalSource) as NSString
        if range.location < source.length,
           source.rangeOfComposedCharacterSequence(at: range.location).location != range.location {
            return nil
        }
        if range.length > 0 {
            let last = NSMaxRange(range) - 1
            if NSMaxRange(source.rangeOfComposedCharacterSequence(at: last)) != NSMaxRange(range) {
                return nil
            }
        }
        return range
    }

    func accessibilityFrameForSemanticRange(_ range: NSRange) -> NSRect {
        guard canonicalRevision == bridge.state.revision,
              !bridge.composition.active,
              range.location >= 0,
              range.length >= 0,
              NSMaxRange(range) <= (string as NSString).length,
              let container = textContainer,
              let layoutManager,
              let window else {
            return .zero
        }
        let glyphRange = layoutManager.glyphRange(
            forCharacterRange: range,
            actualCharacterRange: nil
        )
        guard glyphRange.location != NSNotFound else { return .zero }
        let local = layoutManager
            .boundingRect(forGlyphRange: glyphRange, in: container)
            .offsetBy(dx: textContainerOrigin.x, dy: textContainerOrigin.y)
        return window.convertToScreen(convert(local, to: nil))
    }

    func accessibilityFrameForTableResizeDescriptor(
        _ descriptor: NativeTableResizeAccessibilityDivider
    ) -> NSRect {
        guard descriptor.revision == bridge.state.revision,
              !bridge.composition.active else {
            return .zero
        }
        return tableResizeAccessibilityFrameProvider?(descriptor) ?? .zero
    }

    @discardableResult
    func performTableResizeAccessibilityAction(
        _ descriptor: NativeTableResizeAccessibilityDivider,
        direction: Int
    ) -> Bool {
        guard descriptor.revision == bridge.state.revision,
              descriptor.kind == UInt8(YU_STORAGE_TABLE_RESIZE_COLUMN),
              descriptor.columnCount >= 2,
              descriptor.index < descriptor.columnCount - 1,
              direction == 1 || direction == -1,
              !bridge.composition.active else {
            return false
        }
        let changed = onTableResizeAccessibilityAction?(descriptor, direction) ?? false
        if changed {
            refreshTableResizeAccessibility(postNotification: true)
        }
        return changed
    }

    /// Rebuilds only the visible table splitter children. The provider owns
    /// no AppKit objects; it returns fresh scalar descriptors from the
    /// coordinator, and stale Revision descriptors are discarded before they
    /// become discoverable by VoiceOver.
    func refreshTableResizeAccessibility(postNotification: Bool = false) {
        let descriptors = (tableResizeAccessibilityProvider?() ?? []).filter {
            $0.revision == bridge.state.revision
                && $0.kind == UInt8(YU_STORAGE_TABLE_RESIZE_COLUMN)
                && $0.columnCount >= 2
                && $0.index < $0.columnCount - 1
                && $0.rect.origin.x.isFinite
                && $0.rect.origin.y.isFinite
                && $0.rect.width.isFinite
                && $0.rect.height.isFinite
                && $0.rect.width > 0.0
                && $0.rect.height > 0.0
        }
        guard descriptors != tableResizeAccessibilityDescriptors else {
            if postNotification {
                NSAccessibility.post(element: self, notification: .layoutChanged)
            }
            return
        }
        postDestroyedTableResizeAccessibilityElements()
        tableResizeAccessibilityDescriptors = descriptors
        tableResizeAccessibilityElements = descriptors.map {
            YuAccessibilityTableResizeElement(
                descriptor: $0,
                parent: self,
                owner: self
            )
        }
        if postNotification {
            NSAccessibility.post(element: self, notification: .layoutChanged)
        }
    }

    private func rebuildSemanticAccessibilityTree() {
        postDestroyedSemanticElements()
        let nodes = semanticNodes.filter {
            SemanticAccessibilityKind(rawValue: $0.kind) != .document
        }
        var elementsByIndex: [UInt32: YuAccessibilitySemanticElement] = [:]
        for node in nodes {
            let element = YuAccessibilitySemanticElement(
                node: node,
                bridge: bridge,
                parent: self
            )
            element.frameOwner = self
            elementsByIndex[node.index] = element
        }

        var topLevel: [YuAccessibilitySemanticElement] = []
        for node in nodes {
            guard let element = elementsByIndex[node.index] else { continue }
            guard node.parent != UInt32.max,
                  node.parent != 0,
                  let parent = elementsByIndex[node.parent] else {
                element.parentObject = self
                topLevel.append(element)
                continue
            }
            element.parentObject = parent
            parent.semanticChildren.append(element)
        }
        semanticElements = topLevel
        rebuildTableResizeAccessibilityTree()
    }

    private func postDestroyedSemanticElements() {
        for element in flattenSemanticElements(semanticElements) {
            NSAccessibility.post(element: element, notification: .uiElementDestroyed)
        }
        postDestroyedTableResizeAccessibilityElements()
    }

    private func rebuildTableResizeAccessibilityTree() {
        let descriptors = (tableResizeAccessibilityProvider?() ?? []).filter {
            $0.revision == bridge.state.revision
                && $0.kind == UInt8(YU_STORAGE_TABLE_RESIZE_COLUMN)
                && $0.columnCount >= 2
                && $0.index < $0.columnCount - 1
                && $0.rect.origin.x.isFinite
                && $0.rect.origin.y.isFinite
                && $0.rect.width.isFinite
                && $0.rect.height.isFinite
                && $0.rect.width > 0.0
                && $0.rect.height > 0.0
        }
        tableResizeAccessibilityDescriptors = descriptors
        tableResizeAccessibilityElements = descriptors.map {
            YuAccessibilityTableResizeElement(
                descriptor: $0,
                parent: self,
                owner: self
            )
        }
    }

    private func postDestroyedTableResizeAccessibilityElements() {
        for element in tableResizeAccessibilityElements {
            NSAccessibility.post(element: element, notification: .uiElementDestroyed)
        }
    }

    private func flattenSemanticElements(
        _ elements: [YuAccessibilitySemanticElement]
    ) -> [YuAccessibilitySemanticElement] {
        var result: [YuAccessibilitySemanticElement] = []
        result.reserveCapacity(elements.count)
        for element in elements {
            result.append(element)
            let children = element.semanticChildren.compactMap {
                $0 as? YuAccessibilitySemanticElement
            }
            result.append(contentsOf: flattenSemanticElements(children))
        }
        return result
    }

    func postAccessibilityRefresh() {
        semanticNodes = bridge.accessibilitySemanticNodesIfAvailable ?? []
        rebuildSemanticAccessibilityTree()
        NSAccessibility.post(element: self, notification: .valueChanged)
        NSAccessibility.post(element: self, notification: .layoutChanged)
        postSelectionChanged()
    }

    private func postSelectionChanged() {
        NSAccessibility.post(element: self, notification: .selectedTextChanged)
        needsDisplay = true
        onCaretChange?()
    }
}
