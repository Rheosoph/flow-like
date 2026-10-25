"use client";

import { useSyncExternalStore } from "react";

export type WebGlContextKind = "webgl" | "webgl2";

const support = new Map<WebGlContextKind, boolean>();

/** Probes once per page and releases the probe context, which counts against the browser's limit. */
export function supportsWebGl(kind: WebGlContextKind): boolean {
	const known = support.get(kind);
	if (known !== undefined) return known;
	let supported = false;
	try {
		const gl = document.createElement("canvas").getContext(kind) as
			| WebGLRenderingContext
			| WebGL2RenderingContext
			| null;
		gl?.getExtension("WEBGL_lose_context")?.loseContext();
		supported = gl !== null;
	} catch {
		supported = false;
	}
	support.set(kind, supported);
	return supported;
}

const subscribeNever = () => () => {};

/** `null` while hydrating, where there is no canvas to probe. */
export function useWebGlSupport(kind: WebGlContextKind): boolean | null {
	return useSyncExternalStore(
		subscribeNever,
		() => supportsWebGl(kind),
		() => null,
	);
}
