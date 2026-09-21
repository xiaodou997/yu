import Foundation
import ImageIO
import UniformTypeIdentifiers
import Darwin
import CryptoKit

/// Filesystem-only image import. Markdown serialization and edits stay in Rust.
/// Imported files survive undo: another document may already reference them.
struct NativeImageResources {
    enum Input { case file(URL), data(Data) }
    struct Imported {
        let url: URL
        let destination: String
        let pixelWidth: Int
        let pixelHeight: Int
        let createdFile: Bool
        fileprivate let ownership: Ownership?
        fileprivate let createdDirectories: [URL]
    }
    fileprivate struct Ownership {
        let device: dev_t
        let inode: ino_t
        let size: Int
        let digest: SHA256.Digest
    }
    struct Prepared {
        let image: Imported
        let alternative: String
    }

    static func validate(_ inputs: [Input]) throws {
        for input in inputs {
            switch input {
            case .file(let url):
                do { _ = try inspect(readFile(url)) }
                catch { throw FileFailure(name: url.lastPathComponent, reason: error.localizedDescription) }
            case .data(let data): _ = try inspect(data)
            }
        }
    }

    /// Publish only after every file has been prepared. An unsuccessful source
    /// transaction discards owned copies; referenced originals never participate.
    static func withImports(_ inputs: [Input], document: URL, directory: String,
                            reference: Bool, publish: ([Prepared]) throws -> Void) throws {
        var prepared: [Prepared] = []
        do {
            for input in inputs {
                switch input {
                case .file(let url):
                    let image = try reference ? referenceFile(url) : importFile(url, document: document, directory: directory)
                    prepared.append(Prepared(image: image, alternative: url.deletingPathExtension().lastPathComponent))
                case .data(let data):
                    prepared.append(Prepared(image: try importData(data, document: document, directory: directory), alternative: "图片"))
                }
            }
            try publish(prepared)
        } catch {
            for item in prepared.reversed() { discardUnpublished(item.image) }
            throw error
        }
    }

    /// Do not remove an externally replaced or modified file during rollback.
    static func discardUnpublished(_ image: Imported) {
        guard image.createdFile, let ownership = image.ownership else { return }
        let descriptor = open(image.url.path, O_RDONLY | O_NOFOLLOW)
        if descriptor >= 0 {
            let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
            defer { try? handle.close() }
            var info = stat()
            if fstat(descriptor, &info) == 0,
               info.st_dev == ownership.device, info.st_ino == ownership.inode,
               info.st_size == ownership.size,
               let bytes = try? handle.read(upToCount: ownership.size + 1),
               SHA256.hash(data: bytes) == ownership.digest {
                _ = unlink(image.url.path)
            }
        }
        // rmdir is deliberately nonrecursive: files created by anyone else
        // prevent removal even if they appeared after staging.
        for directory in image.createdDirectories.reversed() { _ = rmdir(directory.path) }
    }

    struct FileFailure: LocalizedError {
        let name: String
        let reason: String
        var errorDescription: String? { "未插入图片“\(name)”：\(reason)" }
    }

    enum Failure: LocalizedError {
        case invalidImage, tooLarge, invalidDirectory, unsupportedFile
        var errorDescription: String? {
            switch self {
            case .invalidImage: return "无法读取这张图片，请检查文件是否完整。"
            case .tooLarge: return "图片超过导入限制（128 MiB 或 2 亿像素）。"
            case .invalidDirectory: return "图片目录必须是文档目录中的相对文件夹。"
            case .unsupportedFile: return "请选择本地图片文件。"
            }
        }
    }

    static let maximumBytes = 128 * 1024 * 1024

