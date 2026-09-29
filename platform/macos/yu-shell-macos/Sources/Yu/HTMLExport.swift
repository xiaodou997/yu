import AppKit
import Foundation
import UniformTypeIdentifiers
import YuStorageFFI

/// An owned Rust task, distinct from the mutable document session. All calls
/// use the native owner thread; Rust performs conversion and I/O on workers.
final class NativeHTMLExportTask {
    struct Status: Decodable {
        let phase: String
        let message: String
        let revision: UInt64
        let warnings: [String]
        let images: Int
        let embedded: Int
        let pages: Int?
        let pngSizes: [[Int]]?
        let outputPath: String?
    }
    private let handle: OpaquePointer
    init(handle: OpaquePointer) { self.handle = handle }
    deinit { yu_storage_html_export_destroy(handle) }
    func status() throws -> Status {
        var capacity = 4096
        for _ in 0..<4 {
            var bytes = [UInt8](repeating: 0, count: capacity)
            var written = 0
            let code = bytes.withUnsafeMutableBufferPointer {
                yu_storage_html_export_copy_status(handle, $0.baseAddress, $0.count, &written)
            }
            if code == YU_STORAGE_BUFFER_TOO_SMALL { capacity = written; continue }
            guard code == YU_STORAGE_OK, written <= bytes.count else { throw failure("无法读取导出任务状态。") }
            return try JSONDecoder().decode(Status.self, from: Data(bytes.prefix(written)))
        }
        throw failure("导出状态持续变化，读取失败。")
    }
    func commit(allowWarnings: Bool) throws {
        guard yu_storage_html_export_commit(handle, allowWarnings ? 1 : 0) == YU_STORAGE_OK else {
            throw failure("任务未准备完成、已取消，或尚未确认资源警告。")
        }
    }
    func drawPrintPage(_ page: Int, context: CGContext) -> Bool {
        guard page >= 0, page < 1000 else { return false }
        return yu_storage_print_draw_page(handle, UInt32(page), Unmanaged.passUnretained(context).toOpaque()) == YU_STORAGE_OK
    }
    func printOutput(_ url: URL, publish: Bool) throws {
        let bytes = Array(url.path.utf8)
        let code = bytes.withUnsafeBufferPointer { yu_storage_print_output(handle, $0.baseAddress, $0.count, publish ? 1 : 0) }
        guard code == YU_STORAGE_OK else { throw failure((try? status().message) ?? "系统打印输出失败。") }
    }
    func cancel() { yu_storage_html_export_cancel(handle) }
    private func failure(_ message: String) -> Error {
        NSError(domain: "Yu.Export", code: 1, userInfo: [NSLocalizedDescriptionKey: message])
    }
}

