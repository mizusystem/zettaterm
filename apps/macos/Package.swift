// swift-tools-version: 6.0
// 先に scripts/build-core.sh で Rust コアをビルドしておくこと（build/ZettaCoreFFI.xcframework と
// Sources/ZettaCore/Generated/ を生成する）。

import PackageDescription

let package = Package(
    name: "ZettaTerm",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "ZettaTerm", targets: ["ZettaTerm"])
    ],
    targets: [
        // Rust の静的ライブラリと C ヘッダ
        .binaryTarget(name: "ZettaCoreFFI", path: "build/ZettaCoreFFI.xcframework"),
        // UniFFI が生成した Swift バインディング
        .target(
            name: "ZettaCore",
            dependencies: ["ZettaCoreFFI"],
            path: "Sources/ZettaCore",
            swiftSettings: [.swiftLanguageMode(.v5)],
            linkerSettings: [.linkedLibrary("iconv")]
        ),
        .executableTarget(
            name: "ZettaTerm",
            dependencies: ["ZettaCore"],
            path: "Sources/ZettaTerm",
            resources: [.copy("Resources/spec"), .copy("Localization")]
        ),
        .testTarget(
            name: "ZettaTermTests",
            dependencies: ["ZettaTerm", "ZettaCore"],
            path: "Tests/ZettaTermTests"
        ),
    ]
)
