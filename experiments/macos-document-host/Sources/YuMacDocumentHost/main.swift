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
    private let textView = NSTextView()
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

        textView.isEditable = false
        textView.isSelectable = true
        textView.isRichText = false
        textView.usesFindBar = true
        textView.font = NSFont.systemFont(ofSize: 16)
        textView.string = bridge.source
        textView.textColor = .labelColor
        textView.backgroundColor = .textBackgroundColor
        textView.minSize = NSSize(width: 0, height: 0)
        textView.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.autoresizingMask = [.width]
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
        textView.string = bridge.source
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
