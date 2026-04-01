// WitBindings — C bridge for wit-bindgen generated WIT bindings.
// Run `mise run generate` to create the actual bindings (flow_like_node.h).

#pragma once

#include <stddef.h>

#if __has_include("flow_like_node.h")
#include "flow_like_node.h"
#endif

// cabi_realloc is defined in the generated flow_like_node.c
extern void* cabi_realloc(void* ptr, size_t old_size, size_t align, size_t new_size);
