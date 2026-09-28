import AppKit
import PDFKit

/// NSPrintPanel owns paper/printer UI. Original frozen CoreText page commands
/// draw into the system context; there is no PDF-page reinterpretation or second
/// Markdown layout. Physical paper changes use one explicit proportional fit.
@MainActor
final class NativePrintController: NSObject, NSWindowDelegate {
    private(set) static var busy = false
    private(set) var isRunning = true
    private let directory: URL
    private let prepared: URL
    private let spool: URL
    private let task: NativeHTMLExportTask
    private weak var owner: NSWindow?
    private let validateDestination: (URL) throws -> Void
    private let onFinish: (String) -> Void
    private let panel: NSPanel
    private let stage = NSTextField(wrappingLabelWithString: "准备打印内容…")
    private let detail = NSTextField(wrappingLabelWithString: "使用启动时的固定正文，您可以继续编辑。打印只在系统面板确认后提交。")
    private let cancelButton = NSButton(title: "取消打印", target: nil, action: nil)
    private var cancelled = false
    private var systemPanelActive = false
    private var warningAlert: NSAlert?
    private var execution: Task<Void, Never>?
    private let info: NSPrintInfo

    static func validateSpoolPrinters(_ names: [String]) throws {
        guard !names.isEmpty else { throw NSError(domain: "Yu.Print", code: 4, userInfo: [NSLocalizedDescriptionKey: "没有可用打印机；请从打印面板选择存储为 PDF。"] ) }
    }

    static func preparationConfig(title: String, untitled: Bool, resourceBase: URL?, info: NSPrintInfo) -> [String: Any] {
        let size = info.paperSize
        let letter = abs(min(size.width, size.height) - 612) < 2 && abs(max(size.width, size.height) - 792) < 2
        var config: [String: Any] = ["exportFormat": "pdf", "printPreparation": true,
            "title": title, "paper": letter ? "Letter" : "A4", "landscape": info.orientation == .landscape,
            "margin": 44, "pageNumbers": true, "untitled": untitled, "replaceExisting": false,
            "referenceDay": RenderCalendarContext.referenceDay(at: Date(), timeZone: .current) ?? 0]
        if let resourceBase { config["resourceBase"] = resourceBase.path }
        return config
    }

