// swift-tools-version: 5.10

import Foundation
import PackageDescription

let packageDirectory = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .path

let package = Package(
    name: "YuMacTextInputSpike",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "YuMacTextInputSpike", targets: ["YuMacTextInputSpike"]),
    ],
    targets: [
        .executableTarget(
            name: "YuMacTextInputSpike",
            dependencies: ["YuEditorFFI"],
            linkerSettings: [
                .unsafeFlags([
                    "-L\(packageDirectory)/.rust",
                    "-lyu_editor_ffi",
                ])
            ]
        ),
        .target(
            name: "YuEditorFFI",
            path: "Sources/YuEditorFFI",
            publicHeadersPath: "include"
        ),
    ]
)
