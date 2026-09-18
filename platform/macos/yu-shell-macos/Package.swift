// swift-tools-version: 6.4

import Foundation
import PackageDescription

let packageDirectory = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .path

let package = Package(
    name: "YuShellMacOS",
    platforms: [.macOS(.v26)],
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
                    // SwiftBuild's linker driver otherwise infers SDK from
                    // the deployment triple. Preflight pins the actual SDK 27.
                    "-Xlinker", "-platform_version", "-Xlinker", "macos",
                    "-Xlinker", "26.0", "-Xlinker", "27.0",
                ])
            ]
        ),
        .target(
            name: "YuStorageFFI",
            path: "Sources/YuStorageFFI",
            publicHeadersPath: "include"
        ),
    ],
    // Toolchain and platform upgrade; strict-concurrency migration is separate.
    swiftLanguageModes: [.v5]
)