    init(bridge: StorageBridge, owner: NSWindow, title: String, untitled: Bool, resourceBase: URL?,
         validateDestination: @escaping (URL) throws -> Void, onFinish: @escaping (String) -> Void) throws {
        guard !Self.busy else { throw NSError(domain: "Yu.Print", code: 1, userInfo: [NSLocalizedDescriptionKey: "请先完成或取消当前打印。"] ) }
        self.owner = owner; self.validateDestination = validateDestination; self.onFinish = onFinish
        // Independent print info: never mutate NSPrintInfo.shared or printer setup.
        info = NSPrintInfo(dictionary: [:])
        info.leftMargin = 0; info.rightMargin = 0; info.topMargin = 0; info.bottomMargin = 0
        directory = FileManager.default.temporaryDirectory.appendingPathComponent("yu-print-" + UUID().uuidString, isDirectory: true)
        prepared = directory.appendingPathComponent("prepared.pdf")
        spool = directory.appendingPathComponent("system-output.pdf")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
        do {
            task = try bridge.beginHTMLExport(to: prepared,
                config: Self.preparationConfig(title: title, untitled: untitled, resourceBase: resourceBase, info: info))
        } catch { try? FileManager.default.removeItem(at: directory); throw error }
        panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 450, height: 165),
            styleMask: [.titled, .closable, .utilityWindow, .nonactivatingPanel], backing: .buffered, defer: false)
        super.init()
        Self.busy = true
        panel.title = "打印"; panel.isReleasedWhenClosed = false; panel.delegate = self
        panel.becomesKeyOnlyIfNeeded = true
        cancelButton.target = self; cancelButton.action = #selector(cancelPressed)
        cancelButton.setAccessibilityIdentifier("yu-print-cancel")
        stage.setAccessibilityIdentifier("yu-print-stage")
        let stack = NSStackView(views: [stage, detail, cancelButton]); stack.orientation = .vertical; stack.alignment = .leading; stack.spacing = 14
        stack.frame = NSRect(x: 20, y: 20, width: 410, height: 125); panel.contentView?.addSubview(stack)
        panel.setFrameOrigin(NSPoint(x: owner.frame.midX - 225, y: owner.frame.maxY - 240))
        owner.addChildWindow(panel, ordered: .above); panel.orderFront(nil)
        execution = Task { @MainActor [self] in await run(title: title) }
    }

    private func awaitPrepared() async throws -> NativeHTMLExportTask.Status {
        var committed = false
        while true {
            let status = try task.status(); stage.stringValue = status.message
            if ["completed", "completed_with_warnings"].contains(status.phase) { return status }
            if ["failed", "cancelled"].contains(status.phase) { throw error(status.message) }
            if cancelled { task.cancel() }
            else if status.phase == "ready" && !committed { try task.commit(allowWarnings: false); committed = true }
            else if status.phase == "warnings" && !committed {
                guard let owner else { throw error("所属文档已关闭") }
                let alert = NSAlert(); alert.messageText = "打印内容包含资源警告，是否继续？"
                alert.informativeText = status.warnings.prefix(8).joined(separator: "\n")
                alert.addButton(withTitle: "取消打印").keyEquivalent = "\u{1b}"
                alert.addButton(withTitle: "带诊断继续打印")
                warningAlert = alert
                let response = await withCheckedContinuation { continuation in
                    alert.beginSheetModal(for: owner) { continuation.resume(returning: $0) }
                }
                warningAlert = nil
                guard response == .alertSecondButtonReturn, !cancelled else { task.cancel(); throw error("已取消打印") }
                try task.commit(allowWarnings: true); committed = true
            }
            try await Task.sleep(nanoseconds: 40_000_000)
        }
    }

    /// PDFKit rewrites metadata without redrawing the pages. This is intentionally
    /// separate from CGContext/PDFPage replay, which damaged text mappings.
    /// Both inputs are private task files; final publication still uses Rust's guard.
    nonisolated static func prepareSystemPDF(_ input: URL, output: URL, title: String) throws {
        try autoreleasepool {
            let limit = 256 * 1024 * 1024
            let length = try input.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
            guard length > 0, length <= limit else { throw NSError(domain: "Yu.Print", code: 5, userInfo: [NSLocalizedDescriptionKey: "系统打印文件超过 256 MiB 预算或为空。"]) }
            let bytes = try Data(contentsOf: input, options: .mappedIfSafe)
            guard bytes.count == length, let document = PDFDocument(data: bytes), (1...1000).contains(document.pageCount) else {
                throw NSError(domain: "Yu.Print", code: 6, userInfo: [NSLocalizedDescriptionKey: "系统没有生成有效的打印文件。"])
            }
            // The system Save as PDF panel prepopulates Author with the account
            // name. Do not implicitly publish it or other environment metadata.
            document.documentAttributes = [PDFDocumentAttribute.titleAttribute: title, PDFDocumentAttribute.creatorAttribute: "Yu"]
            guard let data = document.dataRepresentation(), !data.isEmpty, data.count <= limit,
                  let checked = PDFDocument(data: data), checked.pageCount == document.pageCount,
                  checked.documentAttributes?[PDFDocumentAttribute.authorAttribute] == nil else {
                throw NSError(domain: "Yu.Print", code: 7, userInfo: [NSLocalizedDescriptionKey: "系统打印文件元数据检查失败，未提交文件。"])
            }
            try data.write(to: output, options: .withoutOverwriting)
        }
    }

    private func run(title: String) async {
        var message = "已取消打印"
        defer {
            isRunning = false; Self.busy = false; execution = nil
            owner?.removeChildWindow(panel); panel.delegate = nil; panel.close()
            try? FileManager.default.removeItem(at: directory)
            onFinish(message)
        }
        do {
            let preparedStatus = try await awaitPrepared()
            guard !cancelled, let owner else { return }
            guard let document = PDFDocument(url: prepared), (1...1000).contains(document.pageCount),
                  let firstPage = document.page(at: 0) else { throw error("无法读取打印页面。") }
            let printView = NativePrintedPagesView(task: task, sourceSize: firstPage.bounds(for: .mediaBox).size, pages: document.pageCount)
            guard NSPrintOperation.current == nil else { throw error("已有系统打印操作，请先完成或取消。") }
            let operation = printView.makeOperation(info: info)
            defer { operation.cleanUp() }
            operation.jobTitle = title
            let systemPanel = NativeGuardedPrintPanel()
            operation.printPanel = systemPanel
            systemPanel.options = [.showsCopies, .showsPageRange, .showsPaperSize, .showsOrientation, .showsPreview]
            let accessory = NativePrintExplanation()
            operation.printPanel.addAccessoryController(accessory)
            owner.removeChildWindow(panel)
            panel.orderOut(nil)
            systemPanelActive = true
            // NSPrintOperation owns the native panel lifecycle. The public panel
            // completion hook checks destinations before it authorizes delivery.
            var saving = false
            systemPanel.approval = { [self] approvedInfo in
                guard !cancelled else { throw error("已取消打印") }
                try printView.validatePaper()
                saving = approvedInfo.jobDisposition == .save
                if saving {
                    guard let destination = approvedInfo.dictionary()[NSPrintInfo.AttributeKey.jobSavingURL] as? URL else { throw error("系统未提供打印文件位置。") }
                    try validateDestination(destination)
                    try task.printOutput(destination, publish: false)
                    approvedInfo.dictionary()[NSPrintInfo.AttributeKey.jobSavingURL] = spool
                } else if approvedInfo.jobDisposition == .spool {
                    try Self.validateSpoolPrinters(NSPrinter.printerNames)
                } else { throw error("请选择打印或存储为 PDF；其他系统 PDF 工作流程暂不支持。") }
            }
            operation.showsPrintPanel = true
            operation.showsProgressPanel = true
            let completed: Bool = await withCheckedContinuation { continuation in
                // The standard operation owns its application-modal panel and
                // current-operation association. Enter from the main run loop
                // only after the preparation actor task has suspended.
                DispatchQueue.main.async { continuation.resume(returning: operation.run()) }
            }
            systemPanelActive = false
            if let failure = systemPanel.failure { throw failure }
            guard completed, !cancelled else { return }
            guard !printView.drawingFailed else { throw error("系统打印未能完整绘制；文档未改变。") }
            if saving {
                owner.addChildWindow(panel, ordered: .above)
                panel.orderFront(nil); stage.stringValue = "检查系统打印文件…"
                let clean = directory.appendingPathComponent("checked-output.pdf"), input = spool
                try await Task.detached(priority: .userInitiated) { try Self.prepareSystemPDF(input, output: clean, title: title) }.value
                guard !cancelled else { task.cancel(); return }
                try task.printOutput(clean, publish: true)
                while true {
                    let status = try task.status(); stage.stringValue = status.message
                    if ["completed", "completed_with_warnings"].contains(status.phase) { message = status.message; break }
                    if ["failed", "cancelled"].contains(status.phase) { throw error(status.message) }
                    if cancelled { task.cancel() }
                    try await Task.sleep(nanoseconds: 40_000_000)
                }
            } else { message = "已交给系统打印；设备出纸状态请查看系统队列。" }
            if !preparedStatus.warnings.isEmpty { message += "（包含已确认的资源诊断）" }
        } catch {
            message = cancelled ? "已取消打印" : error.localizedDescription
        }
    }
    private func error(_ message: String) -> Error { NSError(domain: "Yu.Print", code: 2, userInfo: [NSLocalizedDescriptionKey: message]) }
    @objc private func cancelPressed() { cancelAndClose() }
    func cancelAndClose() {
        guard isRunning else { return }
        cancelled = true; task.cancel(); panel.orderOut(nil)
        if let warningAlert { owner?.endSheet(warningAlert.window, returnCode: .abort) }
        if systemPanelActive { NSApp.abortModal() }
    }
    func windowWillClose(_ notification: Notification) { cancelAndClose() }
}

