// Reactor initialization for WASM Component Model.
//
// SwiftWasm executable targets produce a command module with _start.
// For a reactor component, we need _initialize instead.
// This function calls the linker-generated __wasm_call_ctors() to run
// global constructors (Swift runtime initialization) without invoking main().

extern void __wasm_call_ctors(void);

void _initialize(void) {
    __wasm_call_ctors();
}
