import AppKit
import UniformTypeIdentifiers
import YuStorageFFI

private enum StorageStatus {
    static let ok: Int32 = 0
    static let externalChange: Int32 = 4
    static let unsavedChanges: Int32 = 5
}

private enum DiskState: UInt8 {
    case unchanged = 0
    case changed = 1
    case missing = 2

    var label: String {
        switch self {
        case .unchanged: return "磁盘一致"
        case .changed: return "外部已修改"
        case .missing: return "文件不存在"
        }
    }
}

private struct NativeStorageState {
    let revision: UInt64
    let savedRevision: UInt64
    let dirty: Bool
    let disk: DiskState
    let bom: Bool
    let closeState: UInt8

    init(_ value: YuStorageState) {
        revision = value.revision
        savedRevision = value.saved_revision
        dirty = value.dirty != 0
        disk = DiskState(rawValue: value.disk_state) ?? .changed
        bom = value.bom != 0
        closeState = value.close_state
    }
}

private struct NativeSelection {
    let revision: UInt64
    let range: NSRange
    let affinity: UInt8

    init(_ value: YuStorageSelection) {
        revision = value.revision
        range = NSRange(
            location: Int(value.start_utf16),
            length: Int(value.end_utf16 - value.start_utf16)
        )
        affinity = value.affinity
    }
}

private struct NativeComposition {
    let revision: UInt64
    let generation: UInt64
    let replacementRange: NSRange
    let selection: NSRange
    let preeditUTF8Length: Int
    let active: Bool

    init(_ value: YuStorageCompositionState) {
        revision = value.revision
        generation = value.generation
        replacementRange = NSRange(
            location: Int(value.replacement_start_utf16),
            length: Int(value.replacement_end_utf16 - value.replacement_start_utf16)
        )
        selection = NSRange(
            location: Int(value.selection_start_utf16),
            length: Int(value.selection_end_utf16 - value.selection_start_utf16)
        )
        preeditUTF8Length = Int(value.preedit_utf8_length)
        active = value.active != 0
    }
}

private struct NativeCommandResult {
    let revision: UInt64
    let selection: NSRange
    let affinity: UInt8
    let changed: Bool
    let sourceSync: UInt8
    let oldSourceRange: NSRange?
    let newSourceRange: NSRange?

    init(_ value: YuStorageCommandResult) {
        revision = value.revision
        selection = NSRange(
            location: Int(value.selection_start_utf16),
            length: Int(value.selection_end_utf16 - value.selection_start_utf16)
        )
        affinity = value.affinity
        changed = value.changed != 0
        sourceSync = value.source_sync
        oldSourceRange = sourceSync == 1
            ? NSRange(
                location: Int(value.source_start_utf16),
                length: Int(value.source_old_end_utf16 - value.source_start_utf16)
            )
            : nil
        newSourceRange = sourceSync == 1
            ? NSRange(
                location: Int(value.source_new_start_utf16),
                length: Int(value.source_new_end_utf16 - value.source_new_start_utf16)
            )
            : nil
    }
}

private final class StorageBridge {
    private var handle: OpaquePointer

    init(path: String) throws {
        var created: OpaquePointer?
        let bytes = Array(path.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            yu_storage_session_open(buffer.baseAddress, buffer.count, &created)
        }
        guard status == StorageStatus.ok, let created else {
            throw BridgeError.open(status)
        }
        handle = created
    }

    deinit {
        yu_storage_session_destroy(handle)
    }

    var path: String {
        copyBytes { output, capacity, written in
            yu_storage_session_copy_path(
                handle,
                output,
                capacity,
                written
            )
        }
    }

    var source: String {
        copyBytes { output, capacity, written in
            yu_storage_session_copy_source(
                handle,
                output,
                capacity,
                written
            )
        }
    }

    var state: NativeStorageState {
        var value = YuStorageState()
        let status = yu_storage_session_state(
            handle,
            &value
        )
        precondition(status == StorageStatus.ok, "Rust storage state query failed: \(status)")
        return NativeStorageState(value)
    }

    var selection: NativeSelection {
        var value = YuStorageSelection()
        let status = yu_storage_session_selection(handle, &value)
        precondition(status == StorageStatus.ok, "Rust selection query failed: \(status)")
        return NativeSelection(value)
    }

