export interface FrontendToolApprovalScopeInput {
	requestId: string;
	toolName: string;
	arguments: Record<string, unknown>;
	approvalKind?: string;
	approvalSessionKey?: string;
	contextAppId?: string;
}

export interface FrontendToolApprovalScope {
	sessionKey: string;
	rememberable: boolean;
}

function nonEmptyString(value: unknown): string | undefined {
	if (typeof value !== "string") return undefined;
	return value.trim() ? value : undefined;
}

function argumentString(
	args: Record<string, unknown>,
	snakeCase: string,
	camelCase: string,
): string | undefined {
	return nonEmptyString(args[snakeCase]) ?? nonEmptyString(args[camelCase]);
}

function scopePart(value: string): string {
	return encodeURIComponent(value);
}

/**
 * Resolve the frontend session allowlist key for one tool request. Page interaction approvals are
 * reusable only when both the app and exact Event/page target are known. Every other tool retains
 * its existing backend-provided key (or tool/kind fallback).
 */
export function resolveFrontendToolApprovalScope({
	requestId,
	toolName,
	arguments: args,
	approvalKind,
	approvalSessionKey,
	contextAppId,
}: FrontendToolApprovalScopeInput): FrontendToolApprovalScope {
	if (toolName !== "interact_app_page") {
		return {
			sessionKey: approvalSessionKey || `${toolName}:${approvalKind ?? "none"}`,
			rememberable: true,
		};
	}

	const appId =
		argumentString(args, "app_id", "appId") ?? nonEmptyString(contextAppId);
	const eventId = argumentString(args, "event_id", "eventId");
	const pageId = argumentString(args, "page_id", "pageId");
	const target = eventId
		? { kind: "event", id: eventId }
		: pageId
			? { kind: "page", id: pageId }
			: undefined;

	if (appId && target) {
		return {
			sessionKey: `interact_app_page:${scopePart(appId)}:${target.kind}:${scopePart(target.id)}`,
			rememberable: true,
		};
	}

	return {
		sessionKey: `interact_app_page:request:${scopePart(requestId)}`,
		rememberable: false,
	};
}
