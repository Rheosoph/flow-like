import type {
	FlwEnvelope,
	QueryResultPayload,
	WidgetSizing,
} from "@flow-like/widget-sdk";
import {
	DEFAULT_LIGHT_TOKENS,
	createEnvelope,
	isFlwEnvelope,
} from "@flow-like/widget-sdk";
import {
	isDesktopWidgetGrant,
	isWebWidgetGrant,
	isWidgetRuntimeComponent,
} from "./micro-widget-policy";

/**
 * Host-side flw/1 bridge logic for micro widgets. Everything in this module is
 * pure (no DOM, no React) so the protocol rules — envelope filtering, props
 * diffing, URL construction, rate limiting, height clamping, query
 * correlation — stay unit-testable. `A2UIMicroWidget` wires these primitives
 * to the actual iframe.
 */

export const MICRO_WIDGET_DEFAULT_HEIGHT = 320;
export const MICRO_WIDGET_READY_TIMEOUT_MS = 10_000;
export const MICRO_WIDGET_RATE_LIMIT_PER_SECOND = 30;
/** Contract events per iframe (burst and sustained per second). Resize keeps MICRO_WIDGET_RATE_LIMIT_PER_SECOND. */
export const MICRO_WIDGET_EVENT_BURST = 60;
export const MICRO_WIDGET_EVENT_RATE_PER_SECOND = 60;
export const MICRO_WIDGET_QUERY_TIMEOUT_MS = 10_000;

/** Whitelisted theme token names forwarded into the widget sandbox. */
export const MICRO_WIDGET_THEME_TOKENS: readonly string[] =
	Object.keys(DEFAULT_LIGHT_TOKENS);

export { createEnvelope };
export type { FlwEnvelope };

export function generateNonce(): string {
	const bytes = new Uint8Array(16);
	crypto.getRandomValues(bytes);
	return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join(
		"",
	);
}

/**
 * Tauri rewrites custom URI schemes to `http://{scheme}.localhost/…` on
 * platforms whose webview cannot register arbitrary schemes (Windows WebView2,
 * Android). Everywhere else the plain `{scheme}://localhost/…` form works.
 */
export function shouldUseHttpSchemeBridge(userAgent: string): boolean {
	return /windows|android/i.test(userAgent);
}

/** Frame path segment for a widget that runs without a grant. */
export const MICRO_WIDGET_BASELINE_GRANT = "0";

const DESKTOP_WIDGET_ORIGIN = "flow-widget://localhost";
const DESKTOP_WIDGET_BRIDGE_ORIGIN = "http://flow-widget.localhost";

function grantSegment(
	grant: string | null,
	isGrant: (value: string) => boolean,
	subject: string,
): string {
	if (grant === null) return MICRO_WIDGET_BASELINE_GRANT;
	if (!isGrant(grant)) {
		throw new Error(
			`Refusing to build a frame URL for ${subject}: malformed grant`,
		);
	}
	return grant;
}

export interface DesktopMicroWidgetFrameParts {
	packageId: string;
	bundleHash: string;
	widgetId: string;
	/** Minted grant id, or null for the baseline frame. */
	grant: string | null;
	useHttpBridge: boolean;
}

/**
 * The host embeds a wrapper document (`frame/{widgetId}/{grant|0}`) rather
 * than the widget document itself. The wrapper pins its child to the exact
 * document the grant resolves to and relays envelopes, so the host still
 * addresses `iframe.contentWindow` and widgets still address `window.parent`.
 * Capabilities, hosts and downloads come only from the grant the backend
 * resolves; nothing in the URL can widen them.
 */
export function buildDesktopMicroWidgetFrameSrc({
	packageId,
	bundleHash,
	widgetId,
	grant,
	useHttpBridge,
}: DesktopMicroWidgetFrameParts): string {
	const segment = grantSegment(
		grant,
		isDesktopWidgetGrant,
		`${packageId}/${widgetId}`,
	);
	const path = `${encodeURIComponent(packageId)}/${encodeURIComponent(
		bundleHash,
	)}/frame/${encodeURIComponent(widgetId)}/${segment}`;
	return `${useHttpBridge ? DESKTOP_WIDGET_BRIDGE_ORIGIN : DESKTOP_WIDGET_ORIGIN}/${path}`;
}

