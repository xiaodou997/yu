import AppKit
import ApplicationServices

private let notFoundRange = NSRange(location: NSNotFound, length: 0)

final class TextInputView: NSView, NSTextInputClient {
    private let textStorage = NSTextStorage()
    private let layoutManager = NSLayoutManager()
    private let textContainer = NSTextContainer()
    private var selection = NSRange(location: 0, length: 0)
    private var marked = notFoundRange
    private var compositionOriginal: NSAttributedString?
    private var compositionSelectionBefore: NSRange?

    private let textOrigin = NSPoint(x: 24, y: 24)
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
            let caret = caretRect(at: selection.location)
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
        let index = characterIndex(for: point)
        selection = NSRange(location: index, length: 0)
        marked = notFoundRange
        inputContext?.discardMarkedText()
        needsDisplay = true
        postSelectionChanged()
    }

    func insertText(_ value: Any, replacementRange: NSRange) {
        let inserted = attributedString(from: value, marked: false)
        let target = targetRange(replacementRange)
        print("insertText commit=\(inserted.string.debugDescription) replace=\(target)")
        replaceStorage(range: target, with: inserted)
        selection = NSRange(location: target.location + inserted.length, length: 0)
        marked = notFoundRange
        compositionOriginal = nil
        compositionSelectionBefore = nil
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
            localRect = caretRect(at: range.location)
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
        let local = NSPoint(x: point.x - textOrigin.x, y: point.y - textOrigin.y)
        guard local.x >= 0, local.y >= 0 else { return 0 }
        updateContainerSize()
        let glyph = layoutManager.glyphIndex(for: local, in: textContainer)
        return min(layoutManager.characterIndexForGlyph(at: glyph), textStorage.length)
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
        let windowPoint = window?.convertPoint(fromScreen: position) ?? position
        let local = convert(windowPoint, from: nil)
        return accessibilityRange(for: characterIndex(for: local))
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
            return
        }
        replaceStorage(range: marked, with: original)
        selection = compositionSelectionBefore ?? NSRange(location: marked.location, length: 0)
        marked = notFoundRange
        compositionOriginal = nil
        compositionSelectionBefore = nil
        needsDisplay = true
        postTextChanged()
    }

    private func moveLeft() {
        guard selection.location > 0 else { return }
        let string = textStorage.string as NSString
        let range = string.rangeOfComposedCharacterSequence(at: selection.location - 1)
        selection = NSRange(location: range.location, length: 0)
        needsDisplay = true
        postSelectionChanged()
    }

    private func moveRight() {
        guard selection.location < textStorage.length else { return }
        let string = textStorage.string as NSString
        let range = string.rangeOfComposedCharacterSequence(at: selection.location)
        selection = NSRange(location: NSMaxRange(range), length: 0)
        needsDisplay = true
        postSelectionChanged()
    }

    private func updateContainerSize() {
        textContainer.containerSize = NSSize(
            width: max(bounds.width - textOrigin.x * 2, 1),
            height: .greatestFiniteMagnitude
        )
    }

    private func caretRect(at characterIndex: Int) -> NSRect {
        updateContainerSize()
        layoutManager.ensureLayout(for: textContainer)

        guard textStorage.length > 0 else {
            return NSRect(x: textOrigin.x, y: textOrigin.y, width: 1.5, height: 26)
        }

        let clampedIndex = min(characterIndex, textStorage.length)
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
