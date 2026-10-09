import AppKit
import YuStorageFFI

/// A separate metadata index, never a document session or text-content cache.
final class NativeWorkspaceIndex {
    struct File: Decodable { let path: String; let relative: String }
    struct Snapshot: Decodable {
        let root: String
        let total: Int
        let truncated: Bool
        let skipped: Int
        let files: [File]
    }
    private var handle: OpaquePointer?
    init(root: URL) throws {
        let bytes = Array(root.path.utf8)
        let status = bytes.withUnsafeBufferPointer {
            yu_storage_workspace_start($0.baseAddress, $0.count, &handle)
        }
        guard status == 0, handle != nil else { throw CocoaError(.fileReadUnknown) }
    }
    deinit { yu_storage_workspace_destroy(handle) }

    /// nil means the background scan is still pending, not an empty workspace.
    func query(_ text: String) throws -> Snapshot? {
        let bytes = Array(text.utf8)
        var length = 0
        let status = bytes.withUnsafeBufferPointer {
            yu_storage_workspace_query(handle, $0.baseAddress, $0.count, nil, 0, &length)
        }
        if status == 23 { return nil }
        guard status == 0 || status == 8, length <= 16 * 1024 * 1024 else { throw CocoaError(.fileReadUnknown) }
        var data = [UInt8](repeating: 0, count: length)
        let copied = bytes.withUnsafeBufferPointer { query in
            data.withUnsafeMutableBufferPointer {
                yu_storage_workspace_query(handle, query.baseAddress, query.count, $0.baseAddress, $0.count, &length)
            }
        }
        guard copied == 0, length == data.count else { throw CocoaError(.fileReadUnknown) }
        return try JSONDecoder().decode(Snapshot.self, from: Data(data))
    }
    func resolve(_ path: String) throws -> URL {
        let bytes = Array(path.utf8)
        var length = 0
        let status = bytes.withUnsafeBufferPointer {
            yu_storage_workspace_resolve(handle, $0.baseAddress, $0.count, nil, 0, &length)
        }
        guard status == 0 || status == 8, length <= 1024 * 1024 else { throw CocoaError(.fileReadNoSuchFile) }
        var data = [UInt8](repeating: 0, count: length)
        let copied = bytes.withUnsafeBufferPointer { path in
            data.withUnsafeMutableBufferPointer {
                yu_storage_workspace_resolve(handle, path.baseAddress, path.count, $0.baseAddress, $0.count, &length)
            }
        }
        guard copied == 0 else { throw CocoaError(.fileReadNoSuchFile) }
        // Foundation canonicalizes /private/var aliases differently from POSIX
        // realpath. Return the same URL identity used by native document routing.
        return URL(fileURLWithPath: String(decoding: data, as: UTF8.self)).resolvingSymlinksInPath()
    }
}

private final class QuickOpenTable: NSTableView {
    var accept: (() -> Void)?
    var cancel: (() -> Void)?
    override func keyDown(with event: NSEvent) {
        if event.keyCode == 36 || event.keyCode == 76 { accept?() }
        else if event.keyCode == 53 { cancel?() }
        else { super.keyDown(with: event) }
    }
}

