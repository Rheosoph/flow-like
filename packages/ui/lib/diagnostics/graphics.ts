import {
	type WebGlContextKind,
	supportsWebGl,
} from "../../hooks/use-webgl-support";
import {
	TIMED_OUT,
	attempt,
	attemptAsync,
	errorMessage,
	finiteNumber,
	nonEmptyString,
	optionalBoolean,
	readGlobal,
	readProperty,
	withTimeout,
} from "./safe";
import {
	type DiagnosticsGraphics,
	type DiagnosticsWebGl,
	type DiagnosticsWebGpu,
	NOTABLE_WEBGPU_FEATURES,
	type NotableWebGpuFeature,
	WEBGPU_LIMIT_NAMES,
	type WebGpuAdapterSummary,
	type WebGpuLimitName,
	type WebGpuStatus,
} from "./types";

export const WEBGPU_ADAPTER_TIMEOUT_MS = 3000;
const ADAPTER_INFO_TIMEOUT_MS = 1000;

type WebGlDetails = Omit<DiagnosticsWebGl, "webgl" | "webgl2">;

const EMPTY_WEBGL_DETAILS: WebGlDetails = {
	version: null,
	vendor: null,
	renderer: null,
	unmasked: false,
	maxTextureSize: null,
};

function notable(features: readonly string[]) {
	return Object.fromEntries(
		NOTABLE_WEBGPU_FEATURES.map((feature) => [
			feature,
			features.includes(feature),
		]),
	) as Record<NotableWebGpuFeature, boolean>;
}

export function webGpuUnavailable(
	status: Exclude<WebGpuStatus, "available">,
	error: string | null = null,
): DiagnosticsWebGpu {
	return {
		status,
		error,
		adapter: null,
		isFallbackAdapter: null,
		features: [],
		notableFeatures: notable([]),
		limits: {},
	};
}

export function normaliseAdapterInfo(
	info: unknown,
): WebGpuAdapterSummary | null {
	if (info === null || typeof info !== "object") return null;
	const summary = {
		vendor: nonEmptyString(readProperty(info, "vendor")),
		architecture: nonEmptyString(readProperty(info, "architecture")),
		device: nonEmptyString(readProperty(info, "device")),
		description: nonEmptyString(readProperty(info, "description")),
	};
	return Object.values(summary).some((value) => value !== null)
		? summary
		: null;
}

function listFeatures(features: unknown): string[] {
	const iterator = readProperty<unknown>(features, Symbol.iterator);
	if (typeof iterator !== "function") return [];
	return Array.from(features as Iterable<unknown>)
		.filter((feature): feature is string => typeof feature === "string")
		.sort();
}

function readLimits(limits: unknown) {
	const result: Partial<Record<WebGpuLimitName, number>> = {};
	for (const name of WEBGPU_LIMIT_NAMES) {
		const value = finiteNumber(readProperty(limits, name));
		if (value !== null) result[name] = value;
	}
	return result;
}

export function summariseWebGpuAdapter(
	adapter: unknown,
	info: unknown,
): DiagnosticsWebGpu {
	const features = attempt(
		() => listFeatures(readProperty(adapter, "features")),
		[],
	);
	return {
		status: "available",
		error: null,
		adapter: normaliseAdapterInfo(info),
		isFallbackAdapter:
			optionalBoolean(readProperty(info, "isFallbackAdapter")) ??
			optionalBoolean(readProperty(adapter, "isFallbackAdapter")),
		features,
		notableFeatures: notable(features),
		limits: attempt(() => readLimits(readProperty(adapter, "limits")), {}),
	};
}

async function readAdapterInfo(adapter: unknown): Promise<unknown> {
	const info = readProperty<unknown>(adapter, "info");
	if (info) return info;
	const requestAdapterInfo = readProperty<() => Promise<unknown>>(
		adapter,
		"requestAdapterInfo",
	);
	if (typeof requestAdapterInfo !== "function") return null;
	const result = await withTimeout(
		requestAdapterInfo.call(adapter),
		ADAPTER_INFO_TIMEOUT_MS,
	);
	return result === TIMED_OUT ? null : result;
}

