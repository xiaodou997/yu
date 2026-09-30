import AppKit
import Foundation
import Security

/// Keeps Powerbox grants alive and saves bookmarks for recent documents and
/// window restoration. The Developer ID build keeps its existing file access.
final class SandboxDocumentAccess: @unchecked Sendable {
    static let shared = SandboxDocumentAccess()
    private let key = "Yu.securityScopedBookmarks"
    private var active: [String: URL] = [:]
    private let defaults: UserDefaults

    private static var isSandboxEnabled: Bool {
        guard let task = SecTaskCreateFromSelf(kCFAllocatorDefault) else { return false }
        return SecTaskCopyValueForEntitlement(task,
            "com.apple.security.app-sandbox" as CFString, nil) as? Bool == true
    }
    let enabled: Bool

    init(enabled: Bool? = nil, defaults: UserDefaults = .standard) {
        self.enabled = enabled ?? Self.isSandboxEnabled
        self.defaults = defaults
    }

    deinit {
        for url in active.values { url.stopAccessingSecurityScopedResource() }
    }

    private func retainSelection(_ url: URL) {
        precondition(Thread.isMainThread)
        guard enabled else { return }
        let path = url.standardizedFileURL.path
        if active[path] == nil {
            _ = url.startAccessingSecurityScopedResource()
            active[path] = url
        }
    }

    /// Call for existing URLs returned by open panels or the system open event.
    func rememberSelection(_ url: URL) throws {
        retainSelection(url)
        guard enabled else { return }
        let path = url.standardizedFileURL.path
        let bookmark = try url.bookmarkData(options: .withSecurityScope,
            includingResourceValuesForKeys: nil, relativeTo: nil)
        var saved = defaults.dictionary(forKey: key) as? [String: Data] ?? [:]
        saved[path] = bookmark
        defaults.set(saved, forKey: key)
    }

    /// NSSavePanel may return a file that does not exist yet. Keep its current
    /// grant alive while writing, and create the persistent bookmark afterward.
    /// A bookmark failure is returned separately: the file was already saved.
    func saveSelectedFile(_ url: URL, write: () throws -> Void) throws -> Error? {
        retainSelection(url)
        try write()
        do {
            try rememberSelection(url)
            return nil
        } catch {
            return error
        }
    }

    /// Directory consent is needed for sibling images and multi-file output.
    /// A normal text-only document continues to use its selected-file grant.
    func ensureDirectoryAccess(_ directory: URL, writing: Bool, message: String) throws -> Bool {
        precondition(Thread.isMainThread)
        guard enabled else { return true }
        _ = try accessibleURL(directory)
        let fm = FileManager.default
        if (try? fm.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)) != nil,
           !writing || fm.isWritableFile(atPath: directory.path) { return true }
        NSApp.activate(ignoringOtherApps: true)
        let panel = NSOpenPanel()
        panel.title = L10n.tr("Allow Folder Access")
        panel.message = message
        panel.prompt = L10n.tr("Allow Access")
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.directoryURL = directory
        guard panel.runModal() == .OK, let selected = panel.url else { return false }
        let requested = directory.standardizedFileURL.resolvingSymlinksInPath().path
        let granted = selected.standardizedFileURL.resolvingSymlinksInPath().path
        guard requested == granted || requested.hasPrefix(granted == "/" ? "/" : granted + "/") else {
            throw NSError(domain: "Yu.Sandbox", code: 1, userInfo: [NSLocalizedDescriptionKey:
                L10n.tr("Select the requested folder or a folder containing it.")])
        }
        try rememberSelection(selected)
        return true
    }

    func ensureImageAccess(_ paths: [String]) throws -> Bool {
        guard enabled else { return true }
        for path in paths {
            let url = try accessibleURL(URL(fileURLWithPath: path))
            if FileManager.default.isReadableFile(atPath: url.path) { continue }
            if try !ensureDirectoryAccess(url.deletingLastPathComponent(), writing: false,
                message: L10n.tr("Allow access to this folder to load the document’s local images. Cancelling keeps the document open without those images.")) { return false }
        }
        return true
    }

    /// Resolve a stored file or containing-folder grant before touching disk.
    /// A URL without a saved grant may still have an active system grant, for
    /// example when macOS reopens a document via an application open event.
    func accessibleURL(_ url: URL) throws -> URL {
        precondition(Thread.isMainThread)
        guard enabled else { return url }
        let path = url.standardizedFileURL.path
        let saved = defaults.dictionary(forKey: key) as? [String: Data] ?? [:]
        let scope = saved.keys.filter { path == $0 || path.hasPrefix($0 == "/" ? "/" : $0 + "/") }
            .min(by: { $0.count < $1.count })
        guard let scope, let data = saved[scope] else { return url }
        if active[scope] != nil { return url }
        var stale = false
        let granted = try URL(resolvingBookmarkData: data, options: .withSecurityScope,
            relativeTo: nil, bookmarkDataIsStale: &stale)
        guard granted.startAccessingSecurityScopedResource() else {
            throw CocoaError(.fileReadNoPermission)
        }
        active[scope] = granted
        if stale { try rememberSelection(granted) }
        return scope == path ? granted : url
    }
}
