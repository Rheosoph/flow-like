# cmake-rs delegates platform selection to this file when a toolchain is set.
set(CMAKE_SYSTEM_NAME iOS)
if("$ENV{CARGO_CFG_TARGET_ARCH}" STREQUAL "aarch64")
  set(CMAKE_SYSTEM_PROCESSOR arm64)
else()
  set(CMAKE_SYSTEM_PROCESSOR "$ENV{CARGO_CFG_TARGET_ARCH}")
endif()
set(CMAKE_OSX_ARCHITECTURES "${CMAKE_SYSTEM_PROCESSOR}" CACHE STRING "Target architecture")
if(DEFINED ENV{SDKROOT})
  set(CMAKE_OSX_SYSROOT "$ENV{SDKROOT}" CACHE PATH "Target Apple SDK")
endif()
if(DEFINED ENV{IPHONEOS_DEPLOYMENT_TARGET})
  set(CMAKE_OSX_DEPLOYMENT_TARGET "$ENV{IPHONEOS_DEPLOYMENT_TARGET}" CACHE STRING "Minimum iOS version")
endif()

# Cargo supplies prefixes for dependencies already built for this target.
# Keep iOS's target-only lookup policy and add those dependency roots.
if(DEFINED ENV{CMAKE_PREFIX_PATH})
  file(TO_CMAKE_PATH "$ENV{CMAKE_PREFIX_PATH}" _flowlike_dependency_roots)
  list(APPEND CMAKE_FIND_ROOT_PATH ${_flowlike_dependency_roots})
  list(REMOVE_DUPLICATES CMAKE_FIND_ROOT_PATH)
endif()