/// Save-panel accessory contains only implemented HTML choices. Export options
/// have their own preference key and never change the editor's reading theme.
@MainActor
final class NativeHTMLExportOptions: NSObject {
    let view = NSStackView()
    private let style = NSPopUpButton()
    private let baseLabel = NSTextField(labelWithString: "图片基准目录：未选择")
    private(set) var resourceBase: URL?
    private let preferenceKey: String
    init(untitled: Bool, preferenceKey: String = "Yu.exportHTML.currentTheme") {
        self.preferenceKey = preferenceKey
        super.init()
        style.addItems(withTitles: ["浅色", "当前正文主题"])
        style.selectItem(at: UserDefaults.standard.bool(forKey: preferenceKey) ? 1 : 0)
        let styleRow = NSStackView(views: [NSTextField(labelWithString: "导出样式："), style])
        styleRow.orientation = .horizontal
        view.orientation = .vertical; view.alignment = .leading; view.spacing = 10
        view.addArrangedSubview(styleRow)
        if untitled {
            let choose = NSButton(title: "选择图片基准目录…", target: self, action: #selector(chooseBase))
            view.addArrangedSubview(choose)
            baseLabel.lineBreakMode = .byTruncatingMiddle
            view.addArrangedSubview(baseLabel)
        }
        view.frame = NSRect(x: 0, y: 0, width: 360, height: untitled ? 112 : 40)
    }
    @objc private func chooseBase() {
        let panel = NSOpenPanel()
        panel.title = "选择相对图片路径的基准目录"
        panel.canChooseDirectories = true; panel.canChooseFiles = false; panel.allowsMultipleSelection = false
        if panel.runModal() == .OK, let url = panel.url {
            resourceBase = url; baseLabel.stringValue = "图片基准目录：\(url.lastPathComponent)"
        }
    }
    func config(title: String, untitled: Bool, appearance: NSAppearance) -> [String: Any] {
        let current = style.indexOfSelectedItem == 1
        let dark = current && appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
        let resolved = current ? NativeTheme.resolved(dark: dark) : UInt8(YU_STORAGE_APPEARANCE_LIGHT)
        let theme = NativeTheme.spec(resolved: resolved)
        let reading = NativeWritingPreferences.shared
        let width = reading.columnWidth > 0 ? reading.columnWidth : Double(theme.column_width)
        UserDefaults.standard.set(current, forKey: preferenceKey)
        var result: [String: Any] = ["title": title, "foreground": theme.text,
            "background": theme.background, "link": theme.link,
            "fontSize": reading.fontSize, "width": Int(min(1200, max(360, width))),
            "dark": theme.background >> 24 < 128,
            "referenceDay": RenderCalendarContext.referenceDay(at: Date(), timeZone: .current) ?? 0,
            "untitled": untitled, "replaceExisting": true]
        if let resourceBase { result["resourceBase"] = resourceBase.path }
        return result
    }
}

/// Modeless progress keeps the document editable while the frozen job runs.
@MainActor
final class NativeHTMLExportController: NSObject, NSWindowDelegate {
    private var task: NativeHTMLExportTask?
    private weak var owner: NSWindow?
    private var destination: URL
    private let panel: NSPanel
    private let stage = NSTextField(wrappingLabelWithString: "准备导出…")
    private let detail = NSTextField(wrappingLabelWithString: "导出使用确认时的固定内容，您可以继续编辑文档。")
    private let indicator = NSProgressIndicator()
    private let cancelButton = NSButton(title: "取消导出", target: nil, action: nil)
    private let revealButton = NSButton(title: "在访达中显示", target: nil, action: nil)
    private var timer: Timer?
    private var askedWarnings = false
    private var committed = false
    private var closed = false
    private let onClose: () -> Void
    private(set) var isRunning = true
    private(set) var lastStatus: NativeHTMLExportTask.Status?

