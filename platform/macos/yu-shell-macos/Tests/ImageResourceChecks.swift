import Foundation

@main
struct ImageResourceChecks {
    enum BatchFailure: Error { case expected }
    static func main() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("yu-image-check-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let data = Data(base64Encoded: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a2ioAAAAASUVORK5CYII=")!
        let original = root.appendingPathComponent("图 空格%[1].png")
        try data.write(to: original)
        let document = root.appendingPathComponent("笔记.md")
        let first = try NativeImageResources.importFile(original, document: document, directory: "素材/图片")
        let second = try NativeImageResources.importFile(original, document: document, directory: "素材/图片")
        precondition(first.url != second.url, "imports must never overwrite an existing resource")
        precondition(first.pixelWidth == 1 && first.pixelHeight == 1)
        let originalBytes = try Data(contentsOf: original)
        let importedBytes = try Data(contentsOf: first.url)
        precondition(originalBytes == data && importedBytes == data)
        precondition(first.destination.hasPrefix("素材/图片/image-"))
        let reference = try NativeImageResources.referenceFile(original)
        precondition(!reference.createdFile && first.createdFile)
        precondition(reference.destination == original.standardizedFileURL.resolvingSymlinksInPath().path)
        precondition(reference.pixelWidth == first.pixelWidth && reference.pixelHeight == first.pixelHeight)
        let suiteName = "yu-image-preferences-check-" + UUID().uuidString
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let preferences = NativeWritingPreferences(defaults: defaults)
        precondition(preferences.imagePolicy == .copy && preferences.imageDirectory == "assets")
        preferences.imagePolicy = .reference
        try preferences.setImageDirectory("素材/图片")
        let reloaded = NativeWritingPreferences(defaults: UserDefaults(suiteName: suiteName)!)
        precondition(reloaded.imagePolicy == .reference && reloaded.imageDirectory == "素材/图片")
        do { try preferences.setImageDirectory("../invalid"); preconditionFailure("unsafe preference accepted") }
        catch NativeImageResources.Failure.invalidDirectory {}
        precondition(preferences.imageDirectory == "素材/图片")
        precondition(preferences.fontSize == 16 && preferences.columnWidth == 0)
        preferences.fontSize = 20
        preferences.columnWidth = 600
        precondition(reloaded.fontSize == 20 && reloaded.columnWidth == 600)
        preferences.fontSize = .nan
        preferences.columnWidth = -1
        precondition(reloaded.fontSize == 20 && reloaded.columnWidth == 600)
        precondition(preferences.spellingEnabled)
        preferences.spellingEnabled = false
        precondition(!reloaded.spellingEnabled)
        precondition(!preferences.focusMode)
        preferences.focusMode = true
        precondition(reloaded.focusMode)
        precondition(!preferences.typewriterMode)
        preferences.typewriterMode = true
        precondition(reloaded.typewriterMode)
        preferences.resetReadingPreferences()
        precondition(!reloaded.typewriterMode && !reloaded.focusMode && reloaded.spellingEnabled)
        precondition(reloaded.fontSize == 16 && reloaded.columnWidth == 0)
        preferences.resetImagePreferences()
        precondition(reloaded.imagePolicy == .copy && reloaded.imageDirectory == "assets")
        do {
            try NativeImageResources.withImports([.file(original), .data(Data("invalid".utf8))],
                document: document, directory: "batch/nested", reference: false) { _ in
                    preconditionFailure("partially validated batch was published")
                }
            preconditionFailure("invalid second image was accepted")
        } catch NativeImageResources.Failure.invalidImage {}
        precondition(!FileManager.default.fileExists(atPath: root.appendingPathComponent("batch").path))
        for referencePolicy in [false, true] {
            do {
                try NativeImageResources.withImports([.file(original), .data(data)], document: document,
                    directory: "batch/nested", reference: referencePolicy) { images in
                        precondition(images.count == 2 && images[1].image.createdFile)
                        precondition(images[0].image.createdFile != referencePolicy)
                        throw BatchFailure.expected
                    }
                preconditionFailure("source transaction should fail")
            } catch BatchFailure.expected {}
            precondition(!FileManager.default.fileExists(atPath: root.appendingPathComponent("batch").path))
            let retainedOriginal = try Data(contentsOf: original)
            precondition(retainedOriginal == data)
        }
        // An outside writer can replace a staged path, or modify its inode in
        // place. Neither file belongs to the rollback after that change.
        for replace in [false, true] {
            var changedURL: URL?
            let outsideBytes = Data(repeating: 42, count: data.count)
            do {
                try NativeImageResources.withImports([.data(data)], document: document,
                    directory: "external-\(replace)", reference: false) { images in
                        let url = images[0].image.url
                        changedURL = url
                        if replace {
                            try FileManager.default.removeItem(at: url)
                            try outsideBytes.write(to: url)
                        } else {
                            let handle = try FileHandle(forWritingTo: url)
                            try handle.write(contentsOf: outsideBytes)
                            try handle.close()
                        }
                        throw BatchFailure.expected
                    }
            } catch BatchFailure.expected {}
            let retainedReplacement = try Data(contentsOf: changedURL!)
            precondition(retainedReplacement == outsideBytes)
        }
        for directory in ["", "../outside", "/tmp/outside", "assets/../escape"] {
            do {
                _ = try NativeImageResources.importData(data, document: document, directory: directory)
                preconditionFailure("invalid directory accepted")
            } catch NativeImageResources.Failure.invalidDirectory {}
        }
        do {
            _ = try NativeImageResources.importData(Data("not an image".utf8), document: document)
            preconditionFailure("invalid image accepted")
        } catch NativeImageResources.Failure.invalidImage {}
        precondition(!FileManager.default.fileExists(atPath: root.appendingPathComponent("assets").path))
        print("Image resource checks passed: exact bytes, unique imports, Unicode paths, atomic batch rollback, original/external file protection")
    }
}
