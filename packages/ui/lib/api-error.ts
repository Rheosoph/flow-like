export interface ApiResponseErrorOptions {
	status: number;
	statusText?: string;
	message: string;
	code?: string;
	errorId?: string;
	path?: string;
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
		};
	}
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
	return candidate.status === 402 || candidate.code === "PAYMENT_REQUIRED";
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

	if (body) {
		try {
			const parsed = JSON.parse(body) as Record<string, unknown>;
			const nested =
				parsed.error && typeof parsed.error === "object"
					? (parsed.error as Record<string, unknown>)
					: undefined;
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

	errorId =
		errorId ??
		nonEmptyString(response.headers.get("x-error-id")) ??
		nonEmptyString(response.headers.get("x-request-id"));
	message =
		message ||
		nonEmptyString(response.statusText) ||
		`HTTP request failed with status ${response.status}`;

	return new ApiResponseError({
		status: response.status,
		statusText: nonEmptyString(response.statusText),
		message,
		code,
		errorId,
		path,
	});
}
