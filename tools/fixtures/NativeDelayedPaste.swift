// Synthetic AppKit consumer for the external driver's clipboard-lifetime test.
// It is not Yu, and its success is not product acceptance.
import AppKit

final class DelayedEditor: NSTextView {
    var pasteCount = 0
    let receiptURL: URL
    private let fixtureStorage: NSTextStorage
    init(receiptURL: URL) {
        self.receiptURL = receiptURL
        let storage = NSTextStorage(string: "BEFORE")
        self.fixtureStorage = storage
        let layout = NSLayoutManager()
        let container = NSTextContainer(size: NSSize(width: 700, height: 420))
        storage.addLayoutManager(layout)
        layout.addTextContainer(container)
        super.init(frame: NSRect(x: 0, y: 0, width: 700, height: 420), textContainer: container)
        isRichText = false
        string = "BEFORE"
        setAccessibilityIdentifier("yu-document-text")
    }
    required init?(coder: NSCoder) { fatalError("not used") }
    override func paste(_ sender: Any?) {
        pasteCount += 1
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.9) { [self] in
            let value = NSPasteboard.general.string(forType: .string) ?? "MISSING"
            insertText(value, replacementRange: selectedRange())
            let data = try! JSONSerialization.data(withJSONObject: ["pastes": pasteCount, "result": string])
            try! data.write(to: receiptURL, options: .atomic)
        }
    }
}

final class FixtureDelegate: NSObject, NSApplicationDelegate {
    let directory: URL
    var window: NSWindow?
    var saved: [NSPasteboardItem] = []
    var stopTimer: Timer?
    init(directory: URL) { self.directory = directory }
    func applicationDidFinishLaunching(_ notification: Notification) {
        // Finish potentially failing view construction before borrowing the clipboard.
        let editor = DelayedEditor(receiptURL: directory.appendingPathComponent("receipt.json"))
        let board = NSPasteboard.general
        saved = (board.pasteboardItems ?? []).map { item in
            let copy = NSPasteboardItem()
            for type in item.types {
                if let data = item.data(forType: type) { copy.setData(data, forType: type) }
            }
            return copy
        }
        board.clearContents()
        board.setString("PRIOR_PAYLOAD", forType: .string)
        let window = NSWindow(contentRect: editor.frame, styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.title = "Isolated delayed paste fixture"
        window.contentView = editor
        let menu = NSMenu()
        let edit = NSMenuItem(title: "Edit", action: nil, keyEquivalent: "")
        edit.submenu = NSMenu(title: "Edit")
        edit.submenu?.addItem(NSMenuItem(title: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v"))
        menu.addItem(edit)
        NSApplication.shared.mainMenu = menu
        window.makeFirstResponder(editor)
        window.center()
        window.makeKeyAndOrderFront(nil)
        self.window = window
        NSApplication.shared.activate(ignoringOtherApps: true)
        stopTimer = Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) { [weak self] _ in
            guard let self else { return }
            if FileManager.default.fileExists(atPath: directory.appendingPathComponent("stop").path) {
                NSApplication.shared.terminate(nil)
            }
        }
    }
    func applicationWillTerminate(_ notification: Notification) {
        stopTimer?.invalidate()
        let board = NSPasteboard.general
        board.clearContents()
        if !saved.isEmpty { board.writeObjects(saved) }
    }
}

let app = NSApplication.shared
let delegate = FixtureDelegate(directory: URL(fileURLWithPath: CommandLine.arguments[1]))
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