/// Native document-modal file switcher. Files are opened only after the sheet
/// closes, through the existing unsaved-change/recovery-aware navigation path.
final class QuickOpenPanel: NSObject, NSSearchFieldDelegate, NSTableViewDataSource, NSTableViewDelegate {
    let panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 640, height: 440),
        styleMask: [.titled], backing: .buffered, defer: false)
    private let field = NSSearchField()
    private let table = QuickOpenTable()
    private let status = NSTextField(labelWithString: "")
    private let openButton = NSButton()
    private var index: NativeWorkspaceIndex?
    private var timer: Timer?
    private var files: [NativeWorkspaceIndex.File] = []
    private let root: URL
    private var selectedURL: URL?
    private var completion: ((URL?) -> Void)?

    init(root: URL) {
        self.root = root
        super.init()
        panel.title = L10n.tr("Quick Open")
        panel.isReleasedWhenClosed = false
        let content = NSView()
        panel.contentView = content
        let folder = NSTextField(labelWithString: root.path)
        folder.lineBreakMode = .byTruncatingMiddle
        folder.textColor = .secondaryLabelColor
        folder.toolTip = root.path
        field.placeholderString = L10n.tr("Search file names or paths")
        field.setAccessibilityLabel(L10n.tr("Search file names or paths"))
        field.delegate = self
        field.sendsSearchStringImmediately = true
        field.sendsWholeSearchString = false
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("workspace-file"))
        table.addTableColumn(column)
        table.headerView = nil
        table.rowHeight = 42
        table.columnAutoresizingStyle = .firstColumnOnlyAutoresizingStyle
        table.dataSource = self
        table.delegate = self
        table.target = self
        table.doubleAction = #selector(openSelected(_:))
        table.setAccessibilityLabel(L10n.tr("Workspace Files"))
        table.accept = { [weak self] in self?.openSelected(nil) }
        table.cancel = { [weak self] in self?.close(nil) }
        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        status.textColor = .secondaryLabelColor
        status.font = .systemFont(ofSize: 11)
        status.lineBreakMode = .byTruncatingTail
        openButton.title = L10n.tr("Open")
        openButton.target = self
        openButton.action = #selector(openSelected(_:))
        openButton.keyEquivalent = "\r"
        openButton.isEnabled = false
        let cancel = NSButton(title: L10n.tr("Cancel"), target: self, action: #selector(close(_:)))
        cancel.keyEquivalent = "\u{1b}"
        let refresh = NSButton(title: L10n.tr("Refresh Files"), target: self, action: #selector(refresh(_:)))
        for button in [openButton, cancel, refresh] { button.bezelStyle = .rounded }
        let actions = NSStackView(views: [refresh, NSView(), cancel, openButton])
        actions.orientation = .horizontal
        actions.spacing = 10
        for child in [folder, field, scroll, status, actions] {
            child.translatesAutoresizingMaskIntoConstraints = false
            content.addSubview(child)
        }
        NSLayoutConstraint.activate([
            folder.topAnchor.constraint(equalTo: content.topAnchor, constant: 12),
            folder.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 16),
            folder.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -16),
            field.topAnchor.constraint(equalTo: folder.bottomAnchor, constant: 10),
            field.leadingAnchor.constraint(equalTo: folder.leadingAnchor),
            field.trailingAnchor.constraint(equalTo: folder.trailingAnchor),
            scroll.topAnchor.constraint(equalTo: field.bottomAnchor, constant: 10),
            scroll.leadingAnchor.constraint(equalTo: folder.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: folder.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: status.topAnchor, constant: -10),
            status.leadingAnchor.constraint(equalTo: folder.leadingAnchor),
            status.trailingAnchor.constraint(equalTo: folder.trailingAnchor),
            status.bottomAnchor.constraint(equalTo: actions.topAnchor, constant: -10),
            actions.leadingAnchor.constraint(equalTo: folder.leadingAnchor),
            actions.trailingAnchor.constraint(equalTo: folder.trailingAnchor),
            actions.bottomAnchor.constraint(equalTo: content.bottomAnchor, constant: -12)
        ])
    }
    deinit { timer?.invalidate() }

    func begin(on owner: NSWindow, completion: @escaping (URL?) -> Void) {
        self.completion = completion
        owner.beginSheet(panel) { [weak self] _ in
            guard let self else { return }
            self.timer?.invalidate()
            self.timer = nil
            self.index = nil
            let callback = self.completion
            self.completion = nil
            self.panel.orderOut(nil)
            callback?(self.selectedURL)
        }
        panel.makeFirstResponder(field)
        refresh(nil)
    }
    @objc private func refresh(_ sender: Any?) {
        timer?.invalidate()
        index = nil
        files = []
        table.reloadData()
        setOpenEnabled(false)
        status.stringValue = L10n.tr("Scanning folder…")
        do { index = try NativeWorkspaceIndex(root: root) }
        catch { showFailure(); return }
        timer = Timer.scheduledTimer(withTimeInterval: 0.08, repeats: true) { [weak self] _ in self?.reload() }
        if let timer { RunLoop.main.add(timer, forMode: .common) }
        reload()
    }
    private func showFailure() {
        files = []
        table.reloadData()
        if panel.firstResponder === openButton { panel.makeFirstResponder(field) }
        timer?.invalidate(); timer = nil
        status.stringValue = L10n.tr("Folder unavailable. Choose another folder or refresh.")
        setOpenEnabled(false)
    }
    private func setOpenEnabled(_ enabled: Bool) {
        if !enabled, panel.firstResponder === openButton || panel.firstResponder === table {
            panel.makeFirstResponder(field)
        }
        openButton.isEnabled = enabled
    }
    private func reload() {
        guard (field.currentEditor() as? NSTextView)?.hasMarkedText() != true else { return }
        do {
            guard let snapshot = try index?.query(field.stringValue) else { return }
            timer?.invalidate(); timer = nil
            let selected = files.indices.contains(table.selectedRow) ? files[table.selectedRow].path : nil
            files = snapshot.files
            table.reloadData()
            let row = files.firstIndex(where: { $0.path == selected }) ?? 0
            if !files.isEmpty { table.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false) }
            setOpenEnabled(!files.isEmpty)
            status.stringValue = files.isEmpty
                ? L10n.tr(snapshot.total == 0 ? "No supported files in this folder" : "No matching files")
                : L10n.format("%d files shown · %d indexed", files.count, snapshot.total)
            if snapshot.truncated || snapshot.skipped > 0 { status.stringValue += " · " + L10n.tr("Partial index; choose a smaller folder") }
        } catch { showFailure() }
    }
    func controlTextDidChange(_ notification: Notification) {
        guard (field.currentEditor() as? NSTextView)?.hasMarkedText() != true else { return }
        reload()
    }
    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        guard !textView.hasMarkedText() else { return false }
        switch NSStringFromSelector(selector) {
        case "insertNewline:": openSelected(nil); return true
        case "cancelOperation:": close(nil); return true
        case "moveDown:": moveSelection(1); return true
        case "moveUp:": moveSelection(-1); return true
        default: return false
        }
    }
    private func moveSelection(_ delta: Int) {
        guard !files.isEmpty else { return }
        let row = min(files.count - 1, max(0, table.selectedRow + delta))
        table.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        table.scrollRowToVisible(row)
    }
    @objc private func openSelected(_ sender: Any?) {
        guard (field.currentEditor() as? NSTextView)?.hasMarkedText() != true,
              openButton.isEnabled, files.indices.contains(table.selectedRow) else { return }
        do {
            selectedURL = try index?.resolve(files[table.selectedRow].path)
            close(nil)
        } catch {
            status.stringValue = L10n.tr("File unavailable. Refresh the file list.")
            setOpenEnabled(false)
        }
    }
    @objc private func close(_ sender: Any?) {
        guard (field.currentEditor() as? NSTextView)?.hasMarkedText() != true else { return }
        panel.sheetParent?.endSheet(panel)
    }
    @MainActor
    static func runWindowSelfCheck(on owner: NSWindow) async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("yu-quick-open-window-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root.appendingPathComponent("notes"), withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let target = root.appendingPathComponent("notes/羽🙂.md")
        try "# Local".write(to: target, atomically: true, encoding: .utf8)
        try "# Other".write(to: root.appendingPathComponent("other.md"), atomically: true, encoding: .utf8)
        let picker = QuickOpenPanel(root: root)
        var completed = false
        var selected: URL?
        picker.begin(on: owner) { selected = $0; completed = true }
        defer { if owner.attachedSheet === picker.panel { owner.endSheet(picker.panel) } }
        let deadline = Date().addingTimeInterval(5)
        while picker.files.isEmpty && Date() < deadline { try await Task.sleep(nanoseconds: 10_000_000) }
        guard picker.files.count == 2, owner.attachedSheet === picker.panel,
              picker.panel.firstResponder === picker.field.currentEditor() else { throw CocoaError(.coderInvalidValue) }
        picker.panel.makeFirstResponder(picker.table)
        picker.field.stringValue = "missing-file"
        picker.reload()
        guard picker.files.isEmpty, !picker.openButton.isEnabled,
              picker.panel.firstResponder === picker.field.currentEditor() else { throw CocoaError(.coderInvalidValue) }
        picker.field.stringValue = "other"
        picker.reload()
        try FileManager.default.removeItem(at: root.appendingPathComponent("other.md"))
        picker.panel.makeFirstResponder(picker.openButton)
        picker.openSelected(nil)
        guard !completed, !picker.openButton.isEnabled,
              picker.panel.firstResponder === picker.field.currentEditor() else { throw CocoaError(.coderInvalidValue) }
        picker.field.stringValue = "notes 羽🙂"
        picker.reload()
        guard picker.files.count == 1, picker.files[0].relative == "notes/羽🙂.md" else { throw CocoaError(.coderInvalidValue) }
        picker.openSelected(nil)
        while !completed && Date() < deadline { try await Task.sleep(nanoseconds: 10_000_000) }
        guard completed, selected?.resolvingSymlinksInPath() == target.resolvingSymlinksInPath() else { throw CocoaError(.coderInvalidValue) }
        let cancelled = QuickOpenPanel(root: root)
        var didCancel = false
        cancelled.begin(on: owner) { didCancel = $0 == nil }
        cancelled.close(nil)
        let cancelDeadline = Date().addingTimeInterval(5)
        while !didCancel && Date() < cancelDeadline { try await Task.sleep(nanoseconds: 10_000_000) }
        guard didCancel, cancelled.timer == nil, cancelled.index == nil, owner.attachedSheet == nil else { throw CocoaError(.coderInvalidValue) }
        print("Yu quick-open window self-check: native sheet, query focus, empty/stale result focus, Unicode ranking, selection and scan cancellation passed")
    }

    func numberOfRows(in tableView: NSTableView) -> Int { files.count }
    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard files.indices.contains(row) else { return nil }
        let file = files[row]
        let title = NSTextField(labelWithString: URL(fileURLWithPath: file.path).lastPathComponent)
        title.font = .systemFont(ofSize: 13, weight: .medium)
        let path = NSTextField(labelWithString: file.relative)
        path.font = .systemFont(ofSize: 11)
        path.textColor = .secondaryLabelColor
        for label in [title, path] { label.lineBreakMode = .byTruncatingMiddle }
        let stack = NSStackView(views: [title, path])
        stack.orientation = .vertical; stack.alignment = .leading; stack.spacing = 2
        stack.edgeInsets = NSEdgeInsets(top: 4, left: 6, bottom: 4, right: 6)
        stack.toolTip = file.path
        return stack
    }
}
