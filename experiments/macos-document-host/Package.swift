// swift-tools-version: 5.10

import Foundation
import PackageDescription

let packageDirectory = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .path

let package = Package(
    name: "YuMacDocumentHost",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "YuMacDocumentHost", targets: ["YuMacDocumentHost"]),
    ],
    targets: [
        .executableTarget(
            name: "YuMacDocumentHost",
            dependencies: ["YuStorageFFI"],
            linkerSettings: [
                .unsafeFlags([
                    "-L\(packageDirectory)/.rust",
                    "-lyu_storage_ffi",
                ])
            ]
        ),
        .target(
            name: "YuStorageFFI",
            path: "Sources/YuStorageFFI",
            publicHeadersPath: "include"
        ),
    ]
)
