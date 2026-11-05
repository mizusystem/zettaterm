// swift-tools-version: 6.0
// 先に scripts/build-core.sh で Rust コアをビルドしておくこと（build/MTCoreFFI.xcframework と
// Sources/MTCore/Generated/ を生成する）。

import PackageDescription

let package = Package(
    name: "MultiTerm",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "MultiTerm", targets: ["MultiTerm"])
    ],
    targets: [
        // Rust の静的ライブラリと C ヘッダ
        .binaryTarget(name: "MTCoreFFI", path: "build/MTCoreFFI.xcframework"),
        // UniFFI が生成した Swift バインディング
        .target(
            name: "MTCore",
            dependencies: ["MTCoreFFI"],
            path: "Sources/MTCore",
            swiftSettings: [.swiftLanguageMode(.v5)],
            linkerSettings: [.linkedLibrary("iconv")]
        ),
        .executableTarget(
            name: "MultiTerm",
            dependencies: ["MTCore"],
            path: "Sources/MultiTerm",
            resources: [.copy("Resources/spec")]
        ),
        .testTarget(
            name: "MultiTermTests",
            dependencies: ["MultiTerm", "MTCore"],
            path: "Tests/MultiTermTests"
        ),
    ]
)
