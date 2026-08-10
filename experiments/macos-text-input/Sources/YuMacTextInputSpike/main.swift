import AppKit
import ApplicationServices
import YuEditorFFI

private let notFoundRange = NSRange(location: NSNotFound, length: 0)

private final class RustCompositionBridge {
    private var session: OpaquePointer?
    private(set) var hasOverlay = false

    init(source: String) {
        var created: OpaquePointer?
        let bytes = Array(source.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            yu_composition_session_new(buffer.baseAddress, buffer.count, &created)
        }
        precondition(status == 0 && created != nil, "Rust composition session creation failed")
        session = created
    }

    deinit {
        if let session {
            yu_composition_session_destroy(session)
        }
    }

    func resetSource(_ source: String) {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        let bytes = Array(source.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            yu_composition_session_reset_source(session, buffer.baseAddress, buffer.count)
        }
        precondition(status == 0, "Rust composition source reset failed: \(status)")
        hasOverlay = false
    }

    func begin(replacement: NSRange, preedit: String, selection: NSRange) {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        let bytes = Array(preedit.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            yu_composition_session_begin(
                session,
                UInt64(replacement.location),
                UInt64(NSMaxRange(replacement)),
                buffer.baseAddress,
                buffer.count,
                UInt64(selection.location),
                UInt64(NSMaxRange(selection))
            )
        }
        precondition(status == 0, "Rust composition begin failed: \(status)")
        hasOverlay = true
    }

    func update(preedit: String, selection: NSRange) {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        precondition(hasOverlay, "Rust composition update requires an active overlay")
        let bytes = Array(preedit.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            yu_composition_session_update(
                session,
                buffer.baseAddress,
                buffer.count,
                UInt64(selection.location),
                UInt64(NSMaxRange(selection))
            )
        }
        precondition(status == 0, "Rust composition update failed: \(status)")
    }

    func commit(_ text: String) {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        precondition(hasOverlay, "Rust composition commit requires an active overlay")
        let bytes = Array(text.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            yu_composition_session_commit(session, buffer.baseAddress, buffer.count)
        }
        precondition(status == 0, "Rust composition commit failed: \(status)")
        hasOverlay = false
    }

    func cancel() {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        guard hasOverlay else { return }
        let status = yu_composition_session_cancel(session)
        precondition(status == 0, "Rust composition cancel failed: \(status)")
        hasOverlay = false
    }

    func sourceString() -> String {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        var length = 0
        precondition(
            yu_composition_session_source_length(session, &length) == 0,
            "Rust composition source length failed"
        )
        var bytes = [UInt8](repeating: 0, count: length)
        let status = bytes.withUnsafeMutableBufferPointer { buffer in
            yu_composition_session_copy_source(session, buffer.baseAddress, buffer.count)
        }
        precondition(status == 0, "Rust composition source copy failed: \(status)")
        return String(decoding: bytes, as: UTF8.self)
    }

    func overlayString() -> String {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        precondition(hasOverlay, "Rust composition overlay is not active")
        var length = 0
        precondition(
            yu_composition_session_overlay_length(session, &length) == 0,
            "Rust composition overlay length failed"
        )
        var bytes = [UInt8](repeating: 0, count: length)
        let status = bytes.withUnsafeMutableBufferPointer { buffer in
            yu_composition_session_copy_overlay(session, buffer.baseAddress, buffer.count)
        }
        precondition(status == 0, "Rust composition overlay copy failed: \(status)")
        return String(decoding: bytes, as: UTF8.self)
    }

    func overlaySelection() -> NSRange {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        precondition(hasOverlay, "Rust composition overlay is not active")
        var start: UInt64 = 0
        var end: UInt64 = 0
        let status = yu_composition_session_overlay_selection(session, &start, &end)
        precondition(status == 0, "Rust composition selection query failed: \(status)")
        return NSRange(location: Int(start), length: Int(end - start))
    }
}

final class TextInputView: NSView, NSTextInputClient {
    private let textStorage = NSTextStorage()
    private let layoutManager = NSLayoutManager()
    private let textContainer = NSTextContainer()
    private var selection = NSRange(location: 0, length: 0)
    private var selectionAffinity: NSSelectionAffinity = .downstream
    private var marked = notFoundRange
    private var compositionOriginal: NSAttributedString?
    private var compositionSelectionBefore: NSRange?
    private var compositionAffinityBefore: NSSelectionAffinity?
    private var rustComposition: RustCompositionBridge!