export interface WebMicroWidgetFrameParts {
	packageId: string;
	packageVersion: string;
	widgetId: string;
	/** Minted grant token, or null for the baseline frame. */
	grant: string | null;
	/** Runtime component from the same mint; only ever next to a grant. */
	runtime?: string | null;
}

/**
 * API path (relative to the backend `/api/v1` base, see `getApiUrl`) of the
 * web wrapper: `frame/{wid}/{grant|0}`, or `frame/{wid}/{grant}~{runtime}`
 * when the grant carries runtime sources.
 */
export function buildWebMicroWidgetFramePath({
	packageId,
	packageVersion,
	widgetId,
	grant,
	runtime,
}: WebMicroWidgetFrameParts): string {
	const subject = `${packageId}@${packageVersion}/${widgetId}`;
	let segment = grantSegment(grant, isWebWidgetGrant, subject);
	if (runtime !== undefined && runtime !== null) {
		if (grant === null || !isWidgetRuntimeComponent(runtime)) {
			throw new Error(
				`Refusing to build a frame URL for ${subject}: malformed runtime component`,
			);
		}
		segment = `${segment}~${runtime}`;
	}
	return `registry/package/${encodeURIComponent(
		packageId,
	)}/widget-sandbox/${encodeURIComponent(
		packageVersion,
	)}/frame/${encodeURIComponent(widgetId)}/${segment}`;
}

const WIDGET_SERVING_HOST = "flow-widget.localhost";
const WIDGET_SERVING_ROUTES = new Set(["widget-asset", "widget-sandbox"]);

function decodePath(pathname: string): string | null {
	let current = pathname;
	for (let round = 0; round < 4; round++) {
		let decoded: string;
		try {
			decoded = decodeURIComponent(current);
		} catch {
			return null;
		}
		if (decoded === current) return current;
		current = decoded;
	}
	return null;
}

/** Dot segments are kept: dropping `widget-sandbox/..` here must not hide the route. */
function pathSegments(path: string): string[] {
	return path
		.toLowerCase()
		.split(/[/\\]+/)
		.filter((segment) => segment !== "");
}

/**
 * True for any URL that reaches a package widget document, wrapper or asset:
 * the desktop `flow-widget` scheme and its http bridge host, and the API
 * `widget-asset` / `widget-sandbox` routes on any origin. Such a URL must only
 * ever be framed by the micro widget host, which owns consent. Input that
 * cannot be parsed or decoded counts as a serving URL so callers fail closed.
 */
export function isMicroWidgetServingUrl(url: string, base?: string): boolean {
	let parsed: URL;
	try {
		parsed = new URL(url, base);
	} catch {
		return true;
	}
	if (parsed.protocol === "flow-widget:") return true;
	const host = parsed.hostname.toLowerCase().replace(/\.+$/, "");
	if (
		host === WIDGET_SERVING_HOST ||
		host.endsWith(`.${WIDGET_SERVING_HOST}`)
	) {
		return true;
	}
	const decoded = decodePath(parsed.pathname);
	if (decoded === null) return true;
	const segments = pathSegments(decoded);
	for (let index = 0; index + 1 < segments.length; index++) {
		if (segments[index] !== "registry" || segments[index + 1] !== "package") {
			continue;
		}
		if (
			segments
				.slice(index + 2)
				.some((segment) => WIDGET_SERVING_ROUTES.has(segment))
		) {
			return true;
		}
	}
	return false;
}

export interface DesktopMicroWidgetSrcParts {
	packageId: string;
	bundleHash: string;
	widgetId: string;
	useHttpBridge: boolean;
	/** Adds `allow-downloads` to the inner frame on hosts that predate grants. */
	allowDownloads?: boolean;
}

function frameQuery(allowDownloads: boolean | undefined): string {
	return allowDownloads ? "?downloads=1" : "";
}

/**
 * Legacy desktop wrapper URL (`frame/{widgetId}`), only for backends that
 * cannot describe widget policies. Current backends serve it as the baseline
 * frame and ignore the query.
 */
export function buildDesktopMicroWidgetSrc({
	packageId,
	bundleHash,
	widgetId,
	useHttpBridge,
	allowDownloads,
}: DesktopMicroWidgetSrcParts): string {
	const path = `${encodeURIComponent(packageId)}/${encodeURIComponent(
		bundleHash,
	)}/frame/${encodeURIComponent(widgetId)}${frameQuery(allowDownloads)}`;
	return `${useHttpBridge ? DESKTOP_WIDGET_BRIDGE_ORIGIN : DESKTOP_WIDGET_ORIGIN}/${path}`;
}

