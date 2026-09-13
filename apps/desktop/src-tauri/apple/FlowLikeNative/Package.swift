// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "FlowLikeNative",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [.library(name: "FlowLikeNative", type: .static, targets: ["FlowLikeNative"])],
    targets: [
        .target(name: "FlowLikeNative"),
        .testTarget(name: "FlowLikeNativeTests", dependencies: ["FlowLikeNative"]),
    ],
    swiftLanguageModes: [.v5]
)