    var composition: NativeComposition {
        var value = YuStorageCompositionState()
        let status = yu_storage_session_composition(handle, &value)
        precondition(status == StorageStatus.ok, "Rust composition query failed: \(status)")
        return NativeComposition(value)
    }

    func setSelection(_ range: NSRange, affinity: UInt8 = 1) throws {
        let current = selection
        let status = yu_storage_session_set_selection(
            handle,
            current.revision,
            UInt64(range.location),
            UInt64(range.location + range.length),
            affinity
        )
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
    }

    func insertText(_ text: String) throws -> NativeCommandResult {
        let current = state.revision
        let bytes = Array(text.utf8)
        var result = YuStorageCommandResult()
        let status = bytes.withUnsafeBufferPointer { buffer in
            yu_storage_session_insert_text(
                handle,
                current,
                buffer.baseAddress,
                buffer.count,
                &result
            )
        }
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
        return NativeCommandResult(result)
    }

    func executeCommand(_ command: UInt8, block: UInt64 = 0) throws -> NativeCommandResult {
        var result = YuStorageCommandResult()
        let status = yu_storage_session_execute_command(handle, command, block, &result)
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
        return NativeCommandResult(result)
    }

    func commandAvailable(_ command: UInt8, block: UInt64 = 0) -> Bool {
        var available: UInt8 = 0
        let status = yu_storage_session_command_available(handle, command, block, &available)
        precondition(status == StorageStatus.ok, "Rust command availability query failed: \(status)")
        return available != 0
    }

    func routeKey(kind: UInt8, value: UInt32 = 0, modifiers: UInt8 = 0) throws -> NativeCommandResult {
        var result = YuStorageCommandResult()
        let status = yu_storage_session_route_key(handle, kind, value, modifiers, &result)
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
        return NativeCommandResult(result)
    }

    func beginComposition(replacementRange: NSRange, preedit: String, selection: NSRange) throws {
        let current = state.revision
        let bytes = Array(preedit.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            yu_storage_session_begin_composition(
                handle,
                current,
                UInt64(replacementRange.location),
                UInt64(replacementRange.location + replacementRange.length),
                buffer.baseAddress,
                buffer.count,
                UInt64(selection.location),
                UInt64(selection.location + selection.length)
            )
        }
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
    }

    func updateComposition(preedit: String, selection: NSRange) throws {
        let current = composition
        let bytes = Array(preedit.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            yu_storage_session_update_composition(
                handle,
                current.revision,
                current.generation,
                buffer.baseAddress,
                buffer.count,
                UInt64(selection.location),
                UInt64(selection.location + selection.length)
            )
        }
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
    }

    func commitComposition(_ text: String) throws {
        let current = composition
        let bytes = Array(text.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            yu_storage_session_commit_composition(
                handle,
                current.revision,
                current.generation,
                buffer.baseAddress,
                buffer.count
            )
        }
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
    }

    func cancelComposition() throws {
        let current = composition
        guard current.active else { return }
        let status = yu_storage_session_cancel_composition(
            handle,
            current.revision,
            current.generation
        )
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
    }

    func copyComposition(_ value: NativeComposition) -> String {
        guard value.active else { return "" }
        return copyBytes { output, capacity, written in
            yu_storage_session_copy_composition(
                handle,
                value.revision,
                value.generation,
                output,
                capacity,
                written
            )
        }
    }

    func copySourceRange(_ range: NSRange, revision: UInt64) -> String {
        copyBytes { output, capacity, written in
            yu_storage_session_copy_source_range(
                handle,
                revision,
                UInt64(range.location),
                UInt64(range.location + range.length),
                output,
                capacity,
                written
            )
        }
    }

    func save() throws {
        var revision: UInt64 = 0
        var bytes: Int = 0
        var changed: UInt8 = 0
        let status = yu_storage_session_save(
            handle,
            &revision,
            &bytes,
            &changed
        )
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
    }

    func reload() throws {
        var revision: UInt64 = 0
        let status = yu_storage_session_reload(
            handle,
            &revision
        )
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
    }

    func requestClose() throws -> YuStorageCloseRequest {
        var request = YuStorageCloseRequest()
        let status = yu_storage_session_request_close(
            handle,
            &request
        )
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
        return request
    }

    func cancelClose() throws {
        let status = yu_storage_session_cancel_close(
            handle
        )
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
    }

    func saveAndClose() throws {
        let status = yu_storage_session_save_close(
            handle
        )
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
    }

