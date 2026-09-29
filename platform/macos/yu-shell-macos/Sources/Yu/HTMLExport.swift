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
            guard code == YU_STORAGE_OK, written <= bytes.count else { throw failure(L10n.tr("Could not read export task status.")) }
            return try JSONDecoder().decode(Status.self, from: Data(bytes.prefix(written)))
        }
        throw failure(L10n.tr("Export status kept changing and could not be read."))
    }
    func commit(allowWarnings: Bool) throws {
        guard yu_storage_html_export_commit(handle, allowWarnings ? 1 : 0) == YU_STORAGE_OK else {
            throw failure(L10n.tr("The task is not ready, was cancelled, or resource warnings have not been confirmed."))
        }
    }
    func drawPrintPage(_ page: Int, context: CGContext) -> Bool {
        guard page >= 0, page < 1000 else { return false }
        return yu_storage_print_draw_page(handle, UInt32(page), Unmanaged.passUnretained(context).toOpaque()) == YU_STORAGE_OK
    }
    func printOutput(_ url: URL, publish: Bool) throws {
        let bytes = Array(url.path.utf8)
        let code = bytes.withUnsafeBufferPointer { yu_storage_print_output(handle, $0.baseAddress, $0.count, publish ? 1 : 0) }
        guard code == YU_STORAGE_OK else {
            fputs("Yu backend print output failure: \((try? status().message) ?? "unknown")\n", stderr)
            throw failure(L10n.tr("Print output failed."))
        }
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
    private let baseLabel = NSTextField(labelWithString: L10n.tr("Image base directory: Not selected"))
    private(set) var resourceBase: URL?
    private let preferenceKey: String
    init(untitled: Bool, preferenceKey: String = "Yu.exportHTML.currentTheme") {
        self.preferenceKey = preferenceKey
        super.init()
        style.addItems(withTitles: [L10n.tr("Light"), L10n.tr("Current editor theme")])
        style.selectItem(at: UserDefaults.standard.bool(forKey: preferenceKey) ? 1 : 0)
        let styleRow = NSStackView(views: [NSTextField(labelWithString: L10n.tr("Export style:")), style])
        styleRow.orientation = .horizontal
        view.orientation = .vertical; view.alignment = .leading; view.spacing = 10
        view.addArrangedSubview(styleRow)
        if untitled {
            let choose = NSButton(title: L10n.tr("Choose image base directory…"), target: self, action: #selector(chooseBase))
            view.addArrangedSubview(choose)
            baseLabel.lineBreakMode = .byTruncatingMiddle
            view.addArrangedSubview(baseLabel)
        }
        view.frame = NSRect(x: 0, y: 0, width: 360, height: untitled ? 112 : 40)
    }
    @objc private func chooseBase() {
        let panel = NSOpenPanel()
        panel.title = L10n.tr("Choose the base directory for relative image paths")
        panel.canChooseDirectories = true; panel.canChooseFiles = false; panel.allowsMultipleSelection = false
        if panel.runModal() == .OK, let url = panel.url {
            resourceBase = url; baseLabel.stringValue = L10n.format("Image base directory: %@", url.lastPathComponent)
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
    private let stage = NSTextField(wrappingLabelWithString: L10n.tr("Preparing export…"))
    private let detail = NSTextField(wrappingLabelWithString: L10n.tr("Export uses a fixed snapshot from confirmation time. You can continue editing the document."))
    private let indicator = NSProgressIndicator()
    private let cancelButton = NSButton(title: L10n.tr("Cancel Export"), target: nil, action: nil)
    private let revealButton = NSButton(title: L10n.tr("Show in Finder"), target: nil, action: nil)
    private var timer: Timer?
    private var askedWarnings = false
    private var committed = false
    private var closed = false
    private let onClose: () -> Void
    private(set) var isRunning = true
    private(set) var lastStatus: NativeHTMLExportTask.Status?

    private func localizedStage(for phase: String) -> String {
        switch phase {
        case "ready": return L10n.tr("Ready to export")
        case "warnings", "split": return L10n.tr("Waiting for export confirmation…")
        case "writing", "committing": return L10n.tr("Saving export…")
        case "completed", "completed_with_warnings": return L10n.tr("Export completed")
        case "cancelled": return L10n.tr("Export cancelled")
        case "failed": return L10n.tr("Export failed. No output was committed.")
        default: return L10n.tr("Preparing export…")
        }
    }

    init(task: NativeHTMLExportTask, destination: URL, owner: NSWindow, formatName: String = "HTML", onClose: @escaping () -> Void) {
        self.task = task; self.destination = destination; self.owner = owner; self.onClose = onClose
        panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 440, height: 185),
            styleMask: [.titled, .closable, .utilityWindow, .nonactivatingPanel], backing: .buffered, defer: false)
        super.init()
        panel.title = L10n.format("Export %@", formatName); panel.isReleasedWhenClosed = false; panel.delegate = self
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
            let status = try task.status(); lastStatus = status; stage.stringValue = localizedStage(for: status.phase)
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
                var summary = destination.lastPathComponent
                if let pages = status.pages, pages > 0 {
                    summary += " · " + L10n.format(pages == 1 ? "%d page" : "%d pages", pages)
                }
                summary += " · " + L10n.format(status.images == 1 ? "%d image" : "%d images", status.images)
                summary += " · " + L10n.format(status.embedded == 1 ? "%d formula/chart item" : "%d formula/chart items", status.embedded)
                if !status.warnings.isEmpty {
                    summary += "\n" + L10n.format(status.warnings.count == 1 ? "Contains %d warning; export is not fully successful." : "Contains %d warnings; export is not fully successful.", status.warnings.count)
                }
                detail.stringValue = summary
                if let sizes = status.pngSizes, !sizes.isEmpty {
                    detail.stringValue += " · " + L10n.format(sizes.count == 1 ? "%d PNG file · width %d px" : "%d PNG files · width %d px", sizes.count, sizes[0][0])
                }
                revealButton.isHidden = false; cancelButton.title = L10n.tr("Close")
                self.task = nil
            case "failed":
                fputs("Yu backend export failure: \(status.message)\n", stderr)
                finishFailure(L10n.tr("Export failed. No output was committed."))
            case "cancelled":
                finishFailure(L10n.tr("Export cancelled"))
            default: break
            }
        } catch { task.cancel(); finishFailure(error.localizedDescription) }
    }
    private func confirmSplit(_ status: NativeHTMLExportTask.Status) {
        guard let owner else { cancelAndClose(); return }
        let alert = NSAlert()
        alert.messageText = L10n.tr("Document exceeds the single-PNG budget. Export in segments?")
        let name = status.outputPath.map { URL(fileURLWithPath: $0).lastPathComponent } ?? L10n.tr("New Folder")
        alert.informativeText = L10n.format("Export %d numbered PNG files using complete row/table groups to “%@”. Existing directories are not overwritten; cancelling never leaves a partial set.", status.pngSizes?.count ?? 0, name)
        if !status.warnings.isEmpty {
            alert.informativeText += "\n" + L10n.format(status.warnings.count == 1 ? "%d resource issue requires confirmation." : "%d resource issues require confirmation.", status.warnings.count)
                + " " + L10n.tr("Some content may use placeholders or be omitted from the export.")
        }
        alert.addButton(withTitle: L10n.tr("Cancel Export")).keyEquivalent = "\u{1b}"
        alert.addButton(withTitle: status.warnings.isEmpty ? L10n.tr("Export in Segments") : L10n.tr("Export in Segments with Diagnostics"))
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
        alert.messageText = L10n.tr("Some content requires placeholders or diagnostics. Continue?")
        alert.informativeText = L10n.format(status.warnings.count == 1 ? "%d resource issue requires confirmation." : "%d resource issues require confirmation.", status.warnings.count)
            + " " + L10n.tr("Some content may use placeholders or be omitted from the export.")
            + "\n" + L10n.tr("Cancelling keeps existing output files.")
        let cancel = alert.addButton(withTitle: L10n.tr("Cancel Export"))
        cancel.keyEquivalent = "\u{1b}"
        cancel.setAccessibilityIdentifier("yu-html-warning-cancel")
        let proceed = alert.addButton(withTitle: L10n.tr("Continue Export with Placeholders or Diagnostics"))
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
        stage.stringValue = message; detail.stringValue = L10n.tr("No new output was committed. The source document and existing output remain unchanged.")
        cancelButton.title = L10n.tr("Close"); task = nil
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
