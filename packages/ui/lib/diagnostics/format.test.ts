import { describe, expect, it } from "vitest";
import {
	describeArchitecture,
	formatDiagnosticsReport,
	formatWebGpuLimit,
	shortCommit,
	storageUsagePercent,
} from "./format";
import type { DiagnosticsReport } from "./types";

const FIXTURE: DiagnosticsReport = {
	schemaVersion: 1,
	generatedAt: "2026-10-22T10:00:00.000Z",
	build: {
		version: "0.1.13",
		commit: "76cce5ff0abc",
		builtAt: "2026-10-21T08:00:00.000Z",
		mode: "production",
	},
	environment: {
		origin: "https://app.flow-like.com",
		apiBaseUrl: "https://api.flow-like.com",
		secureContext: true,
		crossOriginIsolated: false,
	},
	session: {
		signedIn: true,
		accountId: "user-123",
		developerMode: false,
		language: "de",
		boardRendererPreference: "egui",
		boardRendererInUse: "react-flow",
	},
	browser: {
		name: "Google Chrome",
		version: "141.0.7390.55",
		os: "macOS",
		osVersion: "15.1.0",
		architecture: "arm",
		bitness: "64",
		model: null,
		mobile: false,
		clientHints: true,
		brands: [{ brand: "Google Chrome", version: "141.0.7390.55" }],
		userAgent: "Mozilla/5.0",
		languages: ["de-DE", "en"],
		timeZone: "Europe/Berlin",
		online: true,
		cookieEnabled: true,
		viewport: { width: 1440, height: 900 },
		screen: { width: 1512, height: 982 },
		devicePixelRatio: 2,
		colorScheme: "dark",
		reducedMotion: false,
	},
	hardware: { logicalCores: 12, deviceMemoryGb: 8, maxTouchPoints: 0 },
	graphics: {
		webgpu: {
			status: "available",
			error: null,
			adapter: {
				vendor: "apple",
				architecture: "metal-3",
				device: null,
				description: null,
			},
			isFallbackAdapter: false,
			features: ["shader-f16", "timestamp-query"],
			notableFeatures: { "shader-f16": true, "timestamp-query": true },
			limits: { maxBufferSize: 4_294_967_296, maxTextureDimension2D: 16384 },
		},
		webgl: {
			webgl: true,
			webgl2: true,
			version: "WebGL 2.0 (OpenGL ES 3.0 Chromium)",
			vendor: "Google Inc. (Apple)",
			renderer:
				"ANGLE (Apple, ANGLE Metal Renderer: Apple M3 Pro, Unspecified Version)",
			unmasked: true,
			maxTextureSize: 16384,
		},
		offscreenCanvas: true,
	},
	capabilities: [
		{ id: "webassembly", available: true },
		{
			id: "wasm-threads",
			available: false,
			detail: "SharedArrayBuffer=true, crossOriginIsolated=false",
		},
		{ id: "opfs", available: null },
	],
	storage: {
		usage: 1024 * 1024,
		quota: 4 * 1024 * 1024,
		usageDetails: { indexedDB: 1024 * 1024 },
		persisted: false,
		error: null,
	},
};

describe("formatDiagnosticsReport", () => {
	it("renders every section as Markdown headings with Label: value lines", () => {
		const text = formatDiagnosticsReport(FIXTURE);
		const headings = text.split("\n").filter((line) => line.startsWith("#"));
		expect(headings).toEqual([
			"# Flow-Like diagnostics",
			"## Build",
			"## Session",
			"## Browser",
			"## Hardware",
			"## Graphics",
			"## Capabilities",
			"## Storage",
		]);
		expect(text).toContain("Generated at: 2026-10-22T10:00:00.000Z");
		expect(text).toContain("Version: 0.1.13");
		expect(text).toContain("Commit: 76cce5ff0abc");
		expect(text).toContain("API base URL: https://api.flow-like.com");
		expect(text).toContain("Account ID: user-123");
		expect(text).toContain("Board renderer in use: React Flow");
		expect(text).toContain("Browser: Google Chrome 141.0.7390.55");
		expect(text).toContain("Architecture: arm 64-bit");
		expect(text).toContain("Viewport: 1440 × 900");
		expect(text).toContain("GPU: Apple M3 Pro");
		expect(text).toContain("WebGPU: available");
		expect(text).toContain("maxBufferSize: 4.0 GiB (4294967296)");
		expect(text).toContain("maxStorageBufferBindingSize: unknown");
		expect(text).toContain("WebGPU features (2): shader-f16, timestamp-query");
		expect(text).toContain(
			"WebAssembly threads: no (SharedArrayBuffer=true, crossOriginIsolated=false)",
		);
		expect(text).toContain("Origin Private File System: unknown");
		expect(text).toContain("Usage: 1.0 MiB (25.0% of quota)");
		expect(text).toContain("Usage details: indexedDB 1.0 MiB");
		expect(text).toContain("Persisted: no");
		expect(text.endsWith("\n")).toBe(true);
	});

	it("explains a missing WebGPU adapter and a missing session", () => {
		const text = formatDiagnosticsReport({
			...FIXTURE,
			session: null,
			graphics: {
				...FIXTURE.graphics,
				webgpu: {
					...FIXTURE.graphics.webgpu,
					status: "error",
					error: "GPU process crashed",
					adapter: null,
				},
			},
			storage: { ...FIXTURE.storage, usage: null, error: "blocked" },
		});
		expect(text).toContain("Session: not collected");
		expect(text).toContain("WebGPU: error - GPU process crashed");
		expect(text).not.toContain("WebGPU vendor");
		expect(text).toContain("Usage: unknown");
		expect(text).toContain("Error: blocked");
	});
});

describe("format helpers", () => {
	it("formats limits, commits, architecture and usage", () => {
		expect(formatWebGpuLimit("maxTextureDimension2D", 8192)).toBe("8192");
		expect(formatWebGpuLimit("maxBufferSize", 1024)).toBe("1.0 KiB (1024)");
		expect(shortCommit("76cce5ff0abc")).toBe("76cce5f");
		expect(shortCommit(null)).toBeNull();
		expect(describeArchitecture(FIXTURE.browser)).toBe("arm 64-bit");
		expect(storageUsagePercent(50, 200)).toBe(25);
		expect(storageUsagePercent(10, 0)).toBeNull();
		expect(storageUsagePercent(null, 10)).toBeNull();
	});
});
