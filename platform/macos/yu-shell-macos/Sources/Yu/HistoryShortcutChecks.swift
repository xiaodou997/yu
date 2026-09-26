import AppKit

/// Native responder checks only. These do not assert delivery through WindowServer.
func checkHistoryShortcutRouting() throws {
    let original = "# History routing\r\n\r\n$x^2$\r\n\r\n```mermaid\r\nflowchart LR\r\nA --> B\r\n```\r\n中文🙂\r\n"
    let temporary = FileManager.default.temporaryDirectory
        .appendingPathComponent("yu-history-routing-\(UUID().uuidString).md")
    try Data(original.utf8).write(to: temporary)
    defer { try? FileManager.default.removeItem(at: temporary) }
    var failures: [String] = []
    func check(_ name: String, _ body: (StorageBridge, DocumentTextView, NSWindow) throws -> Bool) throws {
        let bridge = try StorageBridge(path: temporary.path)
        let view = DocumentTextView(bridge: bridge)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 640, height: 480),
                              styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = view
        defer { window.contentView = nil; window.close() }
        guard window.makeFirstResponder(view) else { throw CocoaError(.coderInvalidValue) }
        view.insertText(" CHANGE", replacementRange: NSRange(location: original.utf16.count, length: 0))
        let passed = try body(bridge, view, window)
        print("Yu history route \(name): \(passed ? "passed" : "FAILED")")
        if !passed { failures.append(name) }
    }
    func event(_ window: NSWindow, redo: Bool = false, characters: String = "z",
               ignoring: String = "z", extra: NSEvent.ModifierFlags = []) -> NSEvent {
        NSEvent.keyEvent(with: .keyDown, location: .zero,
            modifierFlags: [.command, redo ? .shift : [], extra], timestamp: 1,
            windowNumber: window.windowNumber, context: nil, characters: characters,
            charactersIgnoringModifiers: ignoring, isARepeat: false, keyCode: 6)!
    }
    try check("key-equivalent-undo") { bridge, view, window in
        view.performKeyEquivalent(with: event(window)) && bridge.source == original
    }
    try check("key-down-undo") { bridge, view, window in
        view.keyDown(with: event(window))
        return bridge.source == original
    }
    try check("key-down-redo") { bridge, view, window in
        view.performUndo()
        view.keyDown(with: event(window, redo: true, characters: "Z", ignoring: "Z"))
        return bridge.source == original + " CHANGE"
    }
    try check("command-translated-character") { bridge, view, window in
        view.performKeyEquivalent(with: event(window, characters: "z", ignoring: "ｚ"))
            && bridge.source == original
    }
    try check("readonly-key-equivalent") { bridge, view, window in
        view.isEditable = false
        let revision = bridge.revision
        let handled = view.performKeyEquivalent(with: event(window))
        return !handled && bridge.revision == revision && bridge.source == original + " CHANGE"
    }
    try check("other-responder") { bridge, view, window in
        let field = NSTextView(frame: .zero)
        view.addSubview(field)
        guard window.makeFirstResponder(field) else { return false }
        let revision = bridge.revision
        return !view.performKeyEquivalent(with: event(window)) && bridge.revision == revision
    }
    try check("active-preedit") { bridge, view, window in
        view.setMarkedText("ceshi", selectedRange: NSRange(location: 5, length: 0),
                           replacementRange: NSRange(location: NSNotFound, length: 0))
        let revision = bridge.revision
        let handled = view.performKeyEquivalent(with: event(window))
        return !handled && bridge.composition.active && bridge.revision == revision
    }
    try check("modified-command-not-history") { bridge, view, window in
        let revision = bridge.revision
        let handled = view.performKeyEquivalent(with: event(window, extra: .option))
        return !handled && bridge.revision == revision
    }
    try check("one-event-one-history-step") { bridge, view, window in
        view.insertText("PREFIX ", replacementRange: NSRange(location: 0, length: 0))
        let revision = bridge.revision
        var changes = 0
        view.onDocumentChange = { changes += 1 }
        let key = event(window)
        if !view.performKeyEquivalent(with: key) { view.keyDown(with: key) }
        guard bridge.source == original + " CHANGE", bridge.revision == revision + 1,
              changes == 1 else { return false }
        view.keyDown(with: event(window))
        guard bridge.source == original, bridge.revision == revision + 2, changes == 2 else { return false }
        view.keyDown(with: event(window, redo: true))
        view.keyDown(with: event(window, redo: true))
        return bridge.source == "PREFIX " + original + " CHANGE" && changes == 4
    }
    try check("key-down-does-not-steal-field-history") { bridge, view, window in
        let field = NSTextView(frame: .zero)
        view.addSubview(field)
        guard window.makeFirstResponder(field) else { return false }
        let revision = bridge.revision
        view.keyDown(with: event(window))
        return bridge.revision == revision && bridge.source == original + " CHANGE"
    }
    try check("key-down-respects-readonly") { bridge, view, window in
        view.isEditable = false
        let revision = bridge.revision
        view.keyDown(with: event(window))
        return bridge.revision == revision && bridge.source == original + " CHANGE"
    }
    try check("physical-key-is-not-history-identity") { bridge, view, window in
        let revision = bridge.revision
        return !view.performKeyEquivalent(with: event(window, characters: "y", ignoring: "y"))
            && bridge.revision == revision
    }
    if !failures.isEmpty {
        throw NSError(domain: "YuHistoryShortcutChecks", code: 1,
                      userInfo: [NSLocalizedDescriptionKey: failures.joined(separator: ", ")])
    }
}
