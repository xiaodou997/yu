import Foundation

@main
enum SandboxDocumentAccessChecks {
    static func main() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let suite = "Yu.sandbox-access-check." + UUID().uuidString
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let access = SandboxDocumentAccess(enabled: true, defaults: defaults)
        let target = root.appendingPathComponent("新文档.md")
        let bytes = Data("# New document\n".utf8)
        let warning = try access.saveSelectedFile(target) {
            precondition(!FileManager.default.fileExists(atPath: target.path))
            precondition(defaults.dictionary(forKey: "Yu.securityScopedBookmarks") == nil)
            try bytes.write(to: target)
        }
        precondition(warning == nil)
        let published = try Data(contentsOf: target)
        precondition(published == bytes)
        let saved = defaults.dictionary(forKey: "Yu.securityScopedBookmarks")!
        precondition(saved[target.standardizedFileURL.path] is Data)

        let failed = root.appendingPathComponent("failed.md")
        do {
            _ = try access.saveSelectedFile(failed) { throw CocoaError(.fileWriteNoPermission) }
            preconditionFailure("write failure must propagate")
        } catch let error as CocoaError {
            precondition(error.code == .fileWriteNoPermission)
        }
        precondition(!FileManager.default.fileExists(atPath: failed.path))
        precondition(defaults.dictionary(forKey: "Yu.securityScopedBookmarks")?.count == 1)

        // Simulate a file removed by another process after the write completed.
        // The bookmark error must be distinct from a failed write.
        let vanished = root.appendingPathComponent("vanished.md")
        var writeCompleted = false
        let bookmarkWarning = try access.saveSelectedFile(vanished) { writeCompleted = true }
        precondition(writeCompleted && bookmarkWarning != nil)
        precondition(defaults.dictionary(forKey: "Yu.securityScopedBookmarks")?.count == 1)
        print("Sandbox selection lifecycle: new file, failed write, and bookmark warning passed; Powerbox acceptance remains separate")
    }
}