    init(task: NativeHTMLExportTask, destination: URL, owner: NSWindow, formatName: String = "HTML", onClose: @escaping () -> Void) {
        self.task = task; self.destination = destination; self.owner = owner; self.onClose = onClose
        panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 440, height: 185),
            styleMask: [.titled, .closable, .utilityWindow, .nonactivatingPanel], backing: .buffered, defer: false)
        super.init()
        panel.title = "导出 \(formatName)"; panel.isReleasedWhenClosed = false; panel.delegate = self
        panel.becomesKeyOnlyIfNeeded = true
        stage.font = .boldSystemFont(ofSize: 14)
        indicator.style = .bar; indicator.isIndeterminate = true; indicator.startAnimation(nil)
        cancelButton.target = self; cancelButton.action = #selector(cancelPressed)
        revealButton.target = self; revealButton.action = #selector(reveal); revealButton.isHidden = true
        let buttons = NSStackView(views: [revealButton, cancelButton]); buttons.orientation = .horizontal
        let stack = NSStackView(views: [stage, detail, indicator, buttons])
        stack.orientation = .vertical; stack.alignment = .leading; stack.spacing = 12
        stack.translatesAutoresizingMaskIntoConstraints = false
        panel.contentView?.addSubview(stack)
        if let content = panel.contentView {
            NSLayoutConstraint.activate([stack.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 20),
                stack.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -20),
                stack.topAnchor.constraint(equalTo: content.topAnchor, constant: 20),
                indicator.widthAnchor.constraint(equalTo: stack.widthAnchor)])
        }
        panel.setFrameOrigin(NSPoint(x: owner.frame.midX - 220, y: owner.frame.maxY - 250))
        owner.addChildWindow(panel, ordered: .above); panel.orderFront(nil)
        timer = Timer.scheduledTimer(withTimeInterval: 0.15, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.poll() }
        }
    }
    private func poll() {
        guard !closed, let task else { return }
        do {
            let status = try task.status(); lastStatus = status; stage.stringValue = status.message
            switch status.phase {
            case "ready":
                if !committed { committed = true; try task.commit(allowWarnings: false) }
            case "split":
                if !askedWarnings { askedWarnings = true; confirmSplit(status) }
            case "warnings":
                if !askedWarnings { askedWarnings = true; confirmWarnings(status) }
            case "completed", "completed_with_warnings":
                if let output = status.outputPath { destination = URL(fileURLWithPath: output) }
                isRunning = false; timer?.invalidate(); timer = nil
                indicator.stopAnimation(nil); indicator.isHidden = true
                detail.stringValue = "\(destination.lastPathComponent)" + ((status.pages ?? 0) > 0 ? " · \(status.pages ?? 0) 页" : "") + " · \(status.images) 张图片 · \(status.embedded) 项公式/图表" + (status.warnings.isEmpty ? "" : "\n含 \(status.warnings.count) 项警告，不是完整成功。")
                if let sizes = status.pngSizes, !sizes.isEmpty {
                    detail.stringValue += " · \(sizes.count) 张 PNG · 宽 \(sizes[0][0]) px"
                }
                revealButton.isHidden = false; cancelButton.title = "关闭"
                self.task = nil
            case "failed", "cancelled":
                finishFailure(status.message)
            default: break
            }
        } catch { task.cancel(); finishFailure(error.localizedDescription) }
    }
    private func confirmSplit(_ status: NativeHTMLExportTask.Status) {
        guard let owner else { cancelAndClose(); return }
        let alert = NSAlert()
        alert.messageText = "文档超过单张 PNG 预算，是否分段导出？"
        let name = status.outputPath.map { URL(fileURLWithPath: $0).lastPathComponent } ?? "新目录"
        alert.informativeText = "将按完整行／表格合并组输出 \(status.pngSizes?.count ?? 0) 张编号 PNG，保存到“\(name)”。不覆盖已有目录；取消不会产生半套输出。" + (status.warnings.isEmpty ? "" : "\n另有资源诊断：\n" + status.warnings.prefix(8).joined(separator: "\n"))
        alert.addButton(withTitle: "取消导出").keyEquivalent = "\u{1b}"
        alert.addButton(withTitle: status.warnings.isEmpty ? "分段导出" : "带诊断分段导出")
        alert.beginSheetModal(for: owner) { [weak self] response in
            guard let self, !self.closed, let task = self.task else { return }
            if response == .alertSecondButtonReturn {
                do { self.committed = true; try task.commit(allowWarnings: !status.warnings.isEmpty) }
                catch { task.cancel(); self.finishFailure(error.localizedDescription) }
            } else { self.cancelAndClose() }
        }
    }
    private func confirmWarnings(_ status: NativeHTMLExportTask.Status) {
        guard let owner else { cancelAndClose(); return }
        let alert = NSAlert(); alert.alertStyle = .warning
        alert.messageText = "部分内容需要占位或诊断，是否继续？"
        alert.informativeText = status.warnings.prefix(8).joined(separator: "\n") + (status.warnings.count > 8 ? "\n另有 \(status.warnings.count - 8) 项警告。" : "") + "\n取消将保留已有输出文件。"
        let cancel = alert.addButton(withTitle: "取消导出")
        cancel.keyEquivalent = "\u{1b}"
        cancel.setAccessibilityIdentifier("yu-html-warning-cancel")
        let proceed = alert.addButton(withTitle: "带占位或诊断继续导出")
        proceed.setAccessibilityIdentifier("yu-html-warning-proceed")
        alert.beginSheetModal(for: owner) { [weak self] response in
            guard let self, !self.closed, self.isRunning, let task = self.task else { return }
            if response == .alertSecondButtonReturn {
                do { self.committed = true; try task.commit(allowWarnings: true) }
                catch { self.finishFailure(error.localizedDescription) }
            } else { self.cancelAndClose() }
        }
    }
    private func finishFailure(_ message: String) {
        isRunning = false; timer?.invalidate(); timer = nil
        indicator.stopAnimation(nil); indicator.isHidden = true
        stage.stringValue = message; detail.stringValue = "没有提交新的输出文件。原文及已有输出保持不变。"
        cancelButton.title = "关闭"; task = nil
    }
    @objc private func reveal() { NSWorkspace.shared.activateFileViewerSelecting([destination]) }
    @objc private func cancelPressed() { cancelAndClose() }
    func cancelAndClose() {
        guard !closed else { return }; closed = true
        task?.cancel(); task = nil; timer?.invalidate(); timer = nil
        owner?.removeChildWindow(panel); panel.close(); onClose()
    }
    func windowWillClose(_ notification: Notification) { cancelAndClose() }
}
