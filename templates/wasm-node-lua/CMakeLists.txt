cmake_minimum_required(VERSION 3.14)
project(flow_like_wasm_node_lua C)

# --- Lua 5.4 source (compiled to WASM alongside glue) ---
include(FetchContent)
FetchContent_Declare(lua54_src
    URL https://www.lua.org/ftp/lua-5.4.7.tar.gz
    URL_HASH SHA256=9fbf5e28ef86c69858f6d3d34eccc32e911c1a28b4120ff3e84aaa70cfbf1e30
)
FetchContent_MakeAvailable(lua54_src)

# Build Lua as a static library for WASM (exclude lua.c / luac.c)
set(LUA_SRC_DIR ${lua54_src_SOURCE_DIR}/src)
add_library(lua54 STATIC
    ${LUA_SRC_DIR}/lapi.c
    ${LUA_SRC_DIR}/lauxlib.c
    ${LUA_SRC_DIR}/lbaselib.c
    ${LUA_SRC_DIR}/lcode.c
    ${LUA_SRC_DIR}/lcorolib.c
    ${LUA_SRC_DIR}/lctype.c
    ${LUA_SRC_DIR}/ldblib.c
    ${LUA_SRC_DIR}/ldebug.c
    ${LUA_SRC_DIR}/ldo.c
    ${LUA_SRC_DIR}/ldump.c
    ${LUA_SRC_DIR}/lfunc.c
    ${LUA_SRC_DIR}/lgc.c
    ${LUA_SRC_DIR}/linit.c
    # liolib.c and loslib.c are excluded: they pull in OS syscalls not available
    # in bare WASM; stubbed in glue.c
    ${LUA_SRC_DIR}/llex.c
    ${LUA_SRC_DIR}/lmathlib.c
    ${LUA_SRC_DIR}/lmem.c
    ${LUA_SRC_DIR}/loadlib.c
    ${LUA_SRC_DIR}/lobject.c
    ${LUA_SRC_DIR}/lopcodes.c
    ${LUA_SRC_DIR}/lparser.c
    ${LUA_SRC_DIR}/lstate.c
    ${LUA_SRC_DIR}/lstring.c
    ${LUA_SRC_DIR}/lstrlib.c
    ${LUA_SRC_DIR}/ltable.c
    ${LUA_SRC_DIR}/ltablib.c
    ${LUA_SRC_DIR}/ltm.c
    ${LUA_SRC_DIR}/lundump.c
    ${LUA_SRC_DIR}/lutf8lib.c
    ${LUA_SRC_DIR}/lvm.c
    ${LUA_SRC_DIR}/lzio.c
)
target_include_directories(lua54 PUBLIC ${LUA_SRC_DIR})
target_compile_definitions(lua54 PUBLIC LUA_USE_C89)

# --- SDK (C glue + Lua SDK) via FetchContent ---
if(EXISTS ${CMAKE_CURRENT_SOURCE_DIR}/../wasm-sdk-lua/src/glue.c)
    # Local in-tree (monorepo) build
    set(SDK_DIR ${CMAKE_CURRENT_SOURCE_DIR}/../wasm-sdk-lua)
else()
    # Remote fetch for standalone template usage
    FetchContent_Declare(
        flow_like_wasm_sdk_lua
        GIT_REPOSITORY https://github.com/Rheosoph/flow-like.git
        GIT_TAG        dev
        SOURCE_SUBDIR  templates/wasm-sdk-lua
    )
    FetchContent_MakeAvailable(flow_like_wasm_sdk_lua)
    set(SDK_DIR ${flow_like_wasm_sdk_lua_SOURCE_DIR})
endif()

# --- Embed Lua sources as C string arrays ---
# cmake script that converts a .lua file → .c  (const char _lua_<name>_source[] = "...")
function(embed_lua_source LUA_FILE VAR_NAME OUTPUT_C)
    file(READ ${LUA_FILE} LUA_CONTENT)
    # Escape backslashes, quotes, newlines for C string literal
    string(REPLACE "\\" "\\\\" LUA_CONTENT "${LUA_CONTENT}")
    string(REPLACE "\"" "\\\"" LUA_CONTENT "${LUA_CONTENT}")
    string(REPLACE "\n" "\\n\"\n\"" LUA_CONTENT "${LUA_CONTENT}")
    file(WRITE ${OUTPUT_C} "/* Auto-generated — do not edit */\nconst char ${VAR_NAME}[] = \"${LUA_CONTENT}\";\n")
endfunction()

set(GEN_DIR ${CMAKE_CURRENT_BINARY_DIR}/generated)
file(MAKE_DIRECTORY ${GEN_DIR})

embed_lua_source(${SDK_DIR}/src/sdk.lua  _lua_sdk_source  ${GEN_DIR}/sdk_source.c)
embed_lua_source(${CMAKE_CURRENT_SOURCE_DIR}/src/node.lua _lua_node_source ${GEN_DIR}/node_source.c)

# --- Node WASM module ---
add_executable(node
    ${SDK_DIR}/src/glue.c
    ${GEN_DIR}/sdk_source.c
    ${GEN_DIR}/node_source.c
)

target_link_libraries(node PRIVATE lua54)
target_include_directories(node PRIVATE ${LUA_SRC_DIR})

if(EMSCRIPTEN)
    set_target_properties(node PROPERTIES
        OUTPUT_NAME "node"
        SUFFIX ".wasm"
    )
    target_link_options(node PRIVATE
        -sSTANDALONE_WASM
        -sEXPORTED_FUNCTIONS=_get_node,_get_nodes,_run,_alloc,_dealloc,_get_abi_version
        --no-entry
        -sERROR_ON_UNDEFINED_SYMBOLS=0
        -sALLOW_MEMORY_GROWTH=1
        -sSUPPORT_LONGJMP=emscripten
        -O2
    )
    target_compile_options(node PRIVATE -msimd128 -mrelaxed-simd -O2)
endif()
