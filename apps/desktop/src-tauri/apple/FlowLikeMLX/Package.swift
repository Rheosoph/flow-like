// swift-tools-version: 6.1

import PackageDescription

let package = Package(
    name: "FlowLikeMLX",
    platforms: [
        .macOS(.v14),
        .iOS(.v17),
    ],
    products: [
        .library(
            name: "FlowLikeMLX",
            type: .static,
            targets: ["FlowLikeMLX"]
        ),
        .executable(
            name: "flow-like-mlx",
            targets: ["FlowLikeMLXServer"]
        ),
    ],
    dependencies: [
        .package(
            url: "https://github.com/ml-explore/mlx-swift-lm.git",
            exact: "3.31.3"
        ),
        // Pin the MLX version used by mlx-swift-lm 3.31.3 so Xcode does not
        // silently resolve a patch release with a newer Swift tools floor.
        .package(
            url: "https://github.com/ml-explore/mlx-swift.git",
            exact: "0.31.3"
        ),
        .package(
            url: "https://github.com/huggingface/swift-transformers.git",
            exact: "1.3.0"
        ),
    ],
    targets: [
        .target(
            name: "FlowLikeMLX",
            dependencies: [
                .product(name: "MLX", package: "mlx-swift"),
                .product(name: "MLXLMCommon", package: "mlx-swift-lm"),
                .product(name: "MLXLLM", package: "mlx-swift-lm"),
                .product(name: "MLXVLM", package: "mlx-swift-lm"),
                .product(name: "Tokenizers", package: "swift-transformers"),
            ]
        ),
        .executableTarget(
            name: "FlowLikeMLXServer",
            dependencies: ["FlowLikeMLX"]
        ),
        .testTarget(
            name: "FlowLikeMLXTests",
            dependencies: ["FlowLikeMLX"]
        ),
    ]
)
