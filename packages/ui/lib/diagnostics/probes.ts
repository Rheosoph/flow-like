import {
	identityFromUserAgent,
	probeBrowser,
	probeEnvironment,
	probeHardware,
} from "./browser";
import { probeCapabilities } from "./capabilities";
import {
	WEBGPU_ADAPTER_TIMEOUT_MS,
	emptyGraphics,
	probeGraphics,
} from "./graphics";
import {
	TIMED_OUT,
	attempt,
	attemptAsync,
	errorMessage,
	finiteNumber,
	nonEmptyString,
	readGlobal,
	readProperty,
	withTimeout,
} from "./safe";
import type {
	DiagnosticsBrowser,
	DiagnosticsBuild,
	DiagnosticsEnvironment,
	DiagnosticsReport,
	DiagnosticsSession,
	DiagnosticsStorage,
} from "./types";

export interface DiagnosticsContext {
	readonly build?: {
		readonly [Key in keyof DiagnosticsBuild]?: string | null | undefined;
	};
	readonly apiBaseUrl?: string | null;
	readonly session?: DiagnosticsSession | null;
}

export interface DiagnosticsOptions {
	readonly webGpuTimeoutMs?: number;
}

const STORAGE_TIMEOUT_MS = 2000;

const EMPTY_BUILD: DiagnosticsBuild = {
	version: null,
	commit: null,
	builtAt: null,
	mode: null,
};

const EMPTY_ENVIRONMENT: DiagnosticsEnvironment = {
	origin: null,
	apiBaseUrl: null,
	secureContext: null,
	crossOriginIsolated: null,
};

const EMPTY_STORAGE: DiagnosticsStorage = {
	usage: null,
	quota: null,
	usageDetails: null,
	persisted: null,
	error: null,
};

function emptyBrowser(): DiagnosticsBrowser {
	return {
		...identityFromUserAgent(null),
		userAgent: null,
		languages: [],
		timeZone: null,
		online: null,
		cookieEnabled: null,
		viewport: null,
		screen: null,
		devicePixelRatio: null,
		colorScheme: null,
		reducedMotion: null,
	};
}

async function readEstimate(storage: unknown): Promise<unknown> {
	const estimate = readProperty<() => Promise<unknown>>(storage, "estimate");
	if (typeof estimate !== "function")
		throw new Error("navigator.storage.estimate() is unavailable");
	const result = await withTimeout(estimate.call(storage), STORAGE_TIMEOUT_MS);
	if (result === TIMED_OUT)
		throw new Error("navigator.storage.estimate() timed out");
	return result;
}

async function readPersisted(storage: unknown): Promise<boolean | null> {
	const persisted = readProperty<() => Promise<unknown>>(storage, "persisted");
	if (typeof persisted !== "function") return null;
	const result = await withTimeout(persisted.call(storage), STORAGE_TIMEOUT_MS);
	return typeof result === "boolean" ? result : null;
}

function readUsageDetails(value: unknown): Record<string, number> | null {
	if (value === null || typeof value !== "object") return null;
	const entries = Object.entries(value).filter(
		(entry): entry is [string, number] => finiteNumber(entry[1]) !== null,
	);
	return entries.length > 0 ? Object.fromEntries(entries) : null;
}

export async function probeStorage(): Promise<DiagnosticsStorage> {
	const storage = readProperty(readGlobal("navigator"), "storage");
	const [estimate, persisted] = await Promise.all([
		readEstimate(storage).then(
			(value) => ({ value, error: null }),
			(error: unknown) => ({ value: null, error: errorMessage(error) }),
		),
		attemptAsync(() => readPersisted(storage), null),
	]);
	return {
		usage: finiteNumber(readProperty(estimate.value, "usage")),
		quota: finiteNumber(readProperty(estimate.value, "quota")),
		usageDetails: attempt(
			() => readUsageDetails(readProperty(estimate.value, "usageDetails")),
			null,
		),
		persisted,
		error: estimate.error,
	};
}

function normaliseBuild(
	build: DiagnosticsContext["build"],
	fallback: DiagnosticsBuild,
): DiagnosticsBuild {
	if (!build) return fallback;
	return {
		version: nonEmptyString(build.version),
		commit: nonEmptyString(build.commit),
		builtAt: nonEmptyString(build.builtAt),
		mode: nonEmptyString(build.mode),
	};
}

export function withDiagnosticsContext(
	report: DiagnosticsReport,
	context: DiagnosticsContext,
): DiagnosticsReport {
	return {
		...report,
		build: normaliseBuild(context.build, report.build),
		environment: {
			...report.environment,
			apiBaseUrl:
				context.apiBaseUrl === undefined
					? report.environment.apiBaseUrl
					: nonEmptyString(context.apiBaseUrl),
		},
		session: context.session === undefined ? report.session : context.session,
	};
}

export async function collectDiagnostics(
	context: DiagnosticsContext = {},
	options: DiagnosticsOptions = {},
): Promise<DiagnosticsReport> {
	const webGpuTimeoutMs = options.webGpuTimeoutMs ?? WEBGPU_ADAPTER_TIMEOUT_MS;
	const [browser, graphics, capabilities, storage] = await Promise.all([
		attemptAsync(probeBrowser, emptyBrowser()),
		attemptAsync(() => probeGraphics(webGpuTimeoutMs), emptyGraphics()),
		attemptAsync(probeCapabilities, []),
		attemptAsync(probeStorage, EMPTY_STORAGE),
	]);
	const report: DiagnosticsReport = {
		schemaVersion: 1,
		generatedAt: new Date().toISOString(),
		build: EMPTY_BUILD,
		environment: attempt(probeEnvironment, EMPTY_ENVIRONMENT),
		session: null,
		browser,
		hardware: attempt(probeHardware, {
			logicalCores: null,
			deviceMemoryGb: null,
			maxTouchPoints: null,
		}),
		graphics,
		capabilities,
		storage,
	};
	return withDiagnosticsContext(report, context);
}
