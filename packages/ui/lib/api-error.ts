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
	if (candidate.code === "PURCHASE_REQUIRED") return false;
	return (
		candidate.status === 402 ||
		candidate.code === "PAYMENT_REQUIRED" ||
		candidate.code === "PLAN_LIMIT_EXCEEDED"
	);
}

/**
 * True when the backend says the addressed resource is not there (404) or is
 * deliberately no longer served (410). Callers use it to tell "the request
 * failed" apart from "there is nothing left to act on".
 */
export function isMissingResourceError(
	error: unknown,
): error is ApiResponseError {
	if (typeof error !== "object" || error === null) return false;
	const candidate = error as Partial<ApiResponseError>;
	return candidate.status === 404 || candidate.status === 410;
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
		window.dispatchEvent(new window.CustomEvent(PLAN_LIMIT_EVENT, { detail: error }));
	}
	return error;
}
