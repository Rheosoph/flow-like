cmake_minimum_required(VERSION 3.14)
project(flow_like_wasm_node LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# wasi-sdk toolchain (set WASI_SDK_PATH or use default install location)
if(NOT DEFINED WASI_SDK_PATH AND DEFINED ENV{WASI_SDK_PATH})
    set(WASI_SDK_PATH $ENV{WASI_SDK_PATH})
endif()

# Collect wit-bindgen-c generated sources from gen/
file(GLOB WIT_BINDGEN_SOURCES ${CMAKE_CURRENT_SOURCE_DIR}/gen/*.c)

# Component type object (produced by wit-bindgen alongside .c/.h files)
set(WIT_COMPONENT_TYPE_OBJ ${CMAKE_CURRENT_SOURCE_DIR}/gen/flow_like_node_component_type.o)

# Node WASM module (reactor — no main)
add_executable(node
    src/node.cpp
    ${WIT_BINDGEN_SOURCES}
)

target_include_directories(node PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/gen
)

target_compile_options(node PRIVATE
    -fno-exceptions
    -fno-rtti
    -msimd128
    -mrelaxed-simd
    -O2
)

# Build as a WASI reactor (no _start, exports via wit-bindgen)
target_link_options(node PRIVATE
    -mexec-model=reactor
    ${WIT_COMPONENT_TYPE_OBJ}
)

set_target_properties(node PROPERTIES
    OUTPUT_NAME "node"
    SUFFIX ".wasm"
)
