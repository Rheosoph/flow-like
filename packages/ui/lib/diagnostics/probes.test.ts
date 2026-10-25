import { afterEach, describe, expect, it, vi } from "vitest";
import { identityFromClientHints, identityFromUserAgent } from "./browser";
import {
	WASM_SIMD_PROBE,
	WASM_THREADS_PROBE,
	detectWasmSimd,
	detectWasmThreads,
	detectWebAssembly,
	probeCapabilities,
} from "./capabilities";
import { formatDiagnosticsReport } from "./format";
import {
	gpuDisplayName,
	normaliseAdapterInfo,
	probeWebGpu,
	simplifyGpuName,
	summariseWebGpuAdapter,
} from "./graphics";
import {
	collectDiagnostics,
	probeStorage,
	withDiagnosticsContext,
} from "./probes";

const redefined: string[] = [];

function throwingGetter(name: string) {
	Object.defineProperty(globalThis, name, {
		configurable: true,
		get() {
			throw new Error(`${name} is not allowed here`);
		},
	});
	redefined.push(name);
}

const throwingObject = new Proxy(
	{},
	{
		get() {
			throw new Error("blocked");
		},
		has() {
			throw new Error("blocked");
		},
	},
);

afterEach(() => {
	vi.unstubAllGlobals();
	for (const name of redefined.splice(0)) {
		Reflect.deleteProperty(globalThis, name);
	}
});

describe("WebAssembly feature detection", () => {
	it("validates the probe modules on an engine that supports them", () => {
		expect(WebAssembly.validate(WASM_SIMD_PROBE)).toBe(true);
		expect(WebAssembly.validate(WASM_THREADS_PROBE)).toBe(true);
		expect(detectWebAssembly(WebAssembly)).toBe(true);
		expect(detectWasmSimd(WebAssembly)).toBe(true);
	});

	it("returns false instead of throwing for a missing or broken engine", () => {
		expect(detectWasmSimd(undefined)).toBe(false);
		expect(detectWasmSimd({ validate: () => "yes" })).toBe(false);
		expect(
			detectWasmSimd({
				validate: () => {
					throw new Error("no");
				},
			}),
		).toBe(false);
		expect(detectWebAssembly(throwingObject)).toBe(false);
	});

	it("needs SharedArrayBuffer, cross-origin isolation and shared memory for threads", () => {
		const env = {
			wasm: WebAssembly,
			sharedArrayBuffer: true,
			crossOriginIsolated: true,
		};
		expect(detectWasmThreads(env)).toBe(true);
		expect(detectWasmThreads({ ...env, crossOriginIsolated: false })).toBe(
			false,
		);
		expect(detectWasmThreads({ ...env, crossOriginIsolated: null })).toBe(
			false,
		);
		expect(detectWasmThreads({ ...env, sharedArrayBuffer: false })).toBe(false);
		expect(detectWasmThreads({ ...env, wasm: { validate: () => false } })).toBe(
			false,
		);
	});
});

