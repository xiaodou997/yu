import AppKit
import Darwin
import Foundation
import UniformTypeIdentifiers
import QuartzCore
import YuStorageFFI

// `NSTextInputClient` 与 Accessibility 宿主。它不绘制任何像素——
// Rust surface 是唯一渲染路径（不变量 I5）——只负责把原生输入事件
// 转成 Rust command，并把 OS 的几何查询转给 Rust layout。

/// Native input host. Rust owns source, selection, transactions and composition.
/// The UTF-16 string is an input-context cache only; it has no TextKit storage,
/// layout manager or text container. All visible geometry comes from Rust's
/// CoreText paragraph layouts, also consumed by the Metal surface.
final class DocumentTextView: NSView, NSTextInputClient, NSMenuItemValidation {
    var font: NSFont? = NSFont(name: "Open Sans", size: CGFloat(NativeTheme.spec().body_size))
    var isEditable = true
    var isSelectable = true
    var contentInsets = NSSize(width: 30, height: 30)
    var contentOrigin: NSPoint { NSPoint(x: contentInsets.width, y: contentInsets.height) }
    private(set) var string = ""
    private var nativeSelection = NSRange(location: 0, length: 0)
    private var discardingComposition = false
    private(set) var selectedRanges: [NSValue] = []
    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { true }
    override var undoManager: UndoManager? { nil }
    func selectedRange() -> NSRange { nativeSelection }
    private static let traceNativeEvents = ProcessInfo.processInfo.environment["YU_NATIVE_INPUT_TRACE"] == "1"
        && Bundle.main.bundleIdentifier?.hasPrefix("io.github.xiaodou997.yu.editing-check.") == true

    private func traceNativeEvent(_ kind: String, _ fields: [String: Any] = [:]) {
        guard Self.traceNativeEvents else { return }
        var record = fields
        record["event"] = kind
        record["input_source"] = String(describing: inputContext?.selectedKeyboardInputSource)
        record["context_active"] = NSTextInputContext.current === inputContext
        record["time"] = CACurrentMediaTime()
        if let data = try? JSONSerialization.data(withJSONObject: record, options: [.sortedKeys]),
           let value = String(data: data, encoding: .utf8) {
            print("Yu native input: \(value)")
            fflush(stdout)
        }
    }

    override func keyDown(with event: NSEvent) {
        traceNativeEvent("keyDown", ["key_code": event.keyCode, "characters": event.characters ?? "", "flags": event.modifierFlags.rawValue])
        // Handle cancellation before an input source consumes Escape and leaves
        // an empty/unmarked Rust overlay that would disable the undo menu.
        if event.keyCode == 53, bridge.composition.active {
            cancelInputComposition()
            return
        }
        let handled = inputContext?.handleEvent(event) == true
        traceNativeEvent("inputContext", ["key_code": event.keyCode, "handled": handled])
        if handled { return }
        interpretKeyEvents([event])
    }
    func characterIndex(for point: NSPoint) -> Int {
        guard let window else { return NSNotFound }
        let local = convert(window.convertPoint(fromScreen: point), from: nil)
        let content = NSPoint(x: local.x - contentOrigin.x, y: local.y - contentOrigin.y)
        guard let visual = shapedVisualHit(at: content),
              let source = try? bridge.projectionSourceSelection(revision: bridge.revision, visualRange: NSRange(location: visual.offset, length: 0), affinity: visual.affinity) else { return NSNotFound }
        return source.sourceRange.location
    }
    private enum Command {
        static let deleteBackward: UInt8 = 1
        static let deleteForward: UInt8 = 2
        static let deleteSelections: UInt8 = 27
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
    private var pointerSourceAnchor: (source: Int, revision: UInt64)?
    private var pointerUnitAnchor: (range: NSRange, paragraph: Bool, revision: UInt64)?
    private var tableSelectionAnchor: (source: Int, revision: UInt64)?
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
        canonicalRevision = bridge.revision
        super.init(frame: .zero)
        setAccessibilityElement(true)
        setAccessibilityRole(.textArea)
        setAccessibilityLabel("Yu Markdown 文档")
        setAccessibilityIdentifier("yu-document-text")
        headingRotorDelegate = YuAccessibilityRotorDelegate(owner: self, kind: .heading)
        linkRotorDelegate = YuAccessibilityRotorDelegate(owner: self, kind: .link)
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
        canonicalRevision = bridge.revision
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

    /// 真实窗口 self-check 用：⌥ 点那条路的**坐标→源码**那一步。
    /// headless 里没有已发布的 viewport 几何，这一步只有真实窗口能被证伪。
    @discardableResult
    func addCaretAtVisualPointForSelfCheck(_ point: NSPoint) -> Bool {
        addCaretAtVisualPoint(point)
    }

    /// 真实窗口 self-check 用：某个源码偏移的 caret 矩形（document-space），
    /// 拿来推一个可点的坐标。`shapedVisualHit(at:)` 收的就是这个空间。
    func shapedCaretRectForSelfCheck(sourceUTF16: Int) -> NSRect? {
        rustCaretRect(forSourceUTF16: sourceUTF16)
    }