/**
 * Legacy web wrapper path on the `widget-asset` route, only for API servers
 * that cannot describe widget policies.
 */
export function buildWebMicroWidgetPath(
	packageId: string,
	packageVersion: string,
	widgetId: string,
	allowDownloads?: boolean,
): string {
	return `registry/package/${encodeURIComponent(
		packageId,
	)}/widget-asset/${encodeURIComponent(
		packageVersion,
	)}/frame/${encodeURIComponent(widgetId)}${frameQuery(allowDownloads)}`;
}

/** Elements-payload key mirroring a micro widget's `value:changed` state. */
export function microWidgetValuesKey(instanceId: string): string {
	return `${instanceId}/values`;
}

/**
 * Collect the `"{instanceId}/values"` keys for every micro widget instance in
 * a surface's component map, so stored value mirrors survive the elements
 * merge even though they are not prefixed with the surface id.
 */
export function collectMicroWidgetValueKeys(
	components: Record<string, { component?: unknown }> | undefined,
): Set<string> {
	const keys = new Set<string>();
	if (!components) return keys;
	for (const comp of Object.values(components)) {
		const data = comp?.component as Record<string, unknown> | undefined;
		if (data?.type !== "microWidgetInstance") continue;
		const instanceId = data.instanceId;
		if (typeof instanceId === "string" && instanceId.length > 0) {
			keys.add(microWidgetValuesKey(instanceId));
		}
	}
	return keys;
}

/**
 * Validate an inbound widget→host message. Returns the envelope when it passes
 * the protocol checks, null otherwise. `hello` is the only message allowed to
 * carry an empty nonce (it is sent before the widget has learned it).
 * `event.source` identity must be checked by the caller — it needs the iframe.
 */
export function acceptHostEnvelope(
	data: unknown,
	instanceId: string,
	nonce: string,
): FlwEnvelope | null {
	if (!isFlwEnvelope(data)) return null;
	if (data.type === "hello") {
		if (data.nonce !== "" && data.nonce !== nonce) return null;
		return data;
	}
	if (data.nonce !== nonce) return null;
	if (data.instanceId !== instanceId) return null;
	return data;
}

/**
 * Shallow per-key diff between the last sent props and the next props, with
 * JSON-stringify equality per key (a2ui resolve() re-parses literalJson into
 * fresh objects every render, so identity comparison would always differ).
 * Returns only changed/added keys, or null when nothing changed. Removed keys
 * are sent as undefined; postMessage's structured clone preserves the key and
 * the widget SDK treats undefined as an optional-value deletion.
 */
export function diffMicroWidgetProps(
	prev: Record<string, unknown>,
	next: Record<string, unknown>,
): Record<string, unknown> | null {
	const patch: Record<string, unknown> = {};
	let changed = false;
	for (const [key, value] of Object.entries(next)) {
		if (!(key in prev) || JSON.stringify(prev[key]) !== JSON.stringify(value)) {
			patch[key] = value;
			changed = true;
		}
	}
	for (const key of Object.keys(prev)) {
		if (!(key in next)) {
			patch[key] = undefined;
			changed = true;
		}
	}
	return changed ? patch : null;
}

/** Clamp a widget-requested height against its contract sizing. */
export function clampWidgetHeight(
	height: number,
	sizing: WidgetSizing | null | undefined,
): number {
	if (!Number.isFinite(height) || height < 0) {
		return sizing?.defaultHeight ?? MICRO_WIDGET_DEFAULT_HEIGHT;
	}
	const max = sizing?.maxHeight;
	return typeof max === "number" && height > max ? max : Math.ceil(height);
}

/** Simple token bucket: `capacity` tokens, refilled at `refillPerSecond`. */
export class TokenBucket {
	private tokens: number;
	private lastRefill: number;

	constructor(
		private readonly capacity: number = MICRO_WIDGET_RATE_LIMIT_PER_SECOND,
		private readonly refillPerSecond: number = MICRO_WIDGET_RATE_LIMIT_PER_SECOND,
	) {
		this.tokens = capacity;
		this.lastRefill = 0;
	}

