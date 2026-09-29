import Foundation
import Security

/// Keeps Powerbox grants alive and saves bookmarks for recent documents and
/// window restoration. The Developer ID build keeps its existing file access.
final class SandboxDocumentAccess: @unchecked Sendable {
    static let shared = SandboxDocumentAccess()
    private let key = "Yu.securityScopedBookmarks"
    private var active: [String: URL] = [:]

    let enabled: Bool = {
        guard let task = SecTaskCreateFromSelf(kCFAllocatorDefault) else { return false }
        return SecTaskCopyValueForEntitlement(task,
            "com.apple.security.app-sandbox" as CFString, nil) as? Bool == true
    }()

    private init() {}

    deinit {
        for url in active.values { url.stopAccessingSecurityScopedResource() }
    }

    /// Call for URLs returned by open/save panels or the system open event.
    func rememberSelection(_ url: URL) throws {
        precondition(Thread.isMainThread)
        guard enabled else { return }
        let path = url.standardizedFileURL.path
        if active[path] == nil {
            _ = url.startAccessingSecurityScopedResource()
            active[path] = url
        }
        let bookmark = try url.bookmarkData(options: .withSecurityScope,
            includingResourceValuesForKeys: nil, relativeTo: nil)
        var saved = UserDefaults.standard.dictionary(forKey: key) as? [String: Data] ?? [:]
        saved[path] = bookmark
        UserDefaults.standard.set(saved, forKey: key)
    }

    /// Resolve a stored file or containing-folder grant before touching disk.
    /// A URL without a saved grant may still have an active system grant, for
    /// example when macOS reopens a document via an application open event.
    func accessibleURL(_ url: URL) throws -> URL {
        precondition(Thread.isMainThread)
        guard enabled else { return url }
        let path = url.standardizedFileURL.path
        if active[path] != nil { return url }
        let saved = UserDefaults.standard.dictionary(forKey: key) as? [String: Data] ?? [:]
        let scope = saved.keys.filter { path == $0 || path.hasPrefix($0 + "/") }
            .max(by: { $0.count < $1.count })
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
