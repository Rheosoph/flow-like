import type { BoardRenderer } from "../../hooks/use-board-renderer";

export interface DiagnosticsBuild {
	readonly version: string | null;
	readonly commit: string | null;
	readonly builtAt: string | null;
	readonly mode: string | null;
}

export interface DiagnosticsEnvironment {
	readonly origin: string | null;
	readonly apiBaseUrl: string | null;
	readonly secureContext: boolean | null;
	readonly crossOriginIsolated: boolean | null;
}

export interface DiagnosticsSession {
	readonly signedIn: boolean | null;
	readonly accountId: string | null;
	readonly developerMode: boolean | null;
	readonly language: string | null;
	readonly boardRendererPreference: BoardRenderer | null;
	readonly boardRendererInUse: BoardRenderer | null;
}

export interface DiagnosticsSize {
	readonly width: number;
	readonly height: number;
}

export interface DiagnosticsBrand {
	readonly brand: string;
	readonly version: string;
}

export interface DiagnosticsBrowser {
	readonly name: string | null;
	readonly version: string | null;
	readonly os: string | null;
	readonly osVersion: string | null;
	readonly architecture: string | null;
	readonly bitness: string | null;
	readonly model: string | null;
	readonly mobile: boolean | null;
	readonly clientHints: boolean;
	readonly brands: readonly DiagnosticsBrand[];
	readonly userAgent: string | null;
	readonly languages: readonly string[];
	readonly timeZone: string | null;
	readonly online: boolean | null;
	readonly cookieEnabled: boolean | null;
	readonly viewport: DiagnosticsSize | null;
	readonly screen: DiagnosticsSize | null;
	readonly devicePixelRatio: number | null;
	readonly colorScheme: "light" | "dark" | null;
	readonly reducedMotion: boolean | null;
}

export interface DiagnosticsHardware {
	readonly logicalCores: number | null;
	readonly deviceMemoryGb: number | null;
	readonly maxTouchPoints: number | null;
}

export type WebGpuStatus =
	| "available"
	| "unsupported"
	| "no-adapter"
	| "timeout"
	| "error";

export const NOTABLE_WEBGPU_FEATURES = [
	"shader-f16",
	"timestamp-query",
] as const;

export type NotableWebGpuFeature = (typeof NOTABLE_WEBGPU_FEATURES)[number];

export const WEBGPU_LIMIT_NAMES = [
	"maxBufferSize",
	"maxStorageBufferBindingSize",
	"maxComputeWorkgroupStorageSize",
	"maxTextureDimension2D",
] as const;

export type WebGpuLimitName = (typeof WEBGPU_LIMIT_NAMES)[number];

export interface WebGpuAdapterSummary {
	readonly vendor: string | null;
	readonly architecture: string | null;
	readonly device: string | null;
	readonly description: string | null;
}

export interface DiagnosticsWebGpu {
	readonly status: WebGpuStatus;
	readonly error: string | null;
	readonly adapter: WebGpuAdapterSummary | null;
	readonly isFallbackAdapter: boolean | null;
	readonly features: readonly string[];
	readonly notableFeatures: Readonly<Record<NotableWebGpuFeature, boolean>>;
	readonly limits: Readonly<Partial<Record<WebGpuLimitName, number>>>;
}

export interface DiagnosticsWebGl {
	readonly webgl: boolean;
	readonly webgl2: boolean;
	readonly version: string | null;
	readonly vendor: string | null;
	readonly renderer: string | null;
	readonly unmasked: boolean;
	readonly maxTextureSize: number | null;
}

export interface DiagnosticsGraphics {
	readonly webgpu: DiagnosticsWebGpu;
	readonly webgl: DiagnosticsWebGl;
	readonly offscreenCanvas: boolean;
}

export const DIAGNOSTICS_CAPABILITY_IDS = [
	"webassembly",
	"wasm-simd",
	"wasm-threads",
	"web-workers",
	"service-workers",
	"indexeddb",
	"opfs",
	"compression-stream",
	"broadcast-channel",
	"web-locks",
	"scheduler-yield",
	"clipboard",
	"notifications",
	"websocket",
	"webtransport",
	"webcodecs",
] as const;

export type DiagnosticsCapabilityId =
	(typeof DIAGNOSTICS_CAPABILITY_IDS)[number];

export interface DiagnosticsCapability {
	readonly id: DiagnosticsCapabilityId;
	readonly available: boolean | null;
	readonly detail?: string;
}

export interface DiagnosticsStorage {
	readonly usage: number | null;
	readonly quota: number | null;
	readonly usageDetails: Readonly<Record<string, number>> | null;
	readonly persisted: boolean | null;
	readonly error: string | null;
}

export interface DiagnosticsReport {
	readonly schemaVersion: 1;
	readonly generatedAt: string;
	readonly build: DiagnosticsBuild;
	readonly environment: DiagnosticsEnvironment;
	readonly session: DiagnosticsSession | null;
	readonly browser: DiagnosticsBrowser;
	readonly hardware: DiagnosticsHardware;
	readonly graphics: DiagnosticsGraphics;
	readonly capabilities: readonly DiagnosticsCapability[];
	readonly storage: DiagnosticsStorage;
}
