const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.resolveTargetQuery(.{
        .cpu_arch = .wasm32,
        .os_tag = .wasi,
        .cpu_features_add = std.Target.wasm.featureSet(&.{ .simd128, .relaxed_simd }),
    });
    const optimize = b.standardOptimizeOption(.{});

    const lib = b.addExecutable(.{
        .name = "node",
        .root_source_file = b.path("src/main.zig"),
        .target = target,
        .optimize = optimize,
    });

    // wit-bindgen-c generated sources (run `mise run generate` first)
    lib.addIncludePath(b.path("gen"));
    lib.addCSourceFiles(.{
        .files = &.{"gen/flow_like_node.c"},
        .flags = &.{"-std=c11"},
    });
    lib.addObjectFile(b.path("gen/flow_like_node_component_type.o"));

    lib.linkLibC();
    lib.entry = .disabled;
    lib.rdynamic = true;

    b.installArtifact(lib);
}