describe("WebGPU adapter normalisation", () => {
	it("drops empty adapter info fields and empty info objects", () => {
		expect(
			normaliseAdapterInfo({
				vendor: "apple",
				architecture: "metal-3",
				device: "",
				description: "  ",
			}),
		).toEqual({
			vendor: "apple",
			architecture: "metal-3",
			device: null,
			description: null,
		});
		expect(normaliseAdapterInfo({ vendor: "", architecture: "" })).toBeNull();
		expect(normaliseAdapterInfo(null)).toBeNull();
	});

	it("summarises features, notable features, limits and the fallback flag", () => {
		const adapter = {
			features: new Set(["timestamp-query", "bgra8unorm-storage", 7]),
			limits: {
				maxBufferSize: 268_435_456,
				maxStorageBufferBindingSize: 134_217_728,
				maxTextureDimension2D: 8192,
				maxComputeWorkgroupStorageSize: Number.NaN,
			},
			isFallbackAdapter: true,
		};
		const summary = summariseWebGpuAdapter(adapter, {
			vendor: "nvidia",
			isFallbackAdapter: false,
		});
		expect(summary.status).toBe("available");
		expect(summary.features).toEqual(["bgra8unorm-storage", "timestamp-query"]);
		expect(summary.notableFeatures).toEqual({
			"shader-f16": false,
			"timestamp-query": true,
		});
		expect(summary.limits).toEqual({
			maxBufferSize: 268_435_456,
			maxStorageBufferBindingSize: 134_217_728,
			maxTextureDimension2D: 8192,
		});
		expect(summary.isFallbackAdapter).toBe(false);
		expect(
			summariseWebGpuAdapter({ isFallbackAdapter: true }, null)
				.isFallbackAdapter,
		).toBe(true);
	});

	it("reports a browser without navigator.gpu as unsupported", async () => {
		vi.stubGlobal("navigator", {});
		expect((await probeWebGpu()).status).toBe("unsupported");
	});

	it("reads adapter.info and never keeps the adapter", async () => {
		const adapter = {
			info: { vendor: "apple", architecture: "metal-3" },
			features: new Set(["shader-f16"]),
			limits: { maxBufferSize: 1024 },
		};
		vi.stubGlobal("navigator", {
			gpu: { requestAdapter: async () => adapter },
		});
		const result = await probeWebGpu();
		expect(result.adapter).toEqual({
			vendor: "apple",
			architecture: "metal-3",
			device: null,
			description: null,
		});
		expect(result.notableFeatures["shader-f16"]).toBe(true);
		expect(JSON.parse(JSON.stringify(result))).toEqual(result);
	});

	it("falls back to requestAdapterInfo() on older browsers", async () => {
		const requestAdapterInfo = vi.fn(async () => ({ vendor: "intel" }));
		vi.stubGlobal("navigator", {
			gpu: { requestAdapter: async () => ({ requestAdapterInfo }) },
		});
		const result = await probeWebGpu();
		expect(requestAdapterInfo).toHaveBeenCalledOnce();
		expect(result.adapter?.vendor).toBe("intel");
	});

	it("separates no adapter, a hanging request and a failing request", async () => {
		vi.stubGlobal("navigator", { gpu: { requestAdapter: async () => null } });
		expect((await probeWebGpu()).status).toBe("no-adapter");

		vi.stubGlobal("navigator", {
			gpu: { requestAdapter: () => new Promise(() => {}) },
		});
		expect((await probeWebGpu(10)).status).toBe("timeout");

		vi.stubGlobal("navigator", {
			gpu: {
				requestAdapter: () => {
					throw new Error("GPU process crashed");
				},
			},
		});
		const failed = await probeWebGpu();
		expect(failed.status).toBe("error");
		expect(failed.error).toBe("GPU process crashed");
	});
});

describe("GPU names", () => {
	it("extracts the device from ANGLE renderer strings", () => {
		expect(
			simplifyGpuName(
				"ANGLE (Apple, ANGLE Metal Renderer: Apple M3 Pro, Unspecified Version)",
			),
		).toBe("Apple M3 Pro");
		expect(
			simplifyGpuName(
				"ANGLE (NVIDIA, NVIDIA GeForce RTX 3080 (0x00002206) Direct3D11 vs_5_0 ps_5_0, D3D11)",
			),
		).toBe("NVIDIA GeForce RTX 3080");
		expect(simplifyGpuName("Apple GPU")).toBe("Apple GPU");
		expect(simplifyGpuName(null)).toBeNull();
	});

	it("falls back to the WebGPU adapter when WebGL has no renderer", () => {
		const graphics = {
			webgpu: {
				...summariseWebGpuAdapter(
					{},
					{ vendor: "apple", architecture: "metal-3" },
				),
			},
			webgl: {
				webgl: false,
				webgl2: false,
				version: null,
				vendor: null,
				renderer: null,
				unmasked: false,
				maxTextureSize: null,
			},
			offscreenCanvas: false,
		};
		expect(gpuDisplayName(graphics)).toBe("apple metal-3");
	});
});

describe("browser identity", () => {
	it("prefers the real brand over Chromium and GREASE brands", () => {
		const identity = identityFromClientHints(
			{
				brands: [{ brand: "Chromium", version: "141" }],
				mobile: false,
				platform: "Windows",
			},
			{
				platformVersion: "15.0.0",
				architecture: "x86",
				bitness: "64",
				model: "",
				fullVersionList: [
					{ brand: "Not)A;Brand", version: "8.0.0.0" },
					{ brand: "Chromium", version: "141.0.7390.55" },
					{ brand: "Google Chrome", version: "141.0.7390.55" },
				],
			},
		);
		expect(identity).toMatchObject({
			name: "Google Chrome",
			version: "141.0.7390.55",
			os: "Windows",
			osVersion: "11 (15.0.0)",
			architecture: "x86",
			bitness: "64",
			model: null,
			mobile: false,
			clientHints: true,
		});
	});

	it("uses low-entropy brands when high-entropy hints are missing", () => {
		const identity = identityFromClientHints(
			{
				brands: [
					{ brand: "Chromium", version: "141" },
					{ brand: "Not_A Brand", version: "24" },
				],
				platform: "Linux",
			},
			null,
		);
		expect(identity.name).toBe("Chromium");
		expect(identity.version).toBe("141");
		expect(identity.osVersion).toBeNull();
	});

	it("parses common user agents without client hints", () => {
		expect(
			identityFromUserAgent(
				"Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:144.0) Gecko/20100101 Firefox/144.0",
			),
		).toMatchObject({ name: "Firefox", version: "144.0", os: "macOS" });
		expect(
			identityFromUserAgent(
				"Mozilla/5.0 (iPhone; CPU iPhone OS 18_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.6 Mobile/15E148 Safari/604.1",
			),
		).toMatchObject({
			name: "Safari",
			version: "18.6",
			os: "iOS",
			osVersion: "18.6",
			mobile: true,
		});
		expect(
			identityFromUserAgent(
				"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36 Edg/141.0.0.0",
			),
		).toMatchObject({
			name: "Microsoft Edge",
			os: "Windows",
			osVersion: "NT 10.0",
		});
		expect(identityFromUserAgent(null).name).toBeNull();
	});
});

