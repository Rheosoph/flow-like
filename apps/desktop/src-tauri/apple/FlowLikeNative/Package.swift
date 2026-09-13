// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "FlowLikeNative",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [.library(name: "FlowLikeNative", type: .static, targets: ["FlowLikeNative"])],
    targets: [
        .target(name: "FlowLikeNative"),
        .testTarget(name: "FlowLikeNativeTests", dependencies: ["FlowLikeNative"]),
        // Give SourceKit build settings for the macOS extension sources.
        // Xcode builds the extension bundles and configures the iOS intent host.
        .target(
            name: "FlowLikeWidgets",
            dependencies: ["FlowLikeNative"],
            path: "Extensions/Widgets",
            exclude: ["Info-iOS.plist", "Info-macOS.plist", "Widgets-iOS.entitlements", "Widgets-macOS.entitlements"],
            sources: ["FlowLikeWidgets.swift"],
            swiftSettings: [.unsafeFlags(["-application-extension"])]
        ),
        .target(
            name: "FlowLikeShare",
            dependencies: ["FlowLikeNative"],
            path: "Extensions/Share",
            exclude: ["Info-iOS.plist", "Info-macOS.plist", "Share-iOS.entitlements", "Share-macOS.entitlements"],
            sources: ["ShareViewController.swift"],
            swiftSettings: [.unsafeFlags(["-application-extension"])]
        ),
    ],
    swiftLanguageModes: [.v5]
)
