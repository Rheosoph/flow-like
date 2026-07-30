/**
 * Anonymous server-side error capture for the Next.js Node and Edge runtimes.
 *
 * PRIVACY INVARIANT: identical to the browser ingest path. No account, no user
 * identity, no IP address and no request content. The anonymous id is minted
 * once per process and never persisted, so reports cannot be correlated across
 * restarts, deployments or users. A failing request is reduced to a sanitized
 * route path plus the HTTP method — query strings, headers, cookies and request
 * bodies never leave this module.
 */

import { getApiUrl } from "@flow-like/flow-like-ui/lib/api-url";
import {
	type ITelemetryCapturedFrame,
	normalizeError,
	parseErrorFrames,
} from "@flow-like/flow-like-ui/lib/telemetry/errors";
import { sanitizeTelemetryPath } from "@flow-like/flow-like-ui/lib/telemetry/page-view";

export interface IServerErrorRequest {
	readonly path?: string;
	readonly method?: string;
}

export interface IServerErrorContext {
	readonly routerKind?: string;
	readonly routePath?: string;
	readonly routeType?: string;
	readonly renderSource?: string;
}

const SOURCE = "web_server";
const ERROR_PATH = "telemetry/errors";
const MAX_CULPRIT_LENGTH = 200;
const MAX_ROUTE_LENGTH = 256;
const MAX_SHORT_LENGTH = 64;
const REQUEST_TIMEOUT_MS = 5_000;
const HTTP_METHODS = new Set([
	"GET",
	"HEAD",
	"POST",
	"PUT",
	"PATCH",
	"DELETE",
	"OPTIONS",
	"TRACE",
	"CONNECT",
]);

/**
 * Latched once the ingest answers 404, which is how the backend reports that
 * telemetry is disabled for this platform. No further requests are attempted.
 */
let ingestDisabled = false;

function randomAnonId(): string {
	try {
		if (
			typeof crypto !== "undefined" &&
			typeof crypto.randomUUID === "function"
		) {
			return crypto.randomUUID();
		}
	} catch {
		// Falls through to the arithmetic id below.
	}
	return `srv-${Math.random().toString(36).slice(2)}-${Date.now().toString(36)}`;
}

/**
 * Minted at module load, i.e. when the instrumentation hook boots. Anonymous by
 * construction: it identifies a process, never a person or an installation.
 */
const PROCESS_ANON_ID = randomAnonId();

function readEnv(name: string): string | undefined {
	try {
		const value = process.env?.[name];
		return typeof value === "string" && value.trim().length > 0
			? value.trim()
			: undefined;
	} catch {
		return undefined;
	}
}

/**
 * Must match the release the source-map upload is keyed to, otherwise server
 * stack traces stay unsymbolicated.
 */
function release(): string | undefined {
	return readEnv("FLOW_LIKE_RELEASE");
}

function runtime(): string {
	return sanitizeEnum(readEnv("NEXT_RUNTIME")) ?? "nodejs";
}

function truncated(value: string, max: number): string {
	return value.length <= max ? value : value.slice(0, max);
}

/**
 * Only a fixed vocabulary passes through; anything else is dropped rather than
 * forwarded, so a spoofed method can never smuggle content into the ingest.
 */
function sanitizeMethod(method: string | undefined): string | undefined {
	if (typeof method !== "string") return undefined;
	const upper = method.trim().toUpperCase();
	return HTTP_METHODS.has(upper) ? upper : undefined;
}

/**
 * The Next.js route template (`/app/[id]`) is a build-time constant and carries
 * no user data, so it is preferred over the concrete request path.
 */
function sanitizeRoute(route: string | undefined): string | undefined {
	if (typeof route !== "string" || route.trim().length === 0) return undefined;
	return truncated(sanitizeTelemetryPath(route.trim()), MAX_ROUTE_LENGTH);
}

function sanitizeEnum(value: string | undefined): string | undefined {
	if (typeof value !== "string") return undefined;
	const trimmed = value.trim();
	if (trimmed.length === 0 || !/^[a-z0-9 _-]+$/i.test(trimmed))
		return undefined;
	return truncated(trimmed, MAX_SHORT_LENGTH);
}

function buildPayload(
	error: unknown,
	request: IServerErrorRequest | undefined,
	context: IServerErrorContext | undefined,
) {
	const normalized = normalizeError(error);
	const path = sanitizeRoute(request?.path);
	const route = sanitizeRoute(context?.routePath) ?? path ?? "/";
	const method = sanitizeMethod(request?.method);
	const frames: ITelemetryCapturedFrame[] = parseErrorFrames(normalized.stack);
	const culprit = truncated(
		method ? `${method} ${route}` : route,
		MAX_CULPRIT_LENGTH,
	);

	return {
		anon_id: PROCESS_ANON_ID,
		source: SOURCE,
		app_version: null,
		release: release() ?? null,
		platform: runtime(),
		errors: [
			{
				kind: normalized.kind,
				value: normalized.value,
				level: "error",
				culprit,
				stacktrace: frames.length > 0 ? frames : undefined,
				context: {
					path: route,
					method: method ?? null,
					route_type: sanitizeEnum(context?.routeType) ?? null,
					router_kind: sanitizeEnum(context?.routerKind) ?? null,
					render_source: sanitizeEnum(context?.renderSource) ?? null,
					runtime: runtime(),
				},
				client_ts: new Date().toISOString(),
			},
		],
	};
}

async function postError(body: unknown): Promise<void> {
	const controller =
		typeof AbortController === "function" ? new AbortController() : undefined;
	const timer = controller
		? setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS)
		: undefined;
	try {
		const response = await fetch(getApiUrl(undefined, ERROR_PATH), {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(body),
			cache: "no-store",
			signal: controller?.signal,
		});
		if (response.status === 404) ingestDisabled = true;
		await response.body?.cancel().catch(() => undefined);
	} catch {
		// Telemetry is best-effort and must never affect the request path.
	} finally {
		if (timer !== undefined) clearTimeout(timer);
	}
}

/**
 * Fire-and-forget anonymous report of a server or edge request error. Never
 * throws and never awaits into the request path: callers can invoke it from
 * `onRequestError` without changing the response at all.
 */
export function captureServerRequestError(
	error: unknown,
	request?: IServerErrorRequest,
	context?: IServerErrorContext,
): void {
	try {
		if (ingestDisabled) return;
		void postError(buildPayload(error, request, context));
	} catch {
		// Telemetry is best-effort and must never affect the request path.
	}
}
