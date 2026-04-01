cmake_minimum_required(VERSION 3.10)
project(flow_like_wasm_sdk LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

add_library(flow_like_wasm_sdk STATIC
    src/sdk.cpp
)

target_include_directories(flow_like_wasm_sdk PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}/include
)

if(EMSCRIPTEN)
    set_target_properties(flow_like_wasm_sdk PROPERTIES
        COMPILE_FLAGS "-fno-exceptions -fno-rtti -O2"
    )
endif()
