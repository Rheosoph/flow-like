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
            // Earliest upstream revision containing both the Gemma 4 VLM
            // shared-KV strict-loader fix and the cooperative iOS cancellation
            // fixes. The 3.31.4 tag predates them, so keep this immutable until
            // a newer release contains all three repairs.
            revision: "10e0cb7442920d3f67a08e067d6670334e9dadef"
        ),
        // Pin the minimum MLX version required by the selected LM revision so
        // Xcode does not silently resolve a patch with a newer tools floor.
        .package(
            url: "https://github.com/ml-explore/mlx-swift.git",
            exact: "0.31.4"
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
