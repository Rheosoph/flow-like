# cmake-rs delegates platform selection to this file when a toolchain is set.
# The Tauri CLI exports NDK_HOME; other Android tooling uses the remaining names.
foreach(_flowlike_ndk_var NDK_HOME ANDROID_NDK_HOME ANDROID_NDK_ROOT ANDROID_NDK)
  if(NOT _flowlike_ndk AND IS_DIRECTORY "$ENV{${_flowlike_ndk_var}}")
    file(TO_CMAKE_PATH "$ENV{${_flowlike_ndk_var}}" _flowlike_ndk)
  endif()
endforeach()
if(NOT _flowlike_ndk)
  message(FATAL_ERROR "Android CMake builds need the NDK: set NDK_HOME to the NDK directory.")
endif()

set(_flowlike_arch "$ENV{CARGO_CFG_TARGET_ARCH}")
if(_flowlike_arch STREQUAL "aarch64")
  set(ANDROID_ABI arm64-v8a)
elseif(_flowlike_arch STREQUAL "arm")
  set(ANDROID_ABI armeabi-v7a)
elseif(_flowlike_arch STREQUAL "x86" OR _flowlike_arch STREQUAL "x86_64")
  set(ANDROID_ABI "${_flowlike_arch}")
else()
  message(FATAL_ERROR "No Android ABI for Rust target arch '${_flowlike_arch}'.")
endif()

# The Tauri CLI exports the app's minSdk; without it the NDK picks its minimum.
if(DEFINED ENV{ANDROID_NATIVE_API_LEVEL})
  set(ANDROID_NATIVE_API_LEVEL "$ENV{ANDROID_NATIVE_API_LEVEL}")
endif()

include("${_flowlike_ndk}/build/cmake/android.toolchain.cmake")

# Cargo supplies prefixes for dependencies already built for this target.
# Keep the NDK's target-only lookup policy and add those dependency roots.
if(DEFINED ENV{CMAKE_PREFIX_PATH})
  file(TO_CMAKE_PATH "$ENV{CMAKE_PREFIX_PATH}" _flowlike_dependency_roots)
  list(APPEND CMAKE_FIND_ROOT_PATH ${_flowlike_dependency_roots})
  list(REMOVE_DUPLICATES CMAKE_FIND_ROOT_PATH)
endif()
