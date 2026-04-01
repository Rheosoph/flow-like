package = "flow-like-wasm-sdk"
version = "0.1.0-1"
source = {
   url = "git+https://github.com/TM9657/flow-like.git",
   tag = "wasm-sdk-lua/v0.1.0",
   dir = "libs/wasm-sdk/wasm-sdk-lua",
}
description = {
   summary = "SDK for building Flow-Like WASM nodes in Lua",
   detailed = [[
      Lua SDK for building Flow-Like WASM nodes.
      Lua runs embedded in a C glue layer compiled to WebAssembly via Emscripten.
      Provides host function bindings, context management, and JSON helpers.
   ]],
   homepage = "https://github.com/TM9657/flow-like",
   license = "MIT",
}
dependencies = {
   "lua >= 5.4",
}
build = {
   type = "none",
   copy_directories = { "src" },
}
