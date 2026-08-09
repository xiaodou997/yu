// swift-tools-version: 5.10

import PackageDescription

let package = Package(
    name: "YuMacTextInputSpike",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "YuMacTextInputSpike", targets: ["YuMacTextInputSpike"]),
    ],
    targets: [
        .executableTarget(name: "YuMacTextInputSpike"),
    ]
)
