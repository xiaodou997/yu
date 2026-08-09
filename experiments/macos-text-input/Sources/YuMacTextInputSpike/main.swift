import AppKit

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
    }

    private func moveLeft() {
        guard selection.location > 0 else { return }
        let string = textStorage.string as NSString
        let range = string.rangeOfComposedCharacterSequence(at: selection.location - 1)
        selection = NSRange(location: range.location, length: 0)
        needsDisplay = true
    }

    private func moveRight() {
        guard selection.location < textStorage.length else { return }
        let string = textStorage.string as NSString
        let range = string.rangeOfComposedCharacterSequence(at: selection.location)
        selection = NSRange(location: NSMaxRange(range), length: 0)
        needsDisplay = true
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
        NSApp.activate(ignoringOtherApps: true)
        self.window = window
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
}

let application = NSApplication.shared
let delegate = AppDelegate()
application.setActivationPolicy(.regular)
application.delegate = delegate
application.run()
