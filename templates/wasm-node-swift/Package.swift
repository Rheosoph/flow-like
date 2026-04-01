// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "FlowLikeWasmNode",
    targets: [
        .target(
            name: "WitBindings",
            path: "Sources/WitBindings",
            publicHeadersPath: "include"
        ),
        .executableTarget(
            name: "Node",
            dependencies: ["WitBindings"],
            path: "Sources/Node",
            linkerSettings: [
                .unsafeFlags([
                    "-Xlinker", "--no-entry",
                    "-Xlinker", "--export=_initialize",
                    "-Xlinker", "--export=exports_flow_like_node_get_node",
                    "-Xlinker", "--export=exports_flow_like_node_get_nodes",
                    "-Xlinker", "--export=exports_flow_like_node_run",
                    "-Xlinker", "--export=exports_flow_like_node_get_abi_version",
                    "-Xlinker", "--export=cabi_realloc",
                    "-Xlinker", "Sources/WitBindings/flow_like_node_component_type.o",
                ]),
            ]
        ),
    ]
)
