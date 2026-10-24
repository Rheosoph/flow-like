import type { BoardRenderer } from "../../hooks/use-board-renderer";
import { humanFileSize } from "../utils";
import { CAPABILITY_LABELS } from "./capabilities";
import { gpuDisplayName } from "./graphics";
import {
	type DiagnosticsBrowser,
	type DiagnosticsReport,
	type DiagnosticsSize,
	type DiagnosticsStorage,
	type DiagnosticsWebGpu,
	NOTABLE_WEBGPU_FEATURES,
	WEBGPU_LIMIT_NAMES,
	type WebGpuLimitName,
	type WebGpuStatus,
} from "./types";

type Line = readonly [label: string, value: string];
type SectionLines = (report: DiagnosticsReport) => Line[];

const UNKNOWN = "unknown";
const BYTE_LIMITS = new Set<WebGpuLimitName>([
	"maxBufferSize",
	"maxStorageBufferBindingSize",
	"maxComputeWorkgroupStorageSize",
]);

export const BOARD_RENDERER_NAMES: Readonly<Record<BoardRenderer, string>> = {
	egui: "WASM board (egui)",
	"react-flow": "React Flow",
};

const WEBGPU_STATUS_TEXT: Readonly<Record<WebGpuStatus, string>> = {
	available: "available",
	unsupported: "not supported (navigator.gpu missing)",
	"no-adapter": "no adapter",
	timeout: "requestAdapter() timed out",
	error: "error",
};

function text(value: string | number | null | undefined): string {
	if (value === null || value === undefined || value === "") return UNKNOWN;
	return String(value);
}

function yesNo(value: boolean | null | undefined): string {
	if (value === null || value === undefined) return UNKNOWN;
	return value ? "yes" : "no";
}

function bytes(value: number | null | undefined): string {
	return value === null || value === undefined ? UNKNOWN : humanFileSize(value);
}

function joined(...parts: readonly (string | null | undefined)[]) {
	return parts.filter(Boolean).join(" ") || null;
}

export function formatSize(size: DiagnosticsSize | null): string | null {
	return size ? `${size.width} × ${size.height}` : null;
}

export function shortCommit(commit: string | null): string | null {
	return commit ? commit.slice(0, 7) : null;
}

export function describeBrowser(browser: DiagnosticsBrowser): string | null {
	return joined(browser.name, browser.version);
}

export function describeOs(browser: DiagnosticsBrowser): string | null {
	return joined(browser.os, browser.osVersion);
}

export function describeArchitecture(
	browser: DiagnosticsBrowser,
): string | null {
	return joined(
		browser.architecture,
		browser.bitness ? `${browser.bitness}-bit` : null,
	);
}

export function formatWebGpuLimit(name: WebGpuLimitName, value: number) {
	return BYTE_LIMITS.has(name)
		? `${humanFileSize(value)} (${value})`
		: String(value);
}

export function storageUsagePercent(
	usage: number | null,
	quota: number | null,
): number | null {
	if (usage === null || quota === null || quota <= 0) return null;
	return Math.min(100, (usage / quota) * 100);
}

function section(title: string, lines: readonly Line[]): string {
	return [
		`## ${title}`,
		...lines.map(([label, value]) => `${label}: ${value}`),
	].join("\n");
}

const buildLines: SectionLines = ({ build, environment }) => [
	["Version", text(build.version)],
	["Commit", text(build.commit)],
	["Built at", text(build.builtAt)],
	["Mode", text(build.mode)],
	["Origin", text(environment.origin)],
	["API base URL", text(environment.apiBaseUrl)],
	["Secure context", yesNo(environment.secureContext)],
	["Cross-origin isolated", yesNo(environment.crossOriginIsolated)],
];

const rendererName = (value: BoardRenderer | null) =>
	value ? BOARD_RENDERER_NAMES[value] : UNKNOWN;

const sessionLines: SectionLines = ({ session }) => {
	if (!session) return [["Session", "not collected"]];
	return [
		["Signed in", yesNo(session.signedIn)],
		["Account ID", text(session.accountId)],
		["Developer mode", yesNo(session.developerMode)],
		["UI language", text(session.language)],
		[
			"Board renderer preference",
			rendererName(session.boardRendererPreference),
		],
		["Board renderer in use", rendererName(session.boardRendererInUse)],
	];
};