	tryTake(nowMs: number = Date.now()): boolean {
		const elapsed = Math.max(0, nowMs - this.lastRefill);
		this.lastRefill = nowMs;
		this.tokens = Math.min(
			this.capacity,
			this.tokens + (elapsed / 1000) * this.refillPerSecond,
		);
		if (this.tokens < 1) return false;
		this.tokens -= 1;
		return true;
	}
}

/**
 * Read the whitelisted theme tokens through an accessor (in the browser:
 * `getComputedStyle(document.documentElement).getPropertyValue`).
 */
export function readThemeTokens(
	getValue: (name: string) => string,
	names: readonly string[] = MICRO_WIDGET_THEME_TOKENS,
): Record<string, string> {
	const tokens: Record<string, string> = {};
	for (const name of names) {
		const value = getValue(name)?.trim();
		if (value) tokens[name] = value;
	}
	return tokens;
}

interface PendingQuery {
	resolve: (value: unknown) => void;
	reject: (error: Error) => void;
	timer: ReturnType<typeof setTimeout>;
}

export interface QueryCorrelator {
	request: (
		name: string,
		args: unknown,
		timeoutMs?: number,
	) => Promise<unknown>;
	handleResult: (payload: QueryResultPayload) => void;
	dispose: () => void;
}

/**
 * Correlates host→widget `query` messages with widget→host `query:result`
 * replies by queryId, with a per-request timeout.
 */
export function createQueryCorrelator(
	post: (payload: { queryId: string; name: string; args: unknown }) => void,
): QueryCorrelator {
	const pending = new Map<string, PendingQuery>();
	let counter = 0;

	const request = (
		name: string,
		args: unknown,
		timeoutMs: number = MICRO_WIDGET_QUERY_TIMEOUT_MS,
	): Promise<unknown> => {
		const queryId = `q${++counter}-${Date.now().toString(36)}`;
		return new Promise<unknown>((resolve, reject) => {
			const timer = setTimeout(() => {
				pending.delete(queryId);
				reject(
					new Error(
						`Micro widget query "${name}" timed out after ${timeoutMs}ms`,
					),
				);
			}, timeoutMs);
			pending.set(queryId, { resolve, reject, timer });
			post({ queryId, name, args });
		});
	};

	const handleResult = (payload: QueryResultPayload) => {
		const entry = pending.get(payload.queryId);
		if (!entry) return;
		pending.delete(payload.queryId);
		clearTimeout(entry.timer);
		if (payload.ok) {
			entry.resolve(payload.value);
		} else {
			entry.reject(new Error(payload.error ?? "Micro widget query failed"));
		}
	};

	const dispose = () => {
		for (const [queryId, entry] of pending) {
			clearTimeout(entry.timer);
			entry.reject(new Error("Micro widget bridge disposed"));
			pending.delete(queryId);
		}
	};

	return { request, handleResult, dispose };
}

export interface MicroWidgetBridgeHandle {
	query: (name: string, args: unknown, timeoutMs?: number) => Promise<unknown>;
}

const liveBridges = new Map<string, MicroWidgetBridgeHandle>();

/**
 * Register a live micro-widget bridge for imperative host access (run/query
 * bridge). Returns an unregister function; a later registration for the same
 * instance id wins (remount replaces the stale handle).
 */
export function registerMicroWidgetBridge(
	instanceId: string,
	handle: MicroWidgetBridgeHandle,
): () => void {
	liveBridges.set(instanceId, handle);
	return () => {
		if (liveBridges.get(instanceId) === handle) {
			liveBridges.delete(instanceId);
		}
	};
}

/** Whether a live (mounted) micro widget bridge exists for the instance. */
export function microWidgetHasInstance(instanceId: string): boolean {
	return liveBridges.has(instanceId);
}

/**
 * Run a contract query against a live micro widget instance. Rejects when the
 * instance is not mounted, the widget reports an error, or the timeout hits.
 */
export function microWidgetQuery(
	instanceId: string,
	name: string,
	args: unknown,
	timeoutMs: number = MICRO_WIDGET_QUERY_TIMEOUT_MS,
): Promise<unknown> {
	const bridge = liveBridges.get(instanceId);
	if (!bridge) {
		return Promise.reject(
			new Error(`No live micro widget instance "${instanceId}"`),
		);
	}
	return bridge.query(name, args, timeoutMs);
}