    private func currentCompositionGeneration() -> UInt64? {
        guard bridge.composition.active else { return nil }
        return try? bridge.compositionProjection(revision: bridge.revision).generation
    }

    private func visualPoint(for event: NSEvent) -> NSPoint {
        let local = convert(event.locationInWindow, from: nil)
        return NSPoint(
            x: local.x - contentOrigin.x,
            y: local.y - contentOrigin.y
        )
    }

    private func setTableResizeCursor(active: Bool) {
        let next = active && window != nil
        guard tableResizeCursorActive != next else { return }
        tableResizeCursorActive = next
        (next ? NSCursor.resizeLeftRight : NSCursor.arrow).set()
    }

    /// Resolves a visual document point through the Rust CoreText-shaped
    /// snapshot. The native input/IME/accessibility host applies coordinate
    /// transforms without guessing glyph boundaries.
    /// 命中测试完全由 Rust layout 完成。此处不再用 TextKit 布局出的
    /// visual 长度做上界校验——那等于用第二套布局系统验证第一套，
    /// 而第二套布局系统本身就是要消除的对象（不变量 I5、E1）。
    /// Rust 返回的 visualUTF16 已绑定同一 Revision，越界由 Rust 侧拒绝。
    private func shapedVisualHit(at point: NSPoint) -> (offset: Int, source: Int, affinity: UInt8)? {
        guard point.x.isFinite,
              point.y.isFinite,
              let (size, width) = visualLayoutMetrics(),
              let hit = try? bridge.projectionHitTest(
                  revision: bridge.revision,
                  point: CGPoint(x: point.x, y: point.y),
                  size: size,
                  maxWidth: width
              ),
              hit.revision == bridge.revision,
              hit.point.x.isFinite,
              hit.point.y.isFinite,
              let sourceOffset = Int(exactly: hit.sourceUTF16),
              let visualOffset = Int(exactly: hit.visualUTF16),
              visualOffset >= 0 else {
            return nil
        }
        return (visualOffset, sourceOffset, hit.affinity)
    }

    @discardableResult
    private func applyVisualPointerSelection(
        at point: NSPoint,
        extending: Bool
    ) -> Bool {
        guard !bridge.composition.active, let hit = shapedVisualHit(at: point) else { return false }
        if !extending || visualSelectionAnchor == nil {
            let source = extending ? Int(bridge.selectionEndpoints.anchorUTF16) : hit.source
            pointerSourceAnchor = (source, bridge.revision)
            visualSelectionAnchor = hit.offset
        }
        guard let anchor = pointerSourceAnchor, anchor.revision == bridge.revision else { return false }
        do {
            try bridge.setSelectionEndpoints(anchorUTF16: UInt64(anchor.source), focusUTF16: UInt64(hit.source), affinity: hit.affinity)
            synchronizeProjection()
            postSelectionChanged()
            return true
        } catch { return false }
    }

    /// 把一个视口坐标点换成源码偏移，再加一根光标。
    ///
    /// 走的是与拖选同一条视觉→源码的路（`projectionSourceSelection`），不是
    /// AppKit 的 `characterIndexForInsertion`——后者认的是 TextKit 那份可丢弃
    /// 的镜像，而语法标记被藏起来之后两者的偏移不是同一个（不变量 I6）。
    private func addCaretAtVisualPoint(_ point: NSPoint) -> Bool {
        guard !bridge.composition.active else { return false }
        guard let hit = shapedVisualHit(at: point) else { return false }
        return addCaret(atSource: hit.source, affinity: hit.affinity)
    }