    private static func readFile(_ source: URL) throws -> Data {
        guard source.isFileURL else { throw Failure.unsupportedFile }
        let values = try source.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey])
        guard values.isRegularFile == true else { throw Failure.unsupportedFile }
        guard let size = values.fileSize, size <= maximumBytes else { throw Failure.tooLarge }
        return try Data(contentsOf: source, options: .mappedIfSafe)
    }

    static func importFile(_ source: URL, document: URL, directory: String = "assets") throws -> Imported {
        try importData(readFile(source), document: document, directory: directory)
    }

    static func referenceFile(_ source: URL) throws -> Imported {
        let url = source.standardizedFileURL.resolvingSymlinksInPath()
        let (_, width, height) = try inspect(readFile(url))
        return Imported(url: url, destination: url.path, pixelWidth: width, pixelHeight: height, createdFile: false, ownership: nil, createdDirectories: [])
    }

    static func isValidDirectory(_ directory: String) -> Bool {
        let parts = directory.split(separator: "/", omittingEmptySubsequences: false)
        return !directory.hasPrefix("/") && !parts.isEmpty && parts.allSatisfy {
            !$0.isEmpty && $0 != "." && $0 != ".." && !$0.contains("\0")
        }
    }

    private static func inspect(_ data: Data) throws -> (String, Int, Int) {
        guard data.count <= maximumBytes else { throw Failure.tooLarge }
        guard let image = CGImageSourceCreateWithData(data as CFData, nil),
              CGImageSourceGetCount(image) > 0,
              CGImageSourceGetStatus(image) == .statusComplete,
              let type = CGImageSourceGetType(image),
              let suffix = UTType(type as String)?.preferredFilenameExtension,
              let properties = CGImageSourceCopyPropertiesAtIndex(image, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int,
              let height = properties[kCGImagePropertyPixelHeight] as? Int,
              width > 0, height > 0 else { throw Failure.invalidImage }
        guard width <= 200_000_000 / height else { throw Failure.tooLarge }
        // Metadata alone can accept truncated payloads. Decode a tiny thumbnail
        // before publishing any file or source reference, without a full bitmap.
        guard CGImageSourceCreateThumbnailAtIndex(image, 0, [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceThumbnailMaxPixelSize: 32,
            kCGImageSourceShouldCacheImmediately: true
        ] as CFDictionary) != nil else { throw Failure.invalidImage }
        return (suffix, width, height)
    }

    static func importData(_ data: Data, document: URL, directory: String = "assets") throws -> Imported {
        let (suffix, width, height) = try inspect(data)
        guard document.isFileURL, isValidDirectory(directory) else { throw Failure.invalidDirectory }
        let parent = document.deletingLastPathComponent().standardizedFileURL.resolvingSymlinksInPath()
        let folder = parent.appendingPathComponent(directory, isDirectory: true)
        let resolved = folder.resolvingSymlinksInPath().standardizedFileURL
        guard resolved.path.hasPrefix(parent.path == "/" ? "/" : parent.path + "/") else { throw Failure.invalidDirectory }
        var createdDirectories: [URL] = []
        var current = parent
        do {
            for component in directory.split(separator: "/") {
                current.appendPathComponent(String(component), isDirectory: true)
                if mkdir(current.path, 0o755) == 0 { createdDirectories.append(current) }
                else if errno != EEXIST { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
            }
            let name = "image-\(UUID().uuidString.lowercased()).\(suffix)"
            let target = folder.appendingPathComponent(name)
            let descriptor = open(target.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0o600)
            guard descriptor >= 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
            let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
            do {
                try handle.write(contentsOf: data)
                try handle.synchronize()
                var info = stat()
                guard fstat(descriptor, &info) == 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
                let ownership = Ownership(device: info.st_dev, inode: info.st_ino, size: data.count, digest: SHA256.hash(data: data))
                try handle.close()
                return Imported(url: target, destination: directory + "/" + name, pixelWidth: width,
                    pixelHeight: height, createdFile: true, ownership: ownership, createdDirectories: createdDirectories)
            } catch {
                try? handle.close()
                _ = unlink(target.path)
                throw error
            }
        } catch {
            for directory in createdDirectories.reversed() { _ = rmdir(directory.path) }
            throw error
        }
    }
}