/// A public NSPrintPanel completion hook, not a replacement printer UI.
/// The system operation invokes it; no bytes can be delivered to an approved
/// file path before that path has been checked and redirected to owned staging.
@MainActor
private final class NativeGuardedPrintPanel: NSPrintPanel {
    var approval: ((NSPrintInfo) throws -> Void)?
    private(set) var failure: Error?
    private func checked(_ result: NSPrintPanel.Result, info: NSPrintInfo) -> NSPrintPanel.Result {
        guard result == .printed else { return result }
        do { try approval?(info); return result }
        catch { failure = error; return .cancelled }
    }
    override func beginSheet(using printInfo: NSPrintInfo, on parentWindow: NSWindow,
                             completionHandler: ((NSPrintPanel.Result) -> Void)? = nil) {
        super.beginSheet(using: printInfo, on: parentWindow) { [self] result in
            completionHandler?(checked(result, info: printInfo))
        }
    }
    override func runModal(with printInfo: NSPrintInfo) -> Int {
        let result = super.runModal(with: printInfo)
        return checked(result == 1 ? .printed : .cancelled, info: printInfo).rawValue
    }
}

/// AppKit custom pagination only selects existing pages. This view neither
/// parses Markdown nor shapes text, and it never uses a bitmap page snapshot.
@MainActor
final class NativePrintedPagesView: NSView {
    let task: NativeHTMLExportTask
    let sourceSize: NSSize
    let pages: Int
    weak var operation: NSPrintOperation?
    private(set) var drawingFailed = false
    private var requestedPage = 1
    init(task: NativeHTMLExportTask, sourceSize: NSSize, pages: Int) {
        self.task = task; self.sourceSize = sourceSize; self.pages = pages
        super.init(frame: NSRect(origin: .zero, size: sourceSize))
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }
    func makeOperation(info: NSPrintInfo) -> NSPrintOperation {
        // Configure once. Pagination callbacks also run while the native panel
        // edits its settings; mutating printInfo there can reset panel choices.
        info.scalingFactor = 1
        info.horizontalPagination = .clip; info.verticalPagination = .clip
        info.isHorizontallyCentered = false; info.isVerticallyCentered = false
        info.leftMargin = 0; info.rightMargin = 0; info.topMargin = 0; info.bottomMargin = 0
        let op = NSPrintOperation(view: self, printInfo: info)
        op.canSpawnSeparateThread = false // native retained page objects are serialized
        operation = op
        return op
    }
    private var imageable: NSRect {
        guard let info = operation?.printInfo else { return bounds }
        return info.imageablePageBounds.intersection(NSRect(origin: .zero, size: info.paperSize))
    }
    func validatePaper() throws {
        let box = imageable
        guard box.width.isFinite, box.height.isFinite, box.width > 0, box.height > 0,
              sourceSize.width > 0, sourceSize.height > 0,
              min(box.width / sourceSize.width, box.height / sourceSize.height) >= 0.25 else {
            throw NSError(domain: "Yu.Print", code: 3, userInfo: [NSLocalizedDescriptionKey: "可打印区域过小，整页需要缩小至 25% 以下；请调整纸张或方向。"])
        }
        drawingFailed = false
    }
    override func knowsPageRange(_ range: NSRangePointer) -> Bool {
        range.pointee = NSRange(location: 1, length: pages)
        let box = imageable
        if box.width > 0 && box.height > 0 { setFrameSize(box.size) }
        return true
    }
    override func rectForPage(_ page: Int) -> NSRect {
        requestedPage = page
        return NSRect(origin: .zero, size: imageable.size)
    }
    override func locationOfPrintRect(_ rect: NSRect) -> NSPoint { imageable.origin }
    override func draw(_ dirtyRect: NSRect) {
        guard let context = NSGraphicsContext.current?.cgContext else { drawingFailed = true; return }
        let box = imageable
        let scale = min(1, min(box.width / sourceSize.width, box.height / sourceSize.height))
        guard scale.isFinite, scale >= 0.25 else { drawingFailed = true; return }
        let current = operation?.currentPage ?? requestedPage
        let page = current > 0 ? current : requestedPage
        context.saveGState()
        context.translateBy(x: (box.width - sourceSize.width * scale) / 2, y: (box.height - sourceSize.height * scale) / 2)
        context.scaleBy(x: scale, y: scale)
        if !task.drawPrintPage(page - 1, context: context) { drawingFailed = true }
        context.restoreGState()
    }
}

@MainActor
private final class NativePrintExplanation: NSViewController, NSPrintPanelAccessorizing {
    override func loadView() {
        let label = NSTextField(wrappingLabelWithString: "使用与 PDF 相同的固定分页及浅色纸张。更改打印纸张、方向或可打印区域时，整页等比缩小，不重新分页；预览即系统输出内容。")
        label.frame = NSRect(x: 0, y: 0, width: 300, height: 75); view = label
    }
    func localizedSummaryItems() -> [[NSPrintPanel.AccessorySummaryKey: String]] {
        [[.itemName: "页面适配", .itemDescription: "固定分页；整页等比缩小至纸张"]]
    }
}
