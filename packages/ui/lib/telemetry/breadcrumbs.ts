/**
 * Breadcrumb trail attached to anonymous crash reports. Breadcrumbs never carry
 * user content: messages are truncated, URL-like tokens are reduced to
 * sanitized paths and secret-looking assignments are redacted before the
 * breadcrumb enters the ring buffer.
 */

import { sanitizeTelemetryPath } from "./page-view";

export type TelemetryBreadcrumbLevel =
	| "debug"
	| "info"
	| "warning"
	| "error"
	| "fatal";

export interface ITelemetryCapturedBreadcrumb {
	ts?: string;
	category?: string;
	message?: string;
	level?: TelemetryBreadcrumbLevel;
}

export interface ITelemetryCapturedBreadcrumbInput {
	category?: string;
	message?: string;
	level?: TelemetryBreadcrumbLevel;
}

const MAX_TELEMETRY_BREADCRUMBS = 30;
const MAX_BREADCRUMB_MESSAGE_LENGTH = 256;
const MAX_BREADCRUMB_CATEGORY_LENGTH = 64;
/** Slack over the output cap so redaction never scans an unbounded string. */
const SANITIZE_INPUT_OVERHEAD = 8;

const BREADCRUMB_LEVELS: readonly TelemetryBreadcrumbLevel[] = [
	"debug",
	"info",
	"warning",
	"error",
	"fatal",
];

const SECRET_ASSIGNMENT =
	/\b(pass(?:word|wd)?|pwd|secret|token|api[_-]?key|apikey|access[_-]?token|refresh[_-]?token|authorization|auth|bearer|session[_-]?id|signature|sig)\b\s*[:=]\s*("[^"]*"|'[^']*'|\S+)/gi;
const BEARER_PREFIX = /\b(bearer|basic)\s+[A-Za-z0-9._~+/=-]{8,}/gi;
const ABSOLUTE_URL = /^[a-z][a-z0-9+.-]*:\/\/\S+$/i;
const PATH_LIKE = /^\/\S*$/;

const BREADCRUMBS: ITelemetryCapturedBreadcrumb[] = [];

function sanitizeUrlToken(token: string): string {
	try {
		if (ABSOLUTE_URL.test(token)) {
			const url = new URL(token);
			return `${url.origin}${sanitizeTelemetryPath(url.pathname)}`;
		}
		if (PATH_LIKE.test(token)) return sanitizeTelemetryPath(token);
	} catch {
		return sanitizeTelemetryPath(token.split(/[?#]/, 1)[0] ?? token);
	}
	return token;
}

/** Redacts secrets and reduces URL-like tokens to anonymous paths. */
export function sanitizeTelemetryMessage(
	message: string,
	maxLength = MAX_BREADCRUMB_MESSAGE_LENGTH,
): string {
	const bounded =
		message.length > maxLength * SANITIZE_INPUT_OVERHEAD
			? message.slice(0, maxLength * SANITIZE_INPUT_OVERHEAD)
			: message;
	const redacted = bounded
		.replace(BEARER_PREFIX, "$1 [REDACTED]")
		.replace(SECRET_ASSIGNMENT, "$1=[REDACTED]");
	const anonymized = redacted
		.split(/(\s+)/)
		.map((token) =>
			token.trim().length === 0 ? token : sanitizeUrlToken(token),
		)
		.join("");
	return anonymized.length > maxLength
		? `${anonymized.slice(0, maxLength)}…`
		: anonymized;
}

function normalizeLevel(
	level: TelemetryBreadcrumbLevel | undefined,
): TelemetryBreadcrumbLevel | undefined {
	return level && BREADCRUMB_LEVELS.includes(level) ? level : undefined;
}

export function addTelemetryBreadcrumb(
	input: ITelemetryCapturedBreadcrumbInput,
) {
	try {
		const breadcrumb: ITelemetryCapturedBreadcrumb = {
			ts: new Date().toISOString(),
		};
		if (typeof input.category === "string" && input.category.length > 0) {
			breadcrumb.category = input.category.slice(
				0,
				MAX_BREADCRUMB_CATEGORY_LENGTH,
			);
		}
		if (typeof input.message === "string" && input.message.length > 0) {
			breadcrumb.message = sanitizeTelemetryMessage(input.message);
		}
		const level = normalizeLevel(input.level);
		if (level) breadcrumb.level = level;

		BREADCRUMBS.push(breadcrumb);
		if (BREADCRUMBS.length > MAX_TELEMETRY_BREADCRUMBS) {
			BREADCRUMBS.splice(0, BREADCRUMBS.length - MAX_TELEMETRY_BREADCRUMBS);
		}
	} catch {
		// Telemetry is best-effort and must never affect the application path.
	}
}

/** Snapshot of the current trail, oldest first. */
export function getTelemetryBreadcrumbs(): ITelemetryCapturedBreadcrumb[] {
	return BREADCRUMBS.map((breadcrumb) => ({ ...breadcrumb }));
}

export function clearTelemetryBreadcrumbs() {
	BREADCRUMBS.length = 0;
}
