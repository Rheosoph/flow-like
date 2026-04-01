// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "FlowLikeSDK",
    products: [
        .library(name: "FlowLikeSDK", targets: ["FlowLikeSDK"]),
    ],
    targets: [
        .target(
            name: "FlowLikeHostC",
            path: "Sources/FlowLikeHostC",
            publicHeadersPath: "include"
        ),
        .target(
            name: "FlowLikeSDK",
            dependencies: ["FlowLikeHostC"]
        ),
    ]
)