    private func visualUTF16ForSource(
        _ sourceUTF16: UInt64,
        affinity: UInt8
    ) -> Int? {
        let revision = bridge.revision
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
        anchorIsVisualStart: Bool? = nil,
        affinity: UInt8 = 1
    ) -> Bool {
        guard !bridge.composition.active,
              visualRange.location >= 0,
              visualRange.length >= 0 else {
            return false
        }
        do {
            let source = try bridge.projectionSourceSelection(
                revision: bridge.revision,
                visualRange: visualRange,
                affinity: affinity
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
            canonicalRevision = bridge.revision
            synchronizingSelection = true
            nativeSelection = source.sourceRange
            synchronizingSelection = false
            postSelectionChanged()
            return true
        } catch {
            synchronizingSelection = false
            // A stale point must not mutate the current source selection.
            return false
        }
    }

    // Accessibility reads revision-bound source and geometry from the same session.
    override func accessibilityValue() -> Any? {
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

    /// **这以前是一条从单数推出来的假复数。**
    ///
    /// `AXSelectedTextRange`（单数）按定义是一个区间，多光标之后它给的是
    /// primary——那不是降级，那是另一个属性。复数是这一个，留着从单数推出来的
    /// 一条就是对 VoiceOver 撒谎：屏幕上有五根光标，读屏只知道一根，而且不报错。
    override func accessibilitySelectedTextRanges() -> [NSValue]? {
        guard let selections = bridge.selectionsIfAvailable else {
            return [NSValue(range: accessibilitySelectedTextRange())]
        }
        return selections.ranges.map { NSValue(range: $0.range) }
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
              node.revision == bridge.revision,
              let block = node.actionBlock else {
            return false
        }
        return toggleTask(block: block, revision: node.revision)
    }

    private func toggleTask(block: UInt64, revision: UInt64) -> Bool {
        guard revision == bridge.revision,
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

    @objc func setSelectedRange(_ charRange: NSRange) {
        let range = clampedRange(charRange, length: (string as NSString).length)
        let shouldSync = !synchronizingSelection
        synchronizingSelection = true
        nativeSelection = range
        synchronizingSelection = false
        guard shouldSync else { return }
        syncNativeSelectionToRust(range)
    }

    /// AppKit uses this plural entry point for mouse clicks, drag selection,
    /// and some TextKit accessibility paths.
    ///
    /// **这里以前只把第一条送给 Rust。** 多光标之后全部送过去——`AXSelectedRanges`
    /// 赋值、以及 AppKit 自己的不连续选区都走这个入口，丢掉其余几条不报错，
    /// 只是那几根光标从此不存在。归一化（排序、合并）归 Rust 一家做。
    @objc func setSelectedRanges(
        _ ranges: [NSValue],
        affinity: NSSelectionAffinity,
        stillSelecting: Bool
    ) {
        let shouldSync = !synchronizingSelection
        synchronizingSelection = true
        selectedRanges = ranges
        synchronizingSelection = false
        guard shouldSync else { return }
        let length = (string as NSString).length
        let clamped = ranges.map { clampedRange($0.rangeValue, length: length) }
        guard !clamped.isEmpty else { return }
        // 这条路是 **AppKit 发起**的选区变化（鼠标、AX 赋值），primary 只能问
        // AppKit 自己认哪一条——拖选时 `selectedRange()` 才是光标真正在的地方。
        // 由 Yu 发起的多光标不走这里，走 `navigate(toSources:primary:)`，那里
        // primary 是显式给的（`NSTextView` 对不连续选区没有「主」的概念）。
        let current = clampedRange(selectedRange(), length: length)
        let primary = clamped.firstIndex(of: current) ?? 0
        syncNativeSelectionsToRust(clamped, primary: primary)
    }

    func makeTableMenu() -> NSMenu {
        let menu = NSMenu(title: "表格")
        let items: [(String, Int)] = [
            ("在上方插入正文行", Int(YU_STORAGE_COMMAND_TABLE_INSERT_ROW_BEFORE)),
            ("在下方插入正文行", Int(YU_STORAGE_COMMAND_TABLE_INSERT_ROW_AFTER)),
            ("删除正文行", Int(YU_STORAGE_COMMAND_TABLE_DELETE_ROW)),
            ("在左侧插入列", Int(YU_STORAGE_COMMAND_TABLE_INSERT_COLUMN_BEFORE)),
            ("在右侧插入列", Int(YU_STORAGE_COMMAND_TABLE_INSERT_COLUMN_AFTER)),
            ("删除列", Int(YU_STORAGE_COMMAND_TABLE_DELETE_COLUMN)),
            ("列左对齐", Int(YU_STORAGE_COMMAND_TABLE_ALIGN_LEFT)),
            ("列居中", Int(YU_STORAGE_COMMAND_TABLE_ALIGN_CENTER)),
            ("列右对齐", Int(YU_STORAGE_COMMAND_TABLE_ALIGN_RIGHT)),
            ("列默认对齐", Int(YU_STORAGE_COMMAND_TABLE_ALIGN_DEFAULT)),
        ]
        for (index, entry) in items.enumerated() {
            if index == 3 || index == 6 { menu.addItem(.separator()) }
            let item = NSMenuItem(title: entry.0, action: #selector(editTableFromMenu(_:)), keyEquivalent: "")
            item.tag = entry.1
            item.toolTip = "按住 ⇧⌥ 拖动，可选择矩形单元格区域。"
            item.target = self
            menu.addItem(item)
        }
        return menu
    }

    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        guard menuItem.action == #selector(editTableFromMenu(_:)),
              let command = UInt8(exactly: menuItem.tag) else { return false }
        return isEditable && bridge.commandAvailable(command)
    }

    @objc func editTableFromMenu(_ sender: NSMenuItem) {
        guard isEditable, let command = UInt8(exactly: sender.tag) else { return }
        _ = routeCommand(command)
    }

    override func menu(for event: NSEvent) -> NSMenu? {
        window?.makeFirstResponder(self)
        guard applyVisualPointerSelection(at: visualPoint(for: event), extending: false) else {
            return super.menu(for: event)
        }
        let menu = makeTableMenu()
        return menu.items.contains(where: { validateMenuItem($0) }) ? menu : super.menu(for: event)
    }

    /// Option-Shift drag selects whole cells. Source coordinates come from the
    /// same retained CoreText geometry used by ordinary pointer selection.
    @discardableResult
    func selectTableCellsAtVisualPoint(_ point: NSPoint, starting: Bool) -> Bool {
        guard !bridge.composition.active, let hit = shapedVisualHit(at: point) else { return false }
        // Keep the hit's source identity: empty cells may share one visual
        // offset, so a second visual-to-source conversion loses the cell.
        let anchor = starting ? (source: hit.source, revision: bridge.revision) : tableSelectionAnchor
        guard let anchor, anchor.revision == bridge.revision else { return false }
        do {
            try bridge.selectTableCells(anchor: anchor.source, focus: hit.source, revision: anchor.revision)
            tableSelectionAnchor = anchor
            visualSelectionAnchor = nil
            synchronizeProjection()
            postSelectionChanged()
            return true
        } catch { return false }
    }

    /// AppKit defines native word boundaries; Rust still validates and owns
    /// the resulting source selection. Only the clicked source paragraph is
    /// materialized; this is neither a text-layout mirror nor Markdown parsing.
    private func pointerUnit(at sourceOffset: Int, paragraph: Bool) -> NSRange? {
        let source = canonicalSource as NSString
        guard source.length > 0, sourceOffset >= 0, sourceOffset <= source.length else { return nil }
        let point = source.rangeOfComposedCharacterSequence(at: min(sourceOffset, source.length - 1)).location
        let context = source.paragraphRange(for: NSRange(location: point, length: 0))
        if paragraph { return context }
        let text = NSAttributedString(string: source.substring(with: context))
        let word = text.doubleClick(at: point - context.location)
        guard word.location != NSNotFound, word.length > 0 else { return nil }
        return NSRange(location: context.location + word.location, length: word.length)
    }

    private func clickedCharacterOffset(near sourceOffset: Int, point: NSPoint) -> Int {
        let source = canonicalSource as NSString
        // Hit testing returns an insertion boundary. A click in the right half
        // of the last glyph of a word must still select that word, not the
        // following space. Compare the neighboring graphemes' Rust caret boxes.
        for index in [sourceOffset - 1, sourceOffset] where index >= 0 && index < source.length {
            let range = source.rangeOfComposedCharacterSequence(at: index)
            guard let start = rustCaretRect(forSourceUTF16: range.location),
                  let end = rustCaretRect(forSourceUTF16: NSMaxRange(range)),
                  abs(start.minY - end.minY) < 1 else { continue }
            let left = min(start.minX, end.minX), right = max(start.minX, end.minX)
            if right > left, point.x >= left, point.x < right,
               point.y >= start.minY, point.y <= max(start.maxY, end.maxY) { return range.location }
        }
        return sourceOffset
    }

    @discardableResult
    private func selectPointerUnit(at point: NSPoint, clickCount: Int? = nil) -> Bool {
        guard !bridge.composition.active, let hit = shapedVisualHit(at: point) else { return false }
        let paragraph = clickCount.map { $0 >= 3 } ?? pointerUnitAnchor?.paragraph ?? false
        let character = clickedCharacterOffset(near: hit.source, point: point)
        guard let unit = pointerUnit(at: character, paragraph: paragraph) else { return false }
        if clickCount != nil { pointerUnitAnchor = (unit, paragraph, bridge.revision) }
        guard let anchor = pointerUnitAnchor, anchor.revision == bridge.revision else { return false }
        do {
            let backward = hit.source < anchor.range.location
            try bridge.setSelectionEndpoints(
                anchorUTF16: UInt64(backward ? NSMaxRange(anchor.range) : anchor.range.location),
                focusUTF16: UInt64(backward ? unit.location : max(NSMaxRange(anchor.range), NSMaxRange(unit))),
                affinity: hit.affinity)
            visualSelectionAnchor = nil
            synchronizeProjection()
            postSelectionChanged()
            return true
        } catch { return false }
    }

    /// Pointer events use the same CoreText paragraph geometry as Metal.
    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        tableSelectionAnchor = nil
        pointerUnitAnchor = nil
        if event.buttonNumber == 0, event.clickCount >= 2,
           !event.modifierFlags.contains(.option),
           selectPointerUnit(at: visualPoint(for: event), clickCount: event.clickCount) { return }
        if event.buttonNumber == 0,
           event.modifierFlags.contains([.option, .shift]),
           selectTableCellsAtVisualPoint(visualPoint(for: event), starting: true) { return }
        if event.buttonNumber == 0,
           event.clickCount == 1,
           !event.modifierFlags.contains(.shift),
           onTaskCheckboxPress?(visualPoint(for: event)) == true {
            visualSelectionAnchor = nil
            taskCheckboxPointerConsumed = true
            return
        }
        taskCheckboxPointerConsumed = false
        if event.buttonNumber == 0 {
            let point = visualPoint(for: event)
            let began = onTableResizeBegin?(point) == true
            if Self.traceNativeEvents {
                let dividers = (tableResizeAccessibilityProvider?() ?? []).map { ["x": $0.rect.origin.x, "y": $0.rect.origin.y, "width": $0.rect.width, "height": $0.rect.height] }
                traceNativeEvent("resizeProbe", ["x": point.x, "y": point.y, "began": began,
                    "layout_width": visualLayoutMetrics()?.1 ?? 0, "dividers": dividers])
            }
            if began {
                visualSelectionAnchor = nil
                setTableResizeCursor(active: true)
                return
            }
        }
        // ⌥ 点加一根光标。**必须挡在 super 前面**：`NSTextView` 自己的
        // ⌥ 拖是矩形选区，放过去会既加不上光标又把选区改掉。
        if event.buttonNumber == 0,
           event.clickCount == 1,
           event.modifierFlags.contains(.option),
           !event.modifierFlags.contains(.shift),
           addCaretAtVisualPoint(visualPoint(for: event)) {
            visualSelectionAnchor = nil
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
        autoscroll(with: event)
        if tableSelectionAnchor != nil {
            _ = selectTableCellsAtVisualPoint(visualPoint(for: event), starting: false)
            return
        }
        if taskCheckboxPointerConsumed {
            return
        }
        if pointerUnitAnchor != nil {
            _ = selectPointerUnit(at: visualPoint(for: event))
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

    func finishTableCellSelection() { tableSelectionAnchor = nil }

    override func mouseUp(with event: NSEvent) {
        if pointerUnitAnchor != nil { pointerUnitAnchor = nil; return }
        if tableSelectionAnchor != nil { finishTableCellSelection(); return }
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
    override func draw(_ rect: NSRect) {}

    private func syncNativeSelectionToRust(_ range: NSRange) {
        guard !bridge.composition.active else { return }
        do {
            try bridge.setSelection(range)
            canonicalRevision = bridge.revision
            postSelectionChanged()
        } catch {
            onError?(error)
        }
    }

    private func syncNativeSelectionsToRust(_ ranges: [NSRange], primary: Int, affinities: [UInt8]? = nil) {
        guard !bridge.composition.active else { return }
        do {
            try bridge.setSelections(ranges, primary: primary, affinities: affinities)
            canonicalRevision = bridge.revision
            postSelectionChanged()
        } catch {
            onError?(error)
        }
    }

    private static let traceInput = ProcessInfo.processInfo.environment["YU_RENDER_TIMING"] != nil

    @objc func insertText(_ insertString: Any, replacementRange: NSRange) {
        traceNativeEvent("insertText", ["text": stringValue(insertString)])
        var phaseStart = Self.traceInput ? CACurrentMediaTime() : 0
        func phase(_ name: String) {
            guard Self.traceInput else { return }
            let now = CACurrentMediaTime()
            print("yu-render-metric event=input_phase phase=\(name) time_s=\(now) duration_ms=\((now - phaseStart) * 1000)")
            phaseStart = now
        }
        guard !discardingComposition else { return }
        let text = stringValue(insertString)
        guard !text.isEmpty || bridge.composition.active else { return }
        do {
            if bridge.composition.active {
                try bridge.commitComposition(text)
                canonicalSource = bridge.source
                canonicalRevision = bridge.revision
            } else {
                let target = replacementRange.location == NSNotFound
                    ? bridge.selection.range
                    : replacementRange
                if target != bridge.selection.range {
                    try bridge.setSelection(target)
                }
                let result = try bridge.insertText(text)
                phase("rust_command")
                apply(result)
                phase("source_cache")
            }
            nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
            synchronizeProjection()
            phase("projection")
            postAccessibilityRefresh()
            phase("accessibility")
            onDocumentChange?()
            phase("document_observers")
        } catch {
            onError?(error)
        }
    }

    @objc func copy(_ sender: Any?) {
        do {
            try finishCompositionForClipboard()
            let revision = bridge.revision
            let text = bridge.copySelection()
            guard !text.isEmpty || bridge.tableSelectionColumns > 0 else { return }
            let html = try bridge.copySelectionHTML(revision: revision)
            try publishSourceToPasteboard(text, html: html)
        } catch {
            onError?(error)
        }
    }

    @objc func cut(_ sender: Any?) {
        do {
            guard try cutToPasteboard(.general) else { return }
            postAccessibilityRefresh()
            onDocumentChange?()
        } catch {
            onError?(error)
        }
    }

    /// Publish every selected source fragment before deleting any of them.
    /// The private-board check uses this same production operation.
    @discardableResult
    private func cutToPasteboard(_ pasteboard: NSPasteboard) throws -> Bool {
        try finishCompositionForClipboard()
        let revision = bridge.revision
        let selected = bridge.copySelection()
        guard !selected.isEmpty || bridge.tableSelectionColumns > 0 else { return false }
        let html = try bridge.copySelectionHTML(revision: revision)
        try publishSourceToPasteboard(selected, html: html, to: pasteboard)
        guard bridge.commandAvailable(Command.deleteSelections) else { return false }
        apply(try bridge.executeCommand(Command.deleteSelections))
        synchronizeProjection()
        return true
    }

    func cutToPasteboardForSelfCheck(_ pasteboard: NSPasteboard) throws {
        try cutToPasteboard(pasteboard)
    }

    @objc func paste(_ sender: Any?) {
        do {
            try finishCompositionForClipboard()
            guard try pasteSourceFromPasteboard(.general) else { return }
            synchronizeProjection()
            postAccessibilityRefresh()
            onDocumentChange?()
        } catch {
            onError?(error)
        }
    }

    var hasSourceSelection: Bool {
        bridge.tableSelectionColumns > 0 || bridge.commandAvailable(Command.deleteSelections)
    }

    var hasSourceOnPasteboard: Bool {
        let pasteboard = NSPasteboard.general
        return pasteboard.data(forType: .yuFragments) != nil
            || pasteboard.string(forType: .yuMarkdown) != nil
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
        let revision = bridge.revision
        let text = bridge.copySelection()
        guard !text.isEmpty || bridge.tableSelectionColumns > 0 else { return }
        let html = try bridge.copySelectionHTML(revision: revision)
        try publishSourceToPasteboard(text, html: html, to: pasteboard)
    }

    /// Runs the production paste transaction against a private pasteboard.
    /// The same Rust selection/insert path is used; AppKit notifications are
    /// intentionally omitted because this is a headless check.
    func pasteFromPasteboardForSelfCheck(_ pasteboard: NSPasteboard) throws {
        try finishCompositionForClipboard()
        guard try pasteSourceFromPasteboard(pasteboard) else { return }
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

    @objc func setMarkedText(
        _ markedText: Any,
        selectedRange: NSRange,
        replacementRange: NSRange
    ) {
        guard !discardingComposition else { return }
        let text = stringValue(markedText)
        traceNativeEvent("setMarkedText", ["text": text])
        do {
            let active = bridge.composition
            // Deleting the last preedit character ends the conversion. Keeping
            // a zero-length Rust overlay would block pointer input and undo
            // even after the system no longer displays marked text.
            if text.isEmpty {
                if active.active { try bridge.cancelComposition() }
                nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
                synchronizeProjection()
                postAccessibilityRefresh()
                onDocumentChange?()
                onCaretChange?()
                return
            }
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
            onCaretChange?()
        } catch {
            onError?(error)
        }
    }

    @objc func unmarkText() {
        // AppKit's unmark is a presentation transition. The Rust overlay must
        // stay alive because some input sources deliver insertText afterwards.
        synchronizeProjection()
        nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
    }

    @objc func hasMarkedText() -> Bool {
        bridge.composition.active && nativeMarkedRange.location != NSNotFound
    }

    @objc func markedRange() -> NSRange {
        return nativeMarkedRange
    }

    @objc func attributedSubstring(
        forProposedRange proposedRange: NSRange,
        actualRange: NSRangePointer?
    ) -> NSAttributedString? {
        let range = clampedRange(proposedRange, length: (string as NSString).length)
        actualRange?.pointee = range
        guard range.location != NSNotFound else { return nil }
        return NSAttributedString(string: (string as NSString).substring(with: range))
    }

    @objc func validAttributesForMarkedText() -> [NSAttributedString.Key] {
        [.font, .foregroundColor, .underlineStyle]
    }

    private func cancelInputComposition() {
        guard bridge.composition.active else { return }
        discardingComposition = true
        defer { discardingComposition = false }
        do {
            try bridge.cancelComposition()
            nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
            inputContext?.discardMarkedText()
            synchronizeProjection()
            postAccessibilityRefresh()
            onDocumentChange?()
            onCaretChange?()
        } catch { onError?(error) }
    }

    override func doCommand(by selector: Selector) {
        let name = NSStringFromSelector(selector)
        if name == "cancel:" || name == "cancelOperation:" {
            if onTableResizeCancel?() == true {
                setTableResizeCursor(active: false)
                return
            }
            cancelInputComposition()
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

        if name == "deleteWordBackward:" || name == "deleteWordForward:" {
            routeCommand(UInt8(name == "deleteWordBackward:"
                ? YU_STORAGE_COMMAND_DELETE_WORD_BACKWARD : YU_STORAGE_COMMAND_DELETE_WORD_FORWARD))
            return
        }
        let motions: [String: UInt8] = [
            "moveLeftAndModifySelection:": 17, "moveRightAndModifySelection:": 18,
            "moveWordLeftAndModifySelection:": 19, "moveWordRightAndModifySelection:": 20,
            "moveToBeginningOfDocument:": 21, "moveToEndOfDocument:": 22,
            "moveToBeginningOfDocumentAndModifySelection:": 23, "moveToEndOfDocumentAndModifySelection:": 24
        ]
        if let command = motions[name] { routeCommand(command); return }
        if name == "insertTab:" || name == "insertBacktab:" {
            let previous = name == "insertBacktab:"
            if routeCommand(previous ? 26 : 25) { return }
            if routeCommand(previous ? Command.outdentList : Command.indentList) { return }
            if !previous { insertText("\t", replacementRange: NSRange(location: NSNotFound, length: 0)) }
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
        // AppKit probes the whole view tree for key equivalents, including
        // this surface while a search field owns the keyboard. Rust history
        // must only receive shortcuts from the active document input host.
        guard window?.firstResponder === self else { return false }
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
            if isVertical {
                guard let (size, width) = visualLayoutMetrics() else { return false }
                result = try bridge.executeShapedVerticalCommand(
                    command, size: size, maxWidth: width
                )
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

    /// IME 插入点来自当前 Rust 布局快照，包含语法显隐与预编辑投影。
    /// 阅读列、视图和屏幕坐标在这里各转换一次；几何不可用时返回空矩形。
    @objc func firstRect(
        forCharacterRange range: NSRange,
        actualRange: NSRangePointer?
    ) -> NSRect {
        guard let caretRect = rustCaretRect(forSourceUTF16: range.location) else {
            actualRange?.pointee = NSRange(location: NSNotFound, length: 0)
            return .zero
        }
        actualRange?.pointee = NSRange(location: range.location, length: 0)
        let viewRect = NSRect(
            x: caretRect.origin.x + contentOrigin.x,
            y: caretRect.origin.y + contentOrigin.y,
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
        let revision = bridge.revision
        let offset = UInt64(sourceUTF16)
        let selection = bridge.selectionEndpoints
        let affinity: UInt8 = selection.focusUTF16 == offset ? selection.affinity : 0
        if bridge.composition.active {
            guard let caret = try? bridge.compositionShapedCaret(
                revision: revision,
                generation: bridge.composition.generation,
                sourceUTF16: offset,
                affinity: affinity,
                size: size,
                maxWidth: width
            ), caret.revision == revision else {
                return nil
            }
            return NSRect(origin: caret.point, size: caret.size)
        }
        guard let caret = try? bridge.sourceCaret(
            revision: revision,
            sourceUTF16: offset,
            affinity: affinity,
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
        let width = max(bounds.width - 2.0 * contentOrigin.x, 1.0)
        guard width.isFinite, width > 0.0 else { return nil }
        return (Float(max(font.pointSize, 1.0)), Float(width))
    }

    /// 跳到一个源码位置。**这是「怎么跳到一个源码位置」的唯一实现。**
    ///
    /// **导航不另开 FFI。** 选区走 `setSelectedRange` 那条已有的路，它落到
    /// `yu_storage_session_set_selection_endpoints`；滚动由随之而来的
    /// `onCaretChange` 交给 `shapedCaretScrollRequest`，也就是
    /// yu-editor::viewport 那条路。**面板不自己算 y**——它手上只有 UTF-16
    /// 偏移，算 y 就要在平台侧复制一份排版。
    ///
    /// 参数是一个 UTF-16 区间而不是某个面板的条目类型：第三刀来了第二个
    /// 面板（搜索结果），另写一份会立刻产生第二个答案，而这一刀恰好又要动
    /// 选区（搜索有自己的「跳到下一个」）——那是最容易分叉的地方。
    /// `length` 为 0 时是把光标放过去，非 0 时是把那一段选中。
    func navigate(toSource range: NSRange) {
        let length = (string as NSString).length
        guard range.location >= 0,
              range.length >= 0,
              range.location + range.length <= length else {
            return
        }
        setSelectedRange(range)
    }

    /// 跳到大纲里的一条标题：把光标放到它正文的起点。
    func navigateToOutlineItem(_ item: NativeOutlineItem) {
        navigate(toSource: NSRange(location: item.labelRange.location, length: 0))
    }

    /// 跳到一组源码位置，把它们全部选中。
    ///
    /// 与单数那个是同一条路（都落到选区入口，滚动都交给 `onCaretChange`），
    /// 只是一次给 N 段。`primary` 决定滚到哪一处、以及「当前命中」算哪一处。
    func navigate(toSources ranges: [NSRange], primary: Int, affinities: [UInt8]? = nil) {
        let length = (string as NSString).length
        guard !ranges.isEmpty, primary >= 0, primary < ranges.count else { return }
        for range in ranges {
            guard range.location >= 0,
                  range.length >= 0,
                  range.location + range.length <= length else {
                return
            }
        }
        // **primary 直接送给 Rust，不经 AppKit 转手。**
        //
        // `NSTextView` 对不连续选区没有「主」的概念：`selectedRange()` 在
        // `setSelectedRanges` 之后给的是它自己挑的那一条，于是 primary 一律
        // 退回第 0 条——按 ⌥ 加的那根光标不会成为主光标，滚动与「当前命中」
        // 跟着错，而且不报错。选区的权威在 Rust（不变量 I6），镜像跟着走。
        syncNativeSelectionsToRust(ranges, primary: primary, affinities: affinities)
        synchronizingSelection = true
        selectedRanges = ranges.map { NSValue(range: $0) }
        nativeSelection = ranges[primary]
        synchronizingSelection = false
    }

    /// ⌥ 点：在已有的光标之外**再加一根**。
    ///
    /// 这是不依赖搜索面板的那个入口，也是唯一能手动造出「重叠、逆序、同一个
    /// 偏移两次」的路——「选中全部匹配」产出的选区必然有序不重叠，压不住合并。
    /// 合并本身仍然归 Rust：这里只是把新的一根接在后面送过去。
    @discardableResult
    func addCaret(atSource offset: Int, affinity: UInt8 = 1) -> Bool {
        guard !bridge.composition.active else { return false }
        guard let existing = bridge.selectionsIfAvailable else { return false }
        let length = (string as NSString).length
        guard offset >= 0, offset <= length else { return false }
        var ranges = existing.ranges.map { $0.range }
        ranges.append(NSRange(location: offset, length: 0))
        navigate(toSources: ranges, primary: ranges.count - 1, affinities: existing.ranges.map { $0.affinity } + [affinity])
        return true
    }

    /// 跳到一处搜索命中：把它**选中**。
    ///
    /// 选中而不是只放光标，是因为「当前命中」由选区推出来（Rust 侧
    /// `SearchState::current` 要求选区恰好等于那一段）——不存第二份下标，
    /// 就不会有第二个可以对不上的答案。
    func navigateToSearchMatch(_ match: NativeSearchMatch) {
        navigate(toSource: match.range)
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

    /// Explicit Save/Close commits the currently visible preedit once. Autosave
    /// never calls this: it must not select or terminate an IME candidate.
    func finishCompositionForFileOperation() throws {
        let composition = bridge.composition
        guard composition.active else { return }
        let text = bridge.copyComposition(composition)
        discardingComposition = true
        defer { discardingComposition = false }
        try bridge.commitComposition(text)
        inputContext?.discardMarkedText()
        canonicalSource = bridge.source
        canonicalRevision = bridge.revision
        nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
        synchronizeProjection()
        postAccessibilityRefresh()
        onDocumentChange?()
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
        let markdown = try bridge.copySelectionMarkdown(revision: bridge.revision)
        let fragments = try bridge.copySelectionFragments(revision: bridge.revision)
        let payload = try JSONEncoder().encode(SourceFragments(version: 1, fragments: fragments, columns: bridge.tableSelectionColumns))
        pasteboard.clearContents()
        if bridge.tableSelectionColumns > 0 {
            guard pasteboard.setString(source, forType: .tabularText) else { throw BridgeError.clipboard }
        }
        guard pasteboard.setData(payload, forType: .yuFragments),
              pasteboard.setString(source, forType: .string),
              pasteboard.setString(markdown, forType: .yuMarkdown),
              pasteboard.setString(html, forType: .yuHTML) else {
            throw BridgeError.clipboard
        }
    }

    private struct SourceFragments: Codable {
        let version: Int
        let fragments: [String]
        let columns: Int?
    }

    /// Preserve clipboard representation. Rust owns grid recognition, TSV
    /// decoding and the source edit; Swift never splits cells or parses Markdown.
    private func pasteSourceFromPasteboard(_ pasteboard: NSPasteboard) throws -> Bool {
        if let data = pasteboard.data(forType: .yuFragments),
           let payload = try? JSONDecoder().decode(SourceFragments.self, from: data),
           payload.version == 1, !payload.fragments.isEmpty {
            let columns = payload.columns ?? 0
            guard columns >= 0 else { throw BridgeError.operation(24) }
            apply(try bridge.pasteFragments(payload.fragments, columns: columns))
            return true
        }
        if let markdown = pasteboard.string(forType: .yuMarkdown) {
            apply(try bridge.pasteFragments([markdown]))
            return true
        }
        if let html = pasteboard.string(forType: .yuHTML), let imported = try bridge.importHTML(html) {
            apply(try bridge.pasteFragments([imported]))
            return true
        }
        if let tabular = pasteboard.string(forType: .tabularText) {
            apply(try bridge.pasteClipboardText(tabular, tabular: true))
            return true
        }
        guard let text = pasteboard.string(forType: .string), !text.isEmpty else { return false }
        apply(try bridge.pasteClipboardText(text, tabular: false))
        return true
    }

    /// 粘贴时按「信息量从多到少」取剪贴板上的一种表示。
    ///
    /// 顺序是 **canonical Markdown > HTML > 纯文本**。
    ///
    /// **HTML 必须排在纯文本前面**，这一条 S7 第七刀 c 的 G 节验收之前反着：
    /// 原来是 Markdown > 纯文本 > HTML。而**任何真实的剪贴板都带纯文本**
    /// ——浏览器、邮件、文档编辑器无一例外——于是 `importHTML` 这条路在生产里
    /// **一次都没有被走到过**。整个 HTML 导入策略（连同它的白名单、它的用例、
    /// 它的 fixture）都在为一条不可达的分支服务，而没有任何断言看得出来：
    /// self-check 的每一条都是自己往私有剪贴板上摆格式，摆的组合恰好都没有
    /// 「纯文本与可用 HTML 同时在」这一种。
    ///
    /// 纯文本按定义是同一份内容**丢掉结构之后**的样子。两者都在时取纯文本，
    /// 等于每次都主动选那份更少的。
    ///
    /// 回退没有变，只是现在真的会走到：`importHTML` 在策略拒绝时返回 `nil`
    /// （不是抛错），于是接不住的 HTML 仍然落回纯文本。
    private func sourceFromPasteboard(_ pasteboard: NSPasteboard = .general) throws -> String? {
        if let markdown = pasteboard.string(forType: .yuMarkdown) {
            return markdown
        }
        if let html = pasteboard.string(forType: .yuHTML),
            let imported = try bridge.importHTML(html)
        {
            return imported
        }
        return pasteboard.string(forType: .string)
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
        nativeSelection = clampedRange(selection, length: (string as NSString).length)
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
        guard let window, range.location != NSNotFound,
              let first = rustCaretRect(forSourceUTF16: range.location),
              let last = rustCaretRect(forSourceUTF16: NSMaxRange(range)) else { return .zero }
        let local = first.union(last).offsetBy(dx: contentOrigin.x, dy: contentOrigin.y)
        return window.convertToScreen(convert(local, to: nil))
    }

    func accessibilityFrameForTableResizeDescriptor(
        _ descriptor: NativeTableResizeAccessibilityDivider
    ) -> NSRect {
        guard descriptor.revision == bridge.revision,
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
        guard descriptor.revision == bridge.revision,
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
            $0.revision == bridge.revision
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
            $0.revision == bridge.revision
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
