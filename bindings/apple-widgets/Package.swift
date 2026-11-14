// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "IsctWidgets",
    platforms: [.iOS(.v18), .macOS(.v15)],
    products: [.library(name: "IsctWidgets", targets: ["IsctWidgets"])],
    targets: [.target(name: "IsctWidgets")]
)