const browserLines: SectionLines = ({ browser }) => {
	return [
		["Browser", text(describeBrowser(browser))],
		["OS", text(describeOs(browser))],
		["Architecture", text(describeArchitecture(browser))],
		["Model", text(browser.model)],
		["Mobile", yesNo(browser.mobile)],
		["Client hints", yesNo(browser.clientHints)],
		["User agent", text(browser.userAgent)],
		["Languages", text(browser.languages.join(", "))],
		["Time zone", text(browser.timeZone)],
		["Online", yesNo(browser.online)],
		["Cookies enabled", yesNo(browser.cookieEnabled)],
		["Viewport", text(formatSize(browser.viewport))],
		["Screen", text(formatSize(browser.screen))],
		["Device pixel ratio", text(browser.devicePixelRatio)],
		["Color scheme", text(browser.colorScheme)],
		["Reduced motion", yesNo(browser.reducedMotion)],
	];
};

const hardwareLines: SectionLines = ({ hardware }) => [
	["Logical cores", text(hardware.logicalCores)],
	[
		"Device memory",
		hardware.deviceMemoryGb === null
			? UNKNOWN
			: `${hardware.deviceMemoryGb} GB`,
	],
	["Max touch points", text(hardware.maxTouchPoints)],
];

const webGpuStatusLine = (webgpu: DiagnosticsWebGpu): Line => [
	"WebGPU",
	text(
		joined(
			WEBGPU_STATUS_TEXT[webgpu.status],
			webgpu.isFallbackAdapter ? "(fallback adapter)" : null,
			webgpu.error ? `- ${webgpu.error}` : null,
		),
	),
];

const webGpuLimitLine = (
	webgpu: DiagnosticsWebGpu,
	name: WebGpuLimitName,
): Line => {
	const value = webgpu.limits[name];
	return [name, value === undefined ? UNKNOWN : formatWebGpuLimit(name, value)];
};

const webGpuLines: SectionLines = ({ graphics: { webgpu } }) => {
	if (webgpu.status !== "available") return [webGpuStatusLine(webgpu)];
	const adapter = webgpu.adapter;
	return [
		webGpuStatusLine(webgpu),
		["WebGPU vendor", text(adapter?.vendor)],
		["WebGPU architecture", text(adapter?.architecture)],
		["WebGPU device", text(adapter?.device)],
		["WebGPU description", text(adapter?.description)],
		["WebGPU fallback adapter", yesNo(webgpu.isFallbackAdapter)],
		...NOTABLE_WEBGPU_FEATURES.map(
			(feature): Line => [feature, yesNo(webgpu.notableFeatures[feature])],
		),
		...WEBGPU_LIMIT_NAMES.map((name) => webGpuLimitLine(webgpu, name)),
		[
			`WebGPU features (${webgpu.features.length})`,
			text(webgpu.features.join(", ")),
		],
	];
};

const webGlLines: SectionLines = ({ graphics }) => {
	const { webgl } = graphics;
	return [
		["GPU", text(gpuDisplayName(graphics))],
		["WebGL", yesNo(webgl.webgl)],
		["WebGL2", yesNo(webgl.webgl2)],
		["WebGL version", text(webgl.version)],
		["WebGL vendor", text(webgl.vendor)],
		[
			"WebGL renderer",
			text(
				joined(
					webgl.renderer,
					webgl.renderer && !webgl.unmasked ? "(masked)" : null,
				),
			),
		],
		["Max texture size", text(webgl.maxTextureSize)],
		["OffscreenCanvas", yesNo(graphics.offscreenCanvas)],
	];
};

const capabilityLines: SectionLines = ({ capabilities }) =>
	capabilities.map(({ id, available, detail }) => [
		CAPABILITY_LABELS[id],
		detail ? `${yesNo(available)} (${detail})` : yesNo(available),
	]);

const usageText = ({ usage, quota }: DiagnosticsStorage) => {
	const percent = storageUsagePercent(usage, quota);
	return percent === null
		? bytes(usage)
		: `${bytes(usage)} (${percent.toFixed(1)}% of quota)`;
};

const storageLines: SectionLines = ({ storage }) => {
	const details = Object.entries(storage.usageDetails ?? {})
		.map(([name, value]) => `${name} ${humanFileSize(value)}`)
		.join(", ");
	const lines: Line[] = [
		["Usage", usageText(storage)],
		["Quota", bytes(storage.quota)],
		["Usage details", text(details)],
		["Persisted", yesNo(storage.persisted)],
	];
	return storage.error ? [...lines, ["Error", storage.error]] : lines;
};

export function formatDiagnosticsReport(report: DiagnosticsReport): string {
	return [
		["# Flow-Like diagnostics", `Generated at: ${report.generatedAt}`].join(
			"\n",
		),
		section("Build", buildLines(report)),
		section("Session", sessionLines(report)),
		section("Browser", browserLines(report)),
		section("Hardware", hardwareLines(report)),
		section("Graphics", [...webGlLines(report), ...webGpuLines(report)]),
		section("Capabilities", capabilityLines(report)),
		section("Storage", storageLines(report)),
	]
		.join("\n\n")
		.concat("\n");
}