    private let textOrigin = NSPoint(x: 24, y: 24)
    private let maximumTextWidth: CGFloat = 360
    private let defaultAttributes: [NSAttributedString.Key: Any] = [
        .font: NSFont.systemFont(ofSize: 22),
        .foregroundColor: NSColor.labelColor,
    ]

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        textContainer.lineFragmentPadding = 0
        layoutManager.addTextContainer(textContainer)
        textStorage.addLayoutManager(layoutManager)
        replaceStorage(
            range: NSRange(location: 0, length: 0),
            with: NSAttributedString(
                string: "Yu macOS IME spike\n\n请点击这里并输入中文、日文、emoji 或组合字符。\n",
                attributes: defaultAttributes
            )
        )
        selection = NSRange(location: textStorage.length, length: 0)
        rustComposition = RustCompositionBridge(source: textStorage.string)
        setAccessibilityElement(true)
        setAccessibilityRole(.textArea)
        setAccessibilityLabel("Yu Editor document")
        setAccessibilityIdentifier("yu-editor-document")
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    override var acceptsFirstResponder: Bool { true }
    override var isFlipped: Bool { true }

    override func updateLayer() {
        layer?.backgroundColor = NSColor.textBackgroundColor.cgColor
    }

    override func layout() {
        super.layout()
        updateContainerSize()
    }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        updateContainerSize()
        layoutManager.ensureLayout(for: textContainer)

        let glyphRange = layoutManager.glyphRange(for: textContainer)
        layoutManager.drawBackground(forGlyphRange: glyphRange, at: textOrigin)
        layoutManager.drawGlyphs(forGlyphRange: glyphRange, at: textOrigin)

