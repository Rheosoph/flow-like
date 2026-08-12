/**
 * The bundled indexeddbshim UMD build references `self` at module scope.
 * During Next.js prerendering (Node) `self` does not exist, so provide it
 * before that module evaluates. Must stay a separate module: import
 * hoisting would otherwise evaluate the shim first.
 */
if (typeof self === "undefined") {
	(globalThis as Record<string, unknown>).self = globalThis;
}

export {};
