// swift-tools-version: 6.0
import PackageDescription
let package = Package(
    name: "IsctCore",
    platforms: [.iOS(.v17)],
    products: [.library(name: "IsctCore", targets: ["IsctCore"])],
    targets: [
        .binaryTarget(name: "IsctCoreFFI", path: "artifacts/IsctCore.xcframework"),
        .target(name: "IsctCore", dependencies: ["IsctCoreFFI"], swiftSettings: [.swiftLanguageMode(.v5)], linkerSettings: [
            .linkedFramework("Security"), .linkedFramework("SystemConfiguration"),
            .linkedFramework("CoreFoundation"), .linkedFramework("CoreML"),
            .linkedLibrary("c++")
        ])
    ]
)
