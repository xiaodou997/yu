import AppKit
import Foundation

struct NativeDocumentLocations {
    let root: URL
    var drafts: URL { root.appendingPathComponent("Drafts", isDirectory: true) }
    var recovery: URL { root.appendingPathComponent("Recovery", isDirectory: true) }

    static var current: Self {
        if let path = ProcessInfo.processInfo.environment["YU_DOCUMENT_STATE_DIR"], !path.isEmpty {
            return Self(root: URL(fileURLWithPath: path, isDirectory: true))
        }
        if CommandLine.arguments.contains(where: { $0.hasSuffix("-self-check") })
            || ProcessInfo.processInfo.environment["YU_VISUAL_CAPTURE_DIR"] != nil {
            return Self(root: FileManager.default.temporaryDirectory.appendingPathComponent(
                "yu-documents-self-check-\(ProcessInfo.processInfo.processIdentifier)", isDirectory: true))
        }
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support")
        return Self(root: support.appendingPathComponent("Yu/DocumentState", isDirectory: true))
    }

    func prepare() throws {
        for directory in [drafts, recovery] {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true,
                attributes: [.posixPermissions: 0o700])
        }
    }

    func newDraftURL() throws -> URL {
        try prepare()
        return drafts.appendingPathComponent(UUID().uuidString).appendingPathExtension("md")
    }

    func isDraft(_ url: URL) -> Bool {
        url.deletingLastPathComponent().standardizedFileURL.resolvingSymlinksInPath()
            == drafts.standardizedFileURL.resolvingSymlinksInPath()
    }

    func recoveryFiles() throws -> [URL] {
        try prepare()
        return try FileManager.default.contentsOfDirectory(at: recovery,
            includingPropertiesForKeys: nil, options: [.skipsHiddenFiles])
            .filter { $0.pathExtension == "yurecovery" }.sorted { $0.lastPathComponent < $1.lastPathComponent }
    }
}

/// Main-thread scheduling only. Rust remains the owner of bytes, fingerprints,
/// save boundaries, atomic writes and the checksummed recovery format.
final class NativeDocumentPersistence {
    let bridge: StorageBridge
    let locations: NativeDocumentLocations
    private(set) var isUntitled: Bool
    private(set) var needsRecoveryReview: Bool
    private(set) var notice: String?
    private(set) var suspended = false
    var onChange: (() -> Void)?
    private var scheduled: DispatchWorkItem?
    private var firstPendingChange: Date?
    private let automaticWritesEnabled: Bool

    static var autosaveEnabled: Bool {
        get { UserDefaults.standard.object(forKey: "Yu.autosaveEnabled") as? Bool ?? true }
        set { UserDefaults.standard.set(newValue, forKey: "Yu.autosaveEnabled") }
    }

    init(bridge: StorageBridge, recovered: Bool = false,
         locations: NativeDocumentLocations = .current, automaticWritesEnabled: Bool? = nil) {
        self.bridge = bridge
        self.locations = locations
        isUntitled = locations.isDraft(URL(fileURLWithPath: bridge.path))
        needsRecoveryReview = recovered
        self.automaticWritesEnabled = automaticWritesEnabled
            ?? (CommandLine.arguments.contains("--document-lifecycle-self-check")
                || (!CommandLine.arguments.contains(where: { $0.hasSuffix("-self-check") })
                    && ProcessInfo.processInfo.environment["YU_VISUAL_CAPTURE_DIR"] == nil))
        if recovered { notice = "已恢复未保存内容；确认后请保存" }
    }

    deinit { scheduled?.cancel() }

    var isPristineUntitled: Bool { isUntitled && bridge.revision == 0 && bridge.source.isEmpty }

    func documentChanged() {
        guard automaticWritesEnabled, !suspended else { return }
        scheduled?.cancel()
        let now = Date()
        if firstPendingChange == nil { firstPendingChange = now }
        // Debounce typing, but checkpoint continuous edits at least every 5s.
        let delay = min(1.0, max(0, 5.0 - now.timeIntervalSince(firstPendingChange ?? now)))
        let work = DispatchWorkItem { [weak self] in
            guard let self, !self.suspended else { return }
            self.scheduled = nil
            self.firstPendingChange = nil
            self.checkpoint(allowAutosave: Self.autosaveEnabled)
        }
        scheduled = work
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: work)
    }

    func flushRecovery() throws {
        try locations.prepare()
        if isPristineUntitled {
            try bridge.clearRecovery(in: locations.recovery)
        } else {
            try bridge.writeRecovery(in: locations.recovery)
        }
    }

    func checkpoint(allowAutosave: Bool) {
        var recoveryError: Error?
        do { try flushRecovery() } catch { recoveryError = error }
        guard allowAutosave, !isUntitled, !needsRecoveryReview, !bridge.composition.active,
              bridge.state.dirty else {
            if let recoveryError { notice = "恢复副本写入失败：\(recoveryError.localizedDescription)" }
            onChange?()
            return
        }
        do {
            try bridge.save()
            try bridge.clearRecovery(in: locations.recovery)
            notice = nil
        } catch {
            let protected = recoveryError == nil ? "更改已保留在恢复副本" : "更改仍在当前窗口，请尽快另存为"
            notice = "自动保存未完成；\(protected)"
        }
        onChange?()
    }

    /// Explicit save can still succeed when the recovery folder is unavailable.
    /// A checkpoint failure must not block saving the user's actual document.
    func save() throws {
        guard !isUntitled else { throw CocoaError(.fileWriteInvalidFileName) }
        try? flushRecovery()
        try bridge.save()
        needsRecoveryReview = false
        clearSavedRecovery()
    }

    func saveAs(_ url: URL, replaceExisting: Bool) throws {
        let oldRecord = try? bridge.recoveryURL(in: locations.recovery)
        try? flushRecovery()
        try bridge.saveAs(url, replaceExisting: replaceExisting)
        isUntitled = false
        needsRecoveryReview = false
        notice = nil
        if let oldRecord {
            do { try removeRecordIfPresent(oldRecord) }
            catch { notice = "文件已保存，但旧恢复副本清理失败：\(error.localizedDescription)" }
        }
        do { try bridge.persistTableWidths() }
        catch { notice = "文件已保存，但列宽设置写入失败：\(error.localizedDescription)" }
        onChange?()
    }

    private func clearSavedRecovery() {
        notice = nil
        do { try bridge.clearRecovery(in: locations.recovery) }
        catch { notice = "文件已保存，但恢复副本清理失败：\(error.localizedDescription)" }
        onChange?()
    }

    func suspend() {
        suspended = true
        scheduled?.cancel()
        scheduled = nil
        firstPendingChange = nil
    }

    func cancelClose() {
        bridge.abortClose()
        suspended = false
        documentChanged()
    }

    /// Called only after every quit decision has been approved, or immediately
    /// before an individually approved window closes. Cancellation never clears.
    func finalizeClose() throws {
        suspend()
        try bridge.clearRecovery(in: locations.recovery)
    }

    private func removeRecordIfPresent(_ url: URL) throws {
        do { try FileManager.default.removeItem(at: url) }
        catch let error as NSError where error.domain == NSCocoaErrorDomain && error.code == NSFileNoSuchFileError { }
    }
}
