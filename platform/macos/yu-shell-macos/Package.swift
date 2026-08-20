// swift-tools-version: 5.10

import Foundation
import PackageDescription

let packageDirectory = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .path

let package = Package(
    name: "YuShellMacOS",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "Yu", targets: ["Yu"]),
    ],
    targets: [
        .executableTarget(
            name: "Yu",
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
