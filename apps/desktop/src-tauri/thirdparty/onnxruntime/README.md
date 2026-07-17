# ONNX Runtime mobile binaries

FlowLike vendors the official ONNX Runtime 1.24.3 mobile packages:

- Android `arm64-v8a/libonnxruntime.so` is extracted from
  `onnxruntime-android-1.24.3.aar` published on Maven Central.
- Apple `gen/apple/thirdparty/onnxruntime.xcframework` is extracted from the
  `onnxruntime-c` 1.24.3 CocoaPods archive.

Source archives:

- <https://repo1.maven.org/maven2/com/microsoft/onnxruntime/onnxruntime-android/1.24.3/onnxruntime-android-1.24.3.aar>
  (`sha256:67397e4a970e75617f765d2015ceaf911917e1d822276cfb5792744e8085cbce`)
- <https://download.onnxruntime.ai/pod-archive-onnxruntime-c-1.24.3.zip>
  (`sha256:b7eedc45932bac758ffd057cac0feb3f682269e47750b159e4c865145cbf0a8e`)

The Android package requires API level 24. The Apple package requires iOS 15.1
and macOS 14.0. Rust consumers currently use ONNX Runtime C API 24.

Windows desktop builds select ORT's DirectML distribution and must stage its
`DirectML.dll` before Tauri validates bundle resources. Use `bun run build:win:x64`
or `bun run build:win:arm` from `apps/desktop`; those wrappers run
`scripts/prepare-windows-prereqs.ts` before the Tauri build. The same preparation
is wired into the Windows development scripts and release CI.