        if window?.firstResponder === self {
            let caret = caretRect(at: selection.location, affinity: selectionAffinity)
            NSColor.controlAccentColor.setFill()
            caret.fill()
        }
    }

    override func keyDown(with event: NSEvent) {
        if inputContext?.handleEvent(event) != true {
            super.keyDown(with: event)
        }
    }

    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        let point = convert(event.locationInWindow, from: nil)
        let hit = caretHit(forLocalPoint: point)
        selection = NSRange(location: hit.index, length: 0)
        selectionAffinity = hit.affinity
        marked = notFoundRange
        inputContext?.discardMarkedText()
        needsDisplay = true
        postSelectionChanged()
    }

    func insertText(_ value: Any, replacementRange: NSRange) {
        let inserted = attributedString(from: value, marked: false)
        let target = targetRange(replacementRange)
        print("insertText commit=\(inserted.string.debugDescription) replace=\(target)")
        if !rustComposition.hasOverlay {
            rustComposition.begin(
                replacement: target,
                preedit: "",
                selection: NSRange(location: 0, length: 0)
            )
        }
        rustComposition.commit(inserted.string)
        replaceStorage(range: target, with: inserted)
        selection = NSRange(location: target.location + inserted.length, length: 0)
        selectionAffinity = .downstream
        marked = notFoundRange
        compositionOriginal = nil
        compositionSelectionBefore = nil
        compositionAffinityBefore = nil
        needsDisplay = true
        postTextChanged()
    }

    func setMarkedText(
        _ value: Any,
        selectedRange newSelection: NSRange,
        replacementRange: NSRange
    ) {
        let inserted = attributedString(from: value, marked: true)
        let target = targetRange(replacementRange)
        if !hasMarkedText() {
            compositionOriginal = textStorage.attributedSubstring(from: target)
            compositionSelectionBefore = selection
            compositionAffinityBefore = selectionAffinity
        }
        if rustComposition.hasOverlay {
            rustComposition.update(preedit: inserted.string, selection: newSelection)
        } else {
            rustComposition.begin(
                replacement: target,
                preedit: inserted.string,
                selection: newSelection
            )
        }
        print(
            "setMarkedText preedit=\(inserted.string.debugDescription) "
                + "selection=\(newSelection) replace=\(target)"
        )
        replaceStorage(range: target, with: inserted)
        marked = NSRange(location: target.location, length: inserted.length)

        let relativeLocation = min(newSelection.location, inserted.length)
        let maximumLength = inserted.length - relativeLocation
        selection = NSRange(
            location: target.location + relativeLocation,
            length: min(newSelection.length, maximumLength)
        )
        selectionAffinity = .downstream
        needsDisplay = true
        postTextChanged()
    }

    func unmarkText() {
        print("unmarkText range=\(marked)")
        if hasMarkedText() {
            textStorage.removeAttribute(.underlineStyle, range: marked)
        }
        marked = notFoundRange
        compositionOriginal = nil
        compositionSelectionBefore = nil
        compositionAffinityBefore = nil
        needsDisplay = true
        postSelectionChanged()
    }

    func selectedRange() -> NSRange {
        selection
    }

    func markedRange() -> NSRange {
        marked
    }

    func hasMarkedText() -> Bool {
        marked.location != NSNotFound && marked.length > 0
    }

    func attributedSubstring(
        forProposedRange proposedRange: NSRange,
        actualRange: NSRangePointer?
    ) -> NSAttributedString? {
        let range = clamped(proposedRange)
        actualRange?.pointee = range
        guard range.location != NSNotFound else { return nil }
        return textStorage.attributedSubstring(from: range)
    }

    func validAttributesForMarkedText() -> [NSAttributedString.Key] {
        [.font, .foregroundColor, .underlineStyle]
    }

    func firstRect(
        forCharacterRange characterRange: NSRange,
        actualRange: NSRangePointer?
    ) -> NSRect {
        let range = clamped(characterRange)
        actualRange?.pointee = range
        let localRect: NSRect
        if range.length == 0 {
            let affinity = range == selection ? selectionAffinity : .downstream
            localRect = caretRect(at: range.location, affinity: affinity)
        } else {
            let glyphRange = layoutManager.glyphRange(
                forCharacterRange: range,
                actualCharacterRange: nil
            )
            let bounds = layoutManager.boundingRect(forGlyphRange: glyphRange, in: textContainer)
            localRect = bounds.offsetBy(dx: textOrigin.x, dy: textOrigin.y)
        }

        let windowRect = convert(localRect, to: nil)
        return window?.convertToScreen(windowRect) ?? windowRect
    }

    func characterIndex(for point: NSPoint) -> Int {
        let windowPoint = window?.convertPoint(fromScreen: point) ?? point
        return caretHit(forLocalPoint: convert(windowPoint, from: nil)).index
    }

    func fractionOfDistanceThroughGlyph(for point: NSPoint) -> CGFloat {
        let windowPoint = window?.convertPoint(fromScreen: point) ?? point
        let local = convert(windowPoint, from: nil)
        let containerPoint = NSPoint(x: local.x - textOrigin.x, y: local.y - textOrigin.y)
        updateContainerSize()
        return layoutManager.fractionOfDistanceThroughGlyph(
            for: containerPoint,
            in: textContainer
        )
    }

    override func accessibilityValue() -> Any? {
        textStorage.string
    }

    override func accessibilityNumberOfCharacters() -> Int {
        textStorage.length
    }

    override func accessibilitySelectedText() -> String? {
        let range = validatedAccessibilityRange(selection) ?? NSRange(location: 0, length: 0)
        return (textStorage.string as NSString).substring(with: range)
    }

    override func accessibilitySelectedTextRange() -> NSRange {
        selection
    }

    override func setAccessibilitySelectedTextRange(_ range: NSRange) {
        guard let range = validatedAccessibilityRange(range) else { return }
        inputContext?.discardMarkedText()
        marked = notFoundRange
        selection = range
        selectionAffinity = .downstream
        needsDisplay = true
        postSelectionChanged()
    }

    override func accessibilitySelectedTextRanges() -> [NSValue]? {
        [NSValue(range: selection)]
    }

    override func accessibilityVisibleCharacterRange() -> NSRange {
        updateContainerSize()
        layoutManager.ensureLayout(for: textContainer)
        guard layoutManager.numberOfGlyphs > 0 else {
            return NSRange(location: 0, length: 0)
        }
        let visible = NSRect(
            x: bounds.minX - textOrigin.x,
            y: bounds.minY - textOrigin.y,
            width: bounds.width,
            height: bounds.height
        )
        let glyphs = layoutManager.glyphRange(forBoundingRect: visible, in: textContainer)
        return layoutManager.characterRange(forGlyphRange: glyphs, actualGlyphRange: nil)
    }

    override func accessibilityInsertionPointLineNumber() -> Int {
        logicalLine(containing: selection.location)
    }

    override func accessibilityString(for range: NSRange) -> String? {
        guard let range = validatedAccessibilityRange(range) else { return nil }
        return (textStorage.string as NSString).substring(with: range)
    }

    override func accessibilityAttributedString(for range: NSRange) -> NSAttributedString? {
        guard let range = validatedAccessibilityRange(range) else { return nil }
        return textStorage.attributedSubstring(from: range)
    }

    override func accessibilityRange(forLine line: Int) -> NSRange {
        logicalLineRange(line)
    }

    override func accessibilityLine(for index: Int) -> Int {
        guard index >= 0, index <= textStorage.length else { return NSNotFound }
        return logicalLine(containing: index)
    }

    override func accessibilityRange(for index: Int) -> NSRange {
        guard index >= 0, index <= textStorage.length else { return notFoundRange }
        guard index < textStorage.length else {
            return NSRange(location: textStorage.length, length: 0)
        }
        return (textStorage.string as NSString).rangeOfComposedCharacterSequence(at: index)
    }

    override func accessibilityRange(for position: NSPoint) -> NSRange {
        accessibilityRange(for: characterIndex(for: position))
    }

    override func accessibilityFrame(for range: NSRange) -> NSRect {
        guard let range = validatedAccessibilityRange(range) else { return .zero }
        return firstRect(forCharacterRange: range, actualRange: nil)
    }

    override func accessibilityStyleRange(for index: Int) -> NSRange {
        guard index >= 0, index < textStorage.length else { return notFoundRange }
        var effective = NSRange(location: 0, length: 0)
        _ = textStorage.attributes(at: index, effectiveRange: &effective)
        return effective
    }

    func runAccessibilitySelfCheck() {
        let full = NSRange(location: 0, length: accessibilityNumberOfCharacters())
        let firstLine = accessibilityRange(forLine: 0)
        let firstText = accessibilityString(for: firstLine) ?? ""
        let caretFrame = accessibilityFrame(for: accessibilitySelectedTextRange())
        precondition(accessibilityString(for: full) == textStorage.string)
        precondition(firstLine.location == 0 && firstLine.location != NSNotFound)
        precondition(!firstText.isEmpty)
        precondition(!caretFrame.isEmpty)
        print(
            "AX self-check characters=\(accessibilityNumberOfCharacters()) "
                + "selection=\(accessibilitySelectedTextRange()) "
                + "firstLine=\(firstLine) caretFrame=\(caretFrame)"
        )
    }

    func runLayoutRoundTripSelfCheck() {
        updateContainerSize()
        layoutManager.ensureLayout(for: textContainer)
        let glyphRange = layoutManager.glyphRange(for: textContainer)
        var lineFragments = 0
        layoutManager.enumerateLineFragments(forGlyphRange: glyphRange) {
            _, _, _, _, _ in
            lineFragments += 1
        }

        let boundaries = canonicalCaretOffsets()
        var affinitySplits = 0
        var softWrapSplits = 0
        for index in boundaries {
            let downstream = caretRect(at: index, affinity: .downstream)
            let downstreamPoint = probePoint(for: downstream)
            let downstreamHit = caretHit(forLocalPoint: downstreamPoint)
            precondition(
                downstreamHit.index == index,
                "downstream caret round-trip failed at \(index): \(downstreamHit)"
            )
            precondition(
                characterIndex(for: screenPoint(forLocalPoint: downstreamPoint)) == index,
                "screen-space downstream round-trip failed at \(index)"
            )

            let upstream = caretRect(at: index, affinity: .upstream)
            guard !sameVisualLine(upstream, downstream) else { continue }
            affinitySplits += 1
            if index == 0 || (textStorage.string as NSString).character(at: index - 1) != 0x0A {
                softWrapSplits += 1
            }
            let upstreamPoint = probePoint(for: upstream)
            let upstreamHit = caretHit(forLocalPoint: upstreamPoint)
            precondition(
                upstreamHit.index == index && upstreamHit.affinity == .upstream,
                "upstream caret round-trip failed at \(index): \(upstreamHit)"
            )
            precondition(
                characterIndex(for: screenPoint(forLocalPoint: upstreamPoint)) == index,
                "screen-space upstream round-trip failed at \(index)"
            )
        }

        precondition(lineFragments >= 4, "test content must shape into multiple visual lines")
        precondition(affinitySplits > 0, "test content must contain a split caret position")
        precondition(softWrapSplits > 0, "test content must contain a soft-wrap caret split")
        print(
            "Layout self-check lines=\(lineFragments) boundaries=\(boundaries.count) "
                + "affinitySplits=\(affinitySplits) softWrapSplits=\(softWrapSplits)"
        )
    }

    func runUnicodeCompositionSelfCheck() {
        let savedStorage = NSAttributedString(attributedString: textStorage)
        let savedSelection = selection
        let savedAffinity = selectionAffinity
        let savedMarked = marked
        let base = textStorage.string

        selection = NSRange(location: textStorage.length, length: 0)
        marked = notFoundRange
        setMarkedText(
            "にほんご",
            selectedRange: NSRange(location: 4, length: 0),
            replacementRange: notFoundRange
        )
        precondition(hasMarkedText() && marked.length == 4, "Japanese preedit should be marked")
        precondition(rustComposition.overlayString() == "にほんご")
        precondition(rustComposition.overlaySelection() == NSRange(location: 4, length: 0))
        precondition(rustComposition.sourceString() == base)
        setMarkedText(
            "にほんご",
            selectedRange: NSRange(location: 4, length: 0),
            replacementRange: notFoundRange
        )
        precondition(rustComposition.overlayString() == "にほんご")
        insertText("日本語", replacementRange: notFoundRange)
        precondition(!hasMarkedText() && textStorage.string == base + "日本語")
        precondition(!rustComposition.hasOverlay && rustComposition.sourceString() == base + "日本語")

        setMarkedText(
            "\u{301}",
            selectedRange: NSRange(location: 1, length: 0),
            replacementRange: notFoundRange
        )
        setMarkedText(
            "e\u{301}",
            selectedRange: NSRange(location: 2, length: 0),
            replacementRange: notFoundRange
        )
        precondition(rustComposition.overlayString() == "e\u{301}")
        insertText("é", replacementRange: notFoundRange)
        precondition(!hasMarkedText() && textStorage.string == base + "日本語é")
        precondition(!rustComposition.hasOverlay && rustComposition.sourceString() == base + "日本語é")

        let cancelBase = textStorage.string
        setMarkedText(
            "にほん",
            selectedRange: NSRange(location: 3, length: 0),
            replacementRange: notFoundRange
        )
        precondition(hasMarkedText())
        doCommand(by: #selector(NSResponder.cancelOperation(_:)))
        precondition(!hasMarkedText() && textStorage.string == cancelBase)
        precondition(!rustComposition.hasOverlay && rustComposition.sourceString() == cancelBase)

        replaceStorage(
            range: NSRange(location: 0, length: textStorage.length),
            with: savedStorage
        )
        selection = savedSelection
        selectionAffinity = savedAffinity
        marked = savedMarked
        compositionOriginal = nil
        compositionSelectionBefore = nil
        compositionAffinityBefore = nil
        rustComposition.resetSource(textStorage.string)
        needsDisplay = true
        print(
            "Unicode composition self-check japanese=日本語 combining=é "
                + "cancel=restored"
        )
    }

    override func doCommand(by selector: Selector) {
        let command = NSStringFromSelector(selector)
        if command == "cancel:" || command == "cancelOperation:" {
            cancelComposition()
            return
        }
        switch selector {
        case #selector(NSResponder.deleteBackward(_:)):
            deleteBackward()
        case #selector(NSResponder.moveLeft(_:)):
            moveLeft()
        case #selector(NSResponder.moveRight(_:)):
            moveRight()
        case #selector(NSResponder.insertNewline(_:)):
            insertText("\n", replacementRange: notFoundRange)
        default:
            print("unhandled command: \(command)")
        }
    }

    private func attributedString(from value: Any, marked: Bool) -> NSAttributedString {
        let result: NSMutableAttributedString
        if let attributed = value as? NSAttributedString {
            result = NSMutableAttributedString(attributedString: attributed)
            result.addAttributes(defaultAttributes, range: NSRange(location: 0, length: result.length))
        } else {
            result = NSMutableAttributedString(
                string: String(describing: value),
                attributes: defaultAttributes
            )
        }
        if marked, result.length > 0 {
            result.addAttribute(
                .underlineStyle,
                value: NSUnderlineStyle.single.rawValue,
                range: NSRange(location: 0, length: result.length)
            )
        }
        return result
    }

    private func targetRange(_ replacementRange: NSRange) -> NSRange {
        if replacementRange.location != NSNotFound {
            return clamped(replacementRange)
        }
        if hasMarkedText() {
            return clamped(marked)
        }
        return clamped(selection)
    }

    private func clamped(_ range: NSRange) -> NSRange {
        guard range.location != NSNotFound else { return notFoundRange }
        let location = min(range.location, textStorage.length)
        let available = textStorage.length - location
        return NSRange(location: location, length: min(range.length, available))
    }

    private func validatedAccessibilityRange(_ range: NSRange) -> NSRange? {
        guard range.location != NSNotFound, range.location <= textStorage.length else {
            return nil
        }
        let (end, overflow) = range.location.addingReportingOverflow(range.length)
        guard !overflow, end <= textStorage.length else { return nil }
        return range
    }

    private struct CaretHit: CustomStringConvertible {
        let index: Int
        let affinity: NSSelectionAffinity

        var description: String {
            "CaretHit(index: \(index), affinity: \(affinity.rawValue))"
        }
    }

    private func caretHit(forLocalPoint point: NSPoint) -> CaretHit {
        let containerPoint = NSPoint(x: point.x - textOrigin.x, y: point.y - textOrigin.y)
        guard containerPoint.x >= 0, containerPoint.y >= 0 else {
            return CaretHit(index: 0, affinity: .downstream)
        }

        updateContainerSize()
        layoutManager.ensureLayout(for: textContainer)
        var fraction: CGFloat = 0
        let character = min(
            layoutManager.characterIndex(
                for: containerPoint,
                in: textContainer,
                fractionOfDistanceBetweenInsertionPoints: &fraction
            ),
            textStorage.length
        )
        let index: Int
        if character < textStorage.length {
            let cluster = (textStorage.string as NSString)
                .rangeOfComposedCharacterSequence(at: character)
            index = fraction >= 0.5 ? NSMaxRange(cluster) : cluster.location
        } else {
            index = textStorage.length
        }

        let upstream = caretRect(at: index, affinity: .upstream)
        let downstream = caretRect(at: index, affinity: .downstream)
        let affinity: NSSelectionAffinity
        if sameVisualLine(upstream, downstream) {
            affinity = .downstream
        } else {
            affinity = squaredDistance(point, upstream) < squaredDistance(point, downstream)
                ? .upstream : .downstream
        }
        return CaretHit(index: index, affinity: affinity)
    }

    private func canonicalCaretOffsets() -> [Int] {
        let string = textStorage.string as NSString
        var boundaries = [0]
        var index = 0
        while index < string.length {
            index = NSMaxRange(string.rangeOfComposedCharacterSequence(at: index))
            // TextKit canonicalizes a click at the end of a hard line to the
            // position after LF with upstream affinity. The position directly
            // before LF is therefore not an independent visual caret stop.
            if index == string.length || string.character(at: index) != 0x0A {
                boundaries.append(index)
            }
        }
        return boundaries
    }

    private func probePoint(for caret: NSRect) -> NSPoint {
        NSPoint(x: caret.minX + 0.25, y: caret.midY)
    }

    private func screenPoint(forLocalPoint point: NSPoint) -> NSPoint {
        let windowPoint = convert(point, to: nil)
        return window?.convertPoint(toScreen: windowPoint) ?? windowPoint
    }

    private func sameVisualLine(_ lhs: NSRect, _ rhs: NSRect) -> Bool {
        abs(lhs.midY - rhs.midY) < 0.5
    }

    private func squaredDistance(_ point: NSPoint, _ caret: NSRect) -> CGFloat {
        let dx = point.x - caret.minX
        let dy = point.y - caret.midY
        return dx * dx + dy * dy
    }

    private func logicalLine(containing index: Int) -> Int {
        let clampedIndex = min(max(index, 0), textStorage.length)
        let prefix = (textStorage.string as NSString).substring(to: clampedIndex)
        return prefix.utf8.reduce(into: 0) { count, byte in
            if byte == 0x0A { count += 1 }
        }
    }

    private func logicalLineRange(_ requestedLine: Int) -> NSRange {
        guard requestedLine >= 0 else { return notFoundRange }
        let string = textStorage.string as NSString
        var line = 0
        var start = 0

        while line < requestedLine {
            guard start < string.length else { return notFoundRange }
            let range = string.lineRange(for: NSRange(location: start, length: 0))
            let next = NSMaxRange(range)
            guard next > start else { return notFoundRange }
            start = next
            line += 1
        }

        if start == string.length {
            let hasTrailingLine = string.length == 0 || string.character(at: string.length - 1) == 0x0A
            return hasTrailingLine ? NSRange(location: start, length: 0) : notFoundRange
        }
        return string.lineRange(for: NSRange(location: start, length: 0))
    }

    private func postTextChanged() {
        NSAccessibility.post(element: self, notification: .valueChanged)
        postSelectionChanged()
    }

    private func postSelectionChanged() {
        NSAccessibility.post(element: self, notification: .selectedTextChanged)
    }

    private func replaceStorage(range: NSRange, with replacement: NSAttributedString) {
        textStorage.beginEditing()
        textStorage.replaceCharacters(in: range, with: replacement)
        textStorage.endEditing()
    }

    private func deleteBackward() {
        if selection.length > 0 {
            insertText("", replacementRange: selection)
            return
        }
        guard selection.location > 0 else { return }
        let string = textStorage.string as NSString
        let range = string.rangeOfComposedCharacterSequence(at: selection.location - 1)
        insertText("", replacementRange: range)
    }

    private func cancelComposition() {
        print("cancelComposition range=\(marked)")
        guard hasMarkedText(), let original = compositionOriginal else {
            marked = notFoundRange
            rustComposition.cancel()
            return
        }
        replaceStorage(range: marked, with: original)
        selection = compositionSelectionBefore ?? NSRange(location: marked.location, length: 0)
        selectionAffinity = compositionAffinityBefore ?? .downstream
        marked = notFoundRange
        compositionOriginal = nil
        compositionSelectionBefore = nil
        compositionAffinityBefore = nil
        rustComposition.cancel()
        needsDisplay = true
        postTextChanged()
    }

    private func moveLeft() {
        guard selection.location > 0 else { return }
        let string = textStorage.string as NSString
        let range = string.rangeOfComposedCharacterSequence(at: selection.location - 1)
        selection = NSRange(location: range.location, length: 0)
        selectionAffinity = .downstream
        needsDisplay = true
        postSelectionChanged()
    }

    private func moveRight() {
        guard selection.location < textStorage.length else { return }
        let string = textStorage.string as NSString
        let range = string.rangeOfComposedCharacterSequence(at: selection.location)
        selection = NSRange(location: NSMaxRange(range), length: 0)
        selectionAffinity = .downstream
        needsDisplay = true
        postSelectionChanged()
    }

    private func updateContainerSize() {
        textContainer.containerSize = NSSize(
            width: min(max(bounds.width - textOrigin.x * 2, 1), maximumTextWidth),
            height: .greatestFiniteMagnitude
        )
    }

    private func caretRect(
        at characterIndex: Int,
        affinity: NSSelectionAffinity
    ) -> NSRect {
        let downstream = downstreamCaretRect(at: characterIndex)
        guard affinity == .upstream, characterIndex > 0 else { return downstream }

        let previousCharacter = (textStorage.string as NSString)
            .rangeOfComposedCharacterSequence(at: min(characterIndex, textStorage.length) - 1)
        let previousGlyphs = layoutManager.glyphRange(
            forCharacterRange: previousCharacter,
            actualCharacterRange: nil
        )
        guard previousGlyphs.length > 0 else { return downstream }

        let lastGlyph = NSMaxRange(previousGlyphs) - 1
        let previousLine = layoutManager.lineFragmentUsedRect(
            forGlyphAt: lastGlyph,
            effectiveRange: nil
        )
        guard abs(textOrigin.y + previousLine.midY - downstream.midY) >= 0.5 else {
            return downstream
        }
        let previousBounds = layoutManager.boundingRect(
            forGlyphRange: previousGlyphs,
            in: textContainer
        )
        return NSRect(
            x: textOrigin.x + previousBounds.maxX,
            y: textOrigin.y + previousLine.minY,
            width: 1.5,
            height: max(previousLine.height, 26)
        )
    }

    private func downstreamCaretRect(at characterIndex: Int) -> NSRect {
        updateContainerSize()
        layoutManager.ensureLayout(for: textContainer)

        guard textStorage.length > 0 else {
            return NSRect(x: textOrigin.x, y: textOrigin.y, width: 1.5, height: 26)
        }

        let clampedIndex = min(max(characterIndex, 0), textStorage.length)
        if clampedIndex == textStorage.length,
            (textStorage.string as NSString).hasSuffix("\n"),
            !layoutManager.extraLineFragmentUsedRect.isEmpty
        {
            let extra = layoutManager.extraLineFragmentUsedRect
            return NSRect(
                x: textOrigin.x + extra.minX,
                y: textOrigin.y + extra.minY,
                width: 1.5,
                height: max(extra.height, 26)
            )
        }

        let glyphIndex: Int
        if clampedIndex == textStorage.length {
            glyphIndex = max(layoutManager.numberOfGlyphs - 1, 0)
        } else {
            glyphIndex = layoutManager.glyphIndexForCharacter(at: clampedIndex)
        }
        let line = layoutManager.lineFragmentUsedRect(forGlyphAt: glyphIndex, effectiveRange: nil)
        let glyphLocation = layoutManager.location(forGlyphAt: glyphIndex)
        var x = glyphLocation.x
        if clampedIndex == textStorage.length {
            let finalRange = NSRange(location: glyphIndex, length: 1)
            x = NSMaxX(layoutManager.boundingRect(forGlyphRange: finalRange, in: textContainer))
        }
        return NSRect(
            x: textOrigin.x + x,
            y: textOrigin.y + line.minY,
            width: 1.5,
            height: max(line.height, 26)
        )
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var window: NSWindow?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let frame = NSRect(x: 0, y: 0, width: 760, height: 480)
        let window = NSWindow(
            contentRect: frame,
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "Yu — macOS Text Input Spike"
        window.center()

        let inputView = TextInputView(frame: frame)
        inputView.autoresizingMask = [.width, .height]
        window.contentView = inputView
        window.makeKeyAndOrderFront(nil)
        window.makeFirstResponder(inputView)
        inputView.runLayoutRoundTripSelfCheck()
        inputView.runUnicodeCompositionSelfCheck()
        inputView.runAccessibilitySelfCheck()
        NSApp.activate(ignoringOtherApps: true)
        DispatchQueue.global(qos: .userInitiated).asyncAfter(deadline: .now() + 0.25) {
            runAccessibilityRuntimeProbe()
        }
        self.window = window
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
}

private func runAccessibilityRuntimeProbe() {
    let application = AXUIElementCreateApplication(getpid())
    var focusedValue: CFTypeRef?
    let focusedError = AXUIElementCopyAttributeValue(
        application,
        kAXFocusedUIElementAttribute as CFString,
        &focusedValue
    )
    guard focusedError == .success, let focused = focusedValue else {
        print("AX runtime probe focused-element error=\(focusedError.rawValue)")
        return
    }
    let element = focused as! AXUIElement

    var roleValue: CFTypeRef?
    let roleError = AXUIElementCopyAttributeValue(
        element,
        kAXRoleAttribute as CFString,
        &roleValue
    )
    var countValue: CFTypeRef?
    let countError = AXUIElementCopyAttributeValue(
        element,
        kAXNumberOfCharactersAttribute as CFString,
        &countValue
    )
    let count = (countValue as? NSNumber)?.intValue ?? 0
    var firstLine = CFRange(location: 0, length: min(count, 19))
    guard let rangeValue = AXValueCreate(.cfRange, &firstLine) else {
        print("AX runtime probe could not create range value")
        return
    }

    var stringValue: CFTypeRef?
    let stringError = AXUIElementCopyParameterizedAttributeValue(
        element,
        kAXStringForRangeParameterizedAttribute as CFString,
        rangeValue,
        &stringValue
    )
    var boundsValue: CFTypeRef?
    let boundsError = AXUIElementCopyParameterizedAttributeValue(
        element,
        kAXBoundsForRangeParameterizedAttribute as CFString,
        rangeValue,
        &boundsValue
    )

    print(
        "AX runtime probe trusted=\(AXIsProcessTrusted()) "
            + "role=\(String(describing: roleValue)) roleError=\(roleError.rawValue) "
            + "characters=\(count) countError=\(countError.rawValue) "
            + "string=\(String(describing: stringValue)) stringError=\(stringError.rawValue) "
            + "bounds=\(String(describing: boundsValue)) boundsError=\(boundsError.rawValue)"
    )
}

let application = NSApplication.shared
let delegate = AppDelegate()
application.setActivationPolicy(.regular)
application.delegate = delegate
application.run()
