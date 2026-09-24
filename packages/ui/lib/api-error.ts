import type { QuotaDetail } from "./quota";
export const PLAN_LIMIT_EVENT = "flow-like:plan-limit";

export interface ApiResponseErrorOptions {
	status: number;
	statusText?: string;
	message: string;
	code?: string;
	errorId?: string;
	path?: string;
	quota?: QuotaDetail;
}

const CAPABILITY_PATH_SEGMENTS = [
	/((?:^|\/)team\/link\/join\/)[^/?#]+/g,
	/((?:^|\/)solution\/track\/)[^/?#]+/g,
];

/** Remove query data and bearer-capability path segments before a path reaches diagnostics. */
export function redactApiPathSecrets(path?: string): string | undefined {
	if (!path) return path;
	let redacted: string;
	try {
		const url = new URL(path);
		redacted = `${url.origin}${url.pathname}`;
	} catch {
		redacted = path.split(/[?#]/, 1)[0] ?? path;
	}
	for (const pattern of CAPABILITY_PATH_SEGMENTS) {
		redacted = redacted.replace(pattern, "$1[REDACTED]");
	}
	return redacted;
}

/**
 * Error returned by the FlowLike API. Keep the public correlation metadata on
 * the error object so background tasks, audit reports, and UI error boundaries
 * can report the same server-side failure without parsing a console string.
 */
export class ApiResponseError extends Error {
	readonly status: number;
	readonly statusText?: string;
	readonly code?: string;
	readonly errorId?: string;
	readonly path?: string;
	/** The server's message without the `[CODE]` prefix — safe to show to users. */
	readonly serverMessage: string;
	readonly quota?: QuotaDetail;

	constructor(options: ApiResponseErrorOptions) {
		const label = options.code || `HTTP_${options.status}`;
		const reference = options.errorId ? `; ref ${options.errorId}` : "";
		super(`[${label}${reference}] ${options.message}`);
		this.name = "ApiResponseError";
		this.status = options.status;
		this.statusText = options.statusText;
		this.code = options.code;
		this.errorId = options.errorId;
		this.path = options.path;
		this.serverMessage = options.message;
		this.quota = options.quota;
	}

	toJSON() {
		return {
			name: this.name,
			message: this.message,
			status: this.status,
			statusText: this.statusText,
			code: this.code,
			errorId: this.errorId,
			path: this.path,
			quota: this.quota,
		};
	}
}

/** Metadata safe for logs. Response text and path capabilities are omitted or redacted. */
export function apiErrorDiagnostic(error: ApiResponseError) {
	return {
		name: error.name,
		status: error.status,
		code: error.code,
		errorId: error.errorId,
		path: redactApiPathSecrets(error.path),
	};
}

const MARKETPLACE_PURCHASE_CODES: ReadonlySet<string> = new Set([
	"PURCHASE_REQUIRED",
	"PACKAGE_LICENSE_REQUIRED",
]);

/**
 * True when a 402 asks the caller to buy a marketplace item (an app, or a
 * package before pinning it to a project) rather than to upgrade their plan.
 */
export function isPurchaseRequiredError(
	error: unknown,
): error is ApiResponseError {
	if (typeof error !== "object" || error === null) return false;
	const { code } = error as Partial<ApiResponseError>;
	return typeof code === "string" && MARKETPLACE_PURCHASE_CODES.has(code);
}

/**
 * True when the backend rejected the request because the user's plan does not
 * cover it (HTTP 402 / PAYMENT_REQUIRED). Callers route these into the upgrade
 * dialog instead of a plain error toast.
 */
export function isUpgradeRequiredError(
	error: unknown,
): error is ApiResponseError {
	if (typeof error !== "object" || error === null) return false;
	const candidate = error as Partial<ApiResponseError>;
	if (isPurchaseRequiredError(error)) return false;
	return (
		candidate.status === 402 ||
		candidate.code === "PAYMENT_REQUIRED" ||
		candidate.code === "PLAN_LIMIT_EXCEEDED"
	);
}

/**
 * True when the backend says the addressed resource is not there (404) or is
 * deliberately no longer served (410). Callers use it to tell "the request
 * failed" apart from "there is nothing left to act on", and some delete local
 * copies on it. Every API error carries a `code`; a bare 404 comes from a proxy,
 * a CDN or a hub without the route, and says nothing about the resource.
 */
export function isMissingResourceError(
	error: unknown,
): error is ApiResponseError {
	if (typeof error !== "object" || error === null) return false;
	const candidate = error as Partial<ApiResponseError>;
	return (
		(candidate.status === 404 || candidate.status === 410) &&
		typeof candidate.code === "string" &&
		candidate.code.length > 0
	);
}

/**
 * The server's own explanation when it sent one, otherwise the caller's generic
 * copy. Backends that distinguish failure cases (already a member vs. already
 * invited) are only useful if the UI shows what they said.
 */
export function apiErrorMessage(error: unknown, fallback: string): string {
	return error instanceof ApiResponseError && error.serverMessage.trim()
		? error.serverMessage
		: fallback;
}

function nonEmptyString(value: unknown): string | undefined {
	return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

export function apiResponseError(
	response: Pick<Response, "status" | "statusText" | "headers">,
	body: string,
	path?: string,
): ApiResponseError {
	let code: string | undefined;
	let errorId: string | undefined;
	let message: string | undefined;
	let quota: QuotaDetail | undefined;

	if (body) {
		try {
			const parsed = JSON.parse(body) as Record<string, unknown>;
			const nested =
				parsed.error && typeof parsed.error === "object"
					? (parsed.error as Record<string, unknown>)
					: undefined;
			const quotaValue = nested?.quota ?? parsed.quota;
			if (
				quotaValue &&
				typeof quotaValue === "object" &&
				typeof (quotaValue as QuotaDetail).resource === "string"
			)
				quota = quotaValue as QuotaDetail;
			code = nonEmptyString(nested?.code) ?? nonEmptyString(parsed.code);
			errorId = nonEmptyString(nested?.id) ?? nonEmptyString(parsed.id);
			message =
				nonEmptyString(nested?.message) ??
				nonEmptyString(parsed.message) ??
				nonEmptyString(parsed.error);
		} catch {
			message = nonEmptyString(body);
		}
	}

	if (response.status === 426) {
		message = message
			? `${message} Update FlowLike to a version that supports this board, then reopen it.`
			: "Update FlowLike to a version that supports this board, then reopen it.";
		code ??= "CLIENT_UPGRADE_REQUIRED";
	}

	errorId =
		errorId ??
		nonEmptyString(response.headers.get("x-error-id")) ??
		nonEmptyString(response.headers.get("x-request-id"));
	message =
		message ||
		nonEmptyString(response.statusText) ||
		`HTTP request failed with status ${response.status}`;

	const error = new ApiResponseError({
		status: response.status,
		statusText: nonEmptyString(response.statusText),
		message,
		code,
		errorId,
		path,
		quota,
	});
	if (typeof window !== "undefined" && isUpgradeRequiredError(error)) {
		window.dispatchEvent(
			new window.CustomEvent(PLAN_LIMIT_EVENT, { detail: error }),
		);
	}
	return error;
}

/**
 * A request that timed out or never reached the server says nothing about the
 * caller's access or the resource: only a server refusal does.
 */
export function isTransportFailure(error: unknown): boolean {
	if (!error || error instanceof ApiResponseError) return false;
	const { name, message } = error as { name?: unknown; message?: unknown };
	if (name === "RequestTimeoutError") return true;
	if (typeof message !== "string") return false;
	return (
		message.startsWith("Network unavailable") ||
		message.includes("Failed to fetch") ||
		message.includes("NetworkError") ||
		message.includes("Network request failed") ||
		message.includes("fetch failed") ||
		message.includes("Load failed")
	);
}

/**
 * The hub did not rule on the request: it was unreachable, timed out, overloaded
 * or broken. Local-first paths fall back to the device's copy on this; a refusal
 * (401/403/404/…) is a verdict and must not be worked around.
 */
export function isHubUnavailable(error: unknown): boolean {
	if (isTransportFailure(error)) return true;
	const status = (error as { status?: unknown } | null)?.status;
	return (
		typeof status === "number" &&
		(status >= 500 || status === 408 || status === 429)
	);
}

export const UPSTREAM_UNAVAILABLE_CODE = "UPSTREAM_UNAVAILABLE";

/**
 * A 2xx carrying a body the API never sends on success. A Lambda whose runtime
 * died (startup panic, OOM, timeout) still answers its Function URL with 200 and
 * `{errorType, errorMessage}`; captive portals and proxies answer with an HTML
 * page. Returned as data, either reaches every caller in the wrong shape, so
 * both become a 502 — the status offline and transient paths already handle.
 *
 * `data` is the body as the caller parsed it: the JSON value, or the raw text.
 */
export function upstreamFailureInSuccess(
	response: Pick<Response, "status" | "headers">,
	data: unknown,
	path?: string,
): ApiResponseError | undefined {
	const requestId =
		nonEmptyString(response.headers.get("x-amzn-requestid")) ??
		nonEmptyString(response.headers.get("x-request-id"));
	if (response.headers.get("content-type")?.includes("text/html")) {
		return new ApiResponseError({
			status: 502,
			code: UPSTREAM_UNAVAILABLE_CODE,
			message: `Expected API data but received an HTML page (HTTP ${response.status})`,
			errorId: requestId,
			path,
		});
	}
	if (
		typeof data === "object" &&
		data !== null &&
		typeof (data as { errorType?: unknown }).errorType === "string" &&
		typeof (data as { errorMessage?: unknown }).errorMessage === "string"
	) {
		return new ApiResponseError({
			status: 502,
			code: UPSTREAM_UNAVAILABLE_CODE,
			message: `The API runtime failed (${(data as { errorType: string }).errorType})`,
			errorId: requestId,
			path,
		});
	}
	return undefined;
}
