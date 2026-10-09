import AppKit

/// Only folder paths are persisted. Document recents remain owned by AppKit;
/// sandbox bookmarks remain owned by SandboxDocumentAccess.
final class NavigationHistory {
    static let shared = NavigationHistory(enabled: !CommandLine.arguments.contains { $0.contains("self-check") }
        && ProcessInfo.processInfo.environment["YU_VISUAL_CAPTURE_DIR"] == nil)
    private let defaults: UserDefaults
    private let enabled: Bool
    private let foldersKey = "Yu.navigation.recentFolders.v1"
    private let lastKey = "Yu.navigation.lastFolder.v1"
    static let limit = 12

    init(defaults: UserDefaults = .standard, enabled: Bool = true) {
        self.defaults = defaults
        self.enabled = enabled
    }

    var folders: [URL] {
        guard enabled else { return [] }
        var seen = Set<String>()
        return (defaults.stringArray(forKey: foldersKey) ?? []).prefix(64).compactMap { path in
            guard path.hasPrefix("/"), path.utf8.count <= 16_384,
                  !path.contains("\0"), seen.insert(path).inserted else { return nil }
            return URL(fileURLWithPath: path, isDirectory: true)
        }.prefix(Self.limit).map { $0 }
    }

    func noteFolder(_ url: URL) {
        guard enabled else { return }
        let url = url.standardizedFileURL.resolvingSymlinksInPath()
        guard url.isFileURL, (try? url.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true else { return }
        let paths = ([url] + folders.filter { $0.path != url.path }).prefix(Self.limit).map(\.path)
        defaults.set(paths, forKey: foldersKey)
        defaults.set(url.path, forKey: lastKey)
    }

    func restoreFolder() -> URL? {
        guard enabled, let path = defaults.string(forKey: lastKey), path.hasPrefix("/"),
              path.utf8.count <= 16_384, !path.contains("\0"),
              let url = try? SandboxDocumentAccess.shared.accessibleURL(URL(fileURLWithPath: path, isDirectory: true)),
              (try? url.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true else { return nil }
        return url
    }

    func useDocumentFolder() { if enabled { defaults.removeObject(forKey: lastKey) } }
    func clearFolders() {
        guard enabled else { return }
        defaults.removeObject(forKey: foldersKey)
        defaults.removeObject(forKey: lastKey)
    }

    static func checkForSelfCheck() throws {
        let suite = "Yu.NavigationHistory.Check.\(UUID().uuidString)"
        guard let defaults = UserDefaults(suiteName: suite) else { throw CocoaError(.coderInvalidValue) }
        defer { defaults.removePersistentDomain(forName: suite) }
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(suite)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let history = NavigationHistory(defaults: defaults)
        for number in 0..<15 {
            let directory = root.appendingPathComponent("羽🙂-\(number)")
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            history.noteFolder(directory)
        }
        precondition(history.folders.count == Self.limit)
        let first = history.folders[0]
        history.noteFolder(first.appendingPathComponent("."))
        precondition(history.folders.count == Self.limit && history.folders[0].path == first.path)
        let reopened = NavigationHistory(defaults: defaults)
        precondition(reopened.restoreFolder()?.standardizedFileURL.resolvingSymlinksInPath() == first.standardizedFileURL.resolvingSymlinksInPath())
        try FileManager.default.removeItem(at: first)
        precondition(reopened.restoreFolder() == nil && reopened.folders.count == Self.limit)
        reopened.useDocumentFolder()
        precondition(reopened.restoreFolder() == nil && reopened.folders.count == Self.limit)
        reopened.clearFolders()
        precondition(reopened.folders.isEmpty && reopened.restoreFolder() == nil)
        print("Yu navigation history self-check: bounded recents, deduplication, restoration, missing-folder fallback and clearing passed")
    }
}
