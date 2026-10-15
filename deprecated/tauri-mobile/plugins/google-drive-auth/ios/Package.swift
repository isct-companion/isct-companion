// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "tauri-plugin-google-drive-auth",
    // SwiftPM also validates the host platform during iOS cross-compilation.
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [
        .library(
            name: "tauri-plugin-google-drive-auth",
            type: .static,
            targets: ["tauri-plugin-google-drive-auth"]
        )
    ],
    dependencies: [
        .package(name: "Tauri", path: "../.tauri/tauri-api"),
        .package(url: "https://github.com/google/GoogleSignIn-iOS.git", from: "9.0.0")
    ],
    targets: [
        .target(
            name: "tauri-plugin-google-drive-auth",
            dependencies: [
                .byName(name: "Tauri"),
                .product(name: "GoogleSignIn", package: "GoogleSignIn-iOS")
            ],
            path: "Sources"
        )
    ]
)