export async function probeWebGpu(
	timeoutMs = WEBGPU_ADAPTER_TIMEOUT_MS,
): Promise<DiagnosticsWebGpu> {
	const gpu = readProperty<unknown>(readGlobal("navigator"), "gpu");
	const requestAdapter = readProperty<() => Promise<unknown>>(
		gpu,
		"requestAdapter",
	);
	if (typeof requestAdapter !== "function")
		return webGpuUnavailable("unsupported");
	try {
		const adapter = await withTimeout(requestAdapter.call(gpu), timeoutMs);
		if (adapter === TIMED_OUT) return webGpuUnavailable("timeout");
		if (!adapter) return webGpuUnavailable("no-adapter");
		const info = await attemptAsync(() => readAdapterInfo(adapter), null);
		return summariseWebGpuAdapter(adapter, info);
	} catch (error) {
		return webGpuUnavailable("error", errorMessage(error));
	}
}

export function readWebGlDetails(
	gl: WebGLRenderingContext | WebGL2RenderingContext,
): WebGlDetails {
	const debug = gl.getExtension("WEBGL_debug_renderer_info");
	const unmaskedRenderer = debug
		? nonEmptyString(gl.getParameter(debug.UNMASKED_RENDERER_WEBGL))
		: null;
	const unmaskedVendor = debug
		? nonEmptyString(gl.getParameter(debug.UNMASKED_VENDOR_WEBGL))
		: null;
	return {
		version: nonEmptyString(gl.getParameter(gl.VERSION)),
		vendor: unmaskedVendor ?? nonEmptyString(gl.getParameter(gl.VENDOR)),
		renderer: unmaskedRenderer ?? nonEmptyString(gl.getParameter(gl.RENDERER)),
		unmasked: unmaskedRenderer !== null,
		maxTextureSize: finiteNumber(gl.getParameter(gl.MAX_TEXTURE_SIZE)),
	};
}

function probeWebGlDetails(kind: WebGlContextKind): WebGlDetails {
	const canvas = readGlobal<Document>("document")?.createElement("canvas");
	const gl = canvas?.getContext(kind) as
		| WebGLRenderingContext
		| WebGL2RenderingContext
		| null
		| undefined;
	if (!gl) return EMPTY_WEBGL_DETAILS;
	try {
		return readWebGlDetails(gl);
	} finally {
		gl.getExtension("WEBGL_lose_context")?.loseContext();
	}
}

export function probeWebGl(): DiagnosticsWebGl {
	const webgl = attempt(() => supportsWebGl("webgl"), false);
	const webgl2 = attempt(() => supportsWebGl("webgl2"), false);
	const details =
		webgl || webgl2
			? attempt(
					() => probeWebGlDetails(webgl2 ? "webgl2" : "webgl"),
					EMPTY_WEBGL_DETAILS,
				)
			: EMPTY_WEBGL_DETAILS;
	return { webgl, webgl2, ...details };
}

export function emptyGraphics(): DiagnosticsGraphics {
	return {
		webgpu: webGpuUnavailable("error"),
		webgl: { webgl: false, webgl2: false, ...EMPTY_WEBGL_DETAILS },
		offscreenCanvas: false,
	};
}

export async function probeGraphics(
	webGpuTimeoutMs = WEBGPU_ADAPTER_TIMEOUT_MS,
): Promise<DiagnosticsGraphics> {
	const fallback = emptyGraphics();
	const webgpu = await attemptAsync(
		() => probeWebGpu(webGpuTimeoutMs),
		fallback.webgpu,
	);
	return {
		webgpu,
		webgl: attempt(probeWebGl, fallback.webgl),
		offscreenCanvas: typeof readGlobal("OffscreenCanvas") === "function",
	};
}

export function simplifyGpuName(renderer: string | null): string | null {
	if (!renderer) return null;
	const angle = /^ANGLE \((.*)\)$/.exec(renderer);
	if (!angle?.[1]) return renderer;
	const parts = angle[1].split(", ");
	const device = (parts[1] ?? parts[0] ?? "")
		.replace(/^ANGLE Metal Renderer: /, "")
		.replace(/ \(0x[0-9a-f]+\)/gi, "")
		.replace(/ Direct3D.*$/, "")
		.trim();
	return device || renderer;
}

export function gpuDisplayName(graphics: DiagnosticsGraphics): string | null {
	const adapter = graphics.webgpu.adapter;
	const adapterName =
		[adapter?.vendor, adapter?.architecture].filter(Boolean).join(" ") || null;
	return (
		simplifyGpuName(graphics.webgl.renderer) ??
		adapter?.description ??
		adapterName
	);
}