    func discardAndClose() throws {
        let status = yu_storage_session_discard_close(
            handle
        )
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
    }

    private func copyBytes(
        _ operation: (
            UnsafeMutablePointer<UInt8>?, Int, UnsafeMutablePointer<Int>?
        ) -> Int32
    ) -> String {
        var required = 0
        let sizeStatus = operation(nil, 0, &required)
        precondition(sizeStatus == StorageStatus.ok, "Rust storage length query failed: \(sizeStatus)")
        var bytes = Array(repeating: UInt8(0), count: required)
        let copyStatus = bytes.withUnsafeMutableBufferPointer { buffer in
            operation(buffer.baseAddress, buffer.count, &required)
        }
        precondition(copyStatus == StorageStatus.ok, "Rust storage copy failed: \(copyStatus)")
        return String(decoding: bytes, as: UTF8.self)
    }
}

/// The native source mirror is deliberately a view cache, never a second
/// document model. Rust owns canonical source, revision, selection and
/// composition generation; this TextKit object only projects those values for
/// AppKit's NSTextInputClient callbacks.
private final class DocumentTextView: NSTextView {
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
    private var nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
    private var synchronizingSelection = false
    var onDocumentChange: (() -> Void)?
    var onError: ((Error) -> Void)?

    init(bridge: StorageBridge) {
        self.bridge = bridge
        canonicalSource = bridge.source
        canonicalRevision = bridge.state.revision
        super.init(frame: .zero)
        isEditable = true
        isSelectable = true
        isRichText = false
        importsGraphics = false
        allowsUndo = false
        usesFindBar = true
        font = NSFont.systemFont(ofSize: 16)
        textColor = .labelColor
        backgroundColor = .textBackgroundColor
        minSize = NSSize(width: 0, height: 0)
        maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        isVerticallyResizable = true
        isHorizontallyResizable = false
        autoresizingMask = [.width]
        synchronizeProjection()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

    func refreshFromRust() {
        canonicalSource = bridge.source
        canonicalRevision = bridge.state.revision
        nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
        synchronizeProjection()
    }

    override func setSelectedRange(_ charRange: NSRange) {
        let range = clampedRange(charRange, length: (string as NSString).length)
        let shouldSync = !synchronizingSelection
        synchronizingSelection = true
        super.setSelectedRange(range)
        synchronizingSelection = false
        guard shouldSync, !bridge.composition.active else { return }
        do {
            try bridge.setSelection(range)
            canonicalRevision = bridge.state.revision
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
            onDocumentChange?()
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
        nativeMarkedRange
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
            do {
                if bridge.composition.active {
                    try bridge.cancelComposition()
                    nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
                    synchronizeProjection()
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

    private func routeCommand(_ command: UInt8) {
        guard !bridge.composition.active else { return }
        guard bridge.commandAvailable(command) else { return }
        do {
            apply(try bridge.executeCommand(command))
            synchronizeProjection()
            onDocumentChange?()
        } catch { onError?(error) }
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
}

private enum BridgeError: LocalizedError {
    case open(Int32)
    case operation(Int32)

    var errorDescription: String? {
        switch self {
        case .open(let status): return "无法打开 Markdown 文件（Rust status \(status)）"
        case .operation(let status): return "文档操作失败（Rust status \(status)）"
        }
    }
}

private final class DocumentViewController: NSViewController {
    private let bridge: StorageBridge
    private lazy var textView = DocumentTextView(bridge: bridge)
    private let statusLabel = NSTextField(labelWithString: "")
    private var initialState: NativeStorageState

    init(bridge: StorageBridge) {
        self.bridge = bridge
        self.initialState = bridge.state
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

    override func loadView() {
        let root = NSView()
        let scrollView = NSScrollView()
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = false
        scrollView.autohidesScrollers = true
        scrollView.translatesAutoresizingMaskIntoConstraints = false

        textView.isEditable = true
        textView.isSelectable = true
        textView.usesFindBar = true
        textView.onDocumentChange = { [weak self] in
            guard let self else { return }
            self.initialState = self.bridge.state
            self.updateStatus()
        }
        textView.onError = { [weak self] error in self?.show(error) }
        scrollView.documentView = textView

        let toolbar = NSStackView()
        toolbar.orientation = .horizontal
        toolbar.alignment = .centerY
        toolbar.spacing = 10
        toolbar.translatesAutoresizingMaskIntoConstraints = false

        let saveButton = NSButton(title: "保存", target: self, action: #selector(save))
        let reloadButton = NSButton(title: "重新加载", target: self, action: #selector(reload))
        toolbar.addArrangedSubview(saveButton)
        toolbar.addArrangedSubview(reloadButton)
        toolbar.addArrangedSubview(statusLabel)

        root.addSubview(toolbar)
        root.addSubview(scrollView)
        NSLayoutConstraint.activate([
            toolbar.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 16),
            toolbar.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -16),
            toolbar.topAnchor.constraint(equalTo: root.topAnchor, constant: 12),
            scrollView.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            scrollView.topAnchor.constraint(equalTo: toolbar.bottomAnchor, constant: 10),
            scrollView.bottomAnchor.constraint(equalTo: root.bottomAnchor),
        ])
        view = root
        updateStatus()
    }

    func refreshFromRust() {
        textView.refreshFromRust()
        initialState = bridge.state
        updateStatus()
    }

    @objc private func save() {
        do {
            try bridge.save()
            refreshFromRust()
        } catch { show(error) }
    }

    @objc private func reload() {
        do {
            try bridge.reload()
            refreshFromRust()
        } catch { show(error) }
    }

    func requestClose() -> Bool {
        do {
            let request = try bridge.requestClose()
            switch request.result {
            case 0:
                return true
            case 1:
                return prompt(request)
            default:
                return false
            }
        } catch {
            show(error)
            return false
        }
    }

    private func prompt(_ request: YuStorageCloseRequest) -> Bool {
        let alert = NSAlert()
        alert.alertStyle = request.close_state >= 3 ? .warning : .informational
        alert.messageText = request.close_state >= 3 ? "文件已被外部修改" : "保存更改？"
        alert.informativeText = request.close_state >= 3
            ? "保存会覆盖外部版本；请选择丢弃本地修改或取消。"
            : "这个 Markdown 文档有未保存更改。"
        alert.addButton(withTitle: request.close_state >= 3 ? "丢弃本地修改" : "保存")
        alert.addButton(withTitle: "取消")
        let response = alert.runModal()
        do {
            if response == .alertFirstButtonReturn {
                if request.close_state >= 3 {
                    try bridge.discardAndClose()
                } else {
                    try bridge.saveAndClose()
                }
                return true
            }
            try bridge.cancelClose()
            return false
        } catch {
            show(error)
            return false
        }
    }

    private func updateStatus() {
        let state = bridge.state
        let dirty = state.dirty ? "● 未保存" : "已保存"
        let bom = state.bom ? "UTF-8 BOM" : "UTF-8"
        statusLabel.stringValue = "\(dirty) · Rev \(state.revision) · \(state.disk.label) · \(bom)"
    }

    private func show(_ error: Error) {
        let alert = NSAlert(error: error)
        alert.runModal()
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    private var window: NSWindow?
    private var controller: DocumentViewController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let path: String
        if let argument = CommandLine.arguments.dropFirst().first, !argument.hasPrefix("-") {
            path = URL(fileURLWithPath: argument).path
        } else {
            let panel = NSOpenPanel()
            panel.allowedContentTypes = [.text, .plainText]
            panel.canChooseDirectories = false
            guard panel.runModal() == .OK, let url = panel.url else {
                NSApp.terminate(nil)
                return
            }
            path = url.path
        }

        do {
            let bridge = try StorageBridge(path: path)
            let controller = DocumentViewController(bridge: bridge)
            let window = NSWindow(
                contentViewController: controller
            )
            window.setContentSize(NSSize(width: 900, height: 620))
            window.styleMask = [.titled, .closable, .miniaturizable, .resizable]
            window.title = URL(fileURLWithPath: bridge.path).lastPathComponent
            window.center()
            window.delegate = self
            window.isReleasedWhenClosed = false
            window.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
            self.controller = controller
            self.window = window
            print("Yu document host opened path=\(bridge.path) revision=\(bridge.state.revision)")
        } catch {
            let alert = NSAlert(error: error)
            alert.runModal()
            NSApp.terminate(nil)
        }
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        controller?.requestClose() ?? true
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard let controller, !controller.requestClose() else { return .terminateNow }
        return .terminateCancel
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.setActivationPolicy(.regular)
app.delegate = delegate
app.run()