describe("storage probe", () => {
	it("reads usage, quota, Chromium usage details and persistence", async () => {
		vi.stubGlobal("navigator", {
			storage: {
				estimate: async () => ({
					usage: 2048,
					quota: 4096,
					usageDetails: { indexedDB: 2000, caches: 48, bogus: "x" },
				}),
				persisted: async () => true,
			},
		});
		expect(await probeStorage()).toEqual({
			usage: 2048,
			quota: 4096,
			usageDetails: { indexedDB: 2000, caches: 48 },
			persisted: true,
			error: null,
		});
	});

	it("reports a missing or failing storage manager as an error", async () => {
		vi.stubGlobal("navigator", {});
		const missing = await probeStorage();
		expect(missing.usage).toBeNull();
		expect(missing.error).toContain("estimate");

		vi.stubGlobal("navigator", {
			storage: {
				estimate: async () => {
					throw new Error("quota manager unavailable");
				},
			},
		});
		expect((await probeStorage()).error).toBe("quota manager unavailable");
	});
});

describe("capabilities", () => {
	it("lists every capability once with permission and OPFS details", async () => {
		vi.stubGlobal("navigator", {
			storage: {
				getDirectory: async () => {
					throw new Error("SecurityError");
				},
			},
			locks: { request: () => undefined },
		});
		vi.stubGlobal(
			"Notification",
			Object.assign(() => undefined, { permission: "denied" }),
		);
		const capabilities = await probeCapabilities();
		const byId = new Map(capabilities.map((entry) => [entry.id, entry]));
		expect(byId.size).toBe(capabilities.length);
		expect(byId.get("opfs")).toEqual({
			id: "opfs",
			available: false,
			detail: "SecurityError",
		});
		expect(byId.get("notifications")).toEqual({
			id: "notifications",
			available: true,
			detail: "permission=denied",
		});
		expect(byId.get("web-locks")?.available).toBe(true);
		expect(byId.get("webassembly")?.available).toBe(true);
		expect(typeof byId.get("wasm-threads")?.available).toBe("boolean");
	});
});

describe("collectDiagnostics", () => {
	it("survives globals that throw on every access", async () => {
		vi.stubGlobal("navigator", throwingObject);
		vi.stubGlobal("WebAssembly", throwingObject);
		vi.stubGlobal("screen", throwingObject);
		for (const name of [
			"location",
			"matchMedia",
			"devicePixelRatio",
			"innerWidth",
			"crossOriginIsolated",
			"OffscreenCanvas",
			"Notification",
		]) {
			throwingGetter(name);
		}
		const report = await collectDiagnostics({}, { webGpuTimeoutMs: 10 });
		expect(report.browser.name).toBeNull();
		expect(report.environment.origin).toBeNull();
		expect(report.graphics.webgpu.status).toBe("unsupported");
		expect(report.storage.usage).toBeNull();
		expect(
			report.capabilities.find((entry) => entry.id === "webassembly")
				?.available,
		).toBe(false);
		expect(JSON.parse(JSON.stringify(report))).toEqual(report);
		expect(formatDiagnosticsReport(report)).toContain("## Capabilities");
	});

	it("merges caller-provided build, API and session context", async () => {
		const report = await collectDiagnostics(
			{
				build: {
					version: "0.1.13",
					commit: "",
					builtAt: null,
					mode: "production",
				},
				apiBaseUrl: "https://api.example.com",
			},
			{ webGpuTimeoutMs: 10 },
		);
		expect(report.build).toEqual({
			version: "0.1.13",
			commit: null,
			builtAt: null,
			mode: "production",
		});
		expect(report.environment.apiBaseUrl).toBe("https://api.example.com");
		expect(report.session).toBeNull();

		const session = {
			signedIn: true,
			accountId: "user-1",
			developerMode: false,
			language: "en",
			boardRendererPreference: "egui" as const,
			boardRendererInUse: "react-flow" as const,
		};
		const merged = withDiagnosticsContext(report, { session });
		expect(merged.session).toEqual(session);
		expect(merged.build).toBe(report.build);
		expect(merged.environment.apiBaseUrl).toBe("https://api.example.com");
	});
});
