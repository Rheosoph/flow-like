import {
	type AIProvider,
	isAgentBackendProvider,
	normalizeAIProvider,
} from "../../components/flowpilot/types";
import { ApiResponseError } from "../api-error";
import {
	type AgentBackendDiagnostic,
	classifyAgentBackendError,
	isAgentBackendCancellation,
	redactErrorText,
} from "./agent-backend-diagnostics";

/**
 * What went wrong, at the granularity the UI reacts to: the icon, the tone and the recovery
 * affordance are all chosen from this — never from the raw error string.
 */
export type ChatErrorKind =
	| "cancelled"
	| "config"
	| "billing"
	| "auth"
	| "permission"
	| "rate-limit"
	| "network"
	| "input"
	| "server"
	| "backend"
	| "generic";

/** The one recovery affordance the card offers, when the failure has an obvious next step. */
export interface IChatErrorAction {
	/** `navigate` pushes `href`; `upgrade` opens the plan-upgrade dialog. */
	kind: "navigate" | "upgrade";
	label: string;
	href?: string;
}

/**
 * A failed assistant turn, structured. Stored on the message so a reloaded conversation renders the
 * same card, and kept flat + bounded because it goes through Dexie on every checkpoint.
 */
export interface IChatMessageError {
	kind: ChatErrorKind;
	title: string;
	message: string;
	/** Server code (`BAD_REQUEST`, `PAYMENT_REQUIRED`, …) or the backend diagnostic kind. */
	code?: string;
	status?: number;
	/** Server-side incident id from the API error envelope — the support reference. */
	reference?: string;
	/** Terminal command that verifies or repairs an external agent backend. */
	command?: string;
	action?: IChatErrorAction;
	/** Redacted raw failure text, kept behind the technical-details disclosure. */
	detail?: string;
	retryable: boolean;
}

const MODEL_SETTINGS_ROUTE = "/settings/ai?tab=models";
const DETAIL_LIMIT = 1_000;

function providerLabel(provider: AIProvider | string): string {
	switch (normalizeAIProvider(provider as AIProvider)) {
		case "github-copilot":
			return "GitHub Copilot";
		case "codex":
			return "Codex";
		case "claude-code":
			return "Claude Code";
		default:
			return "FlowPilot";
	}
}

function errorText(error: unknown): string {
	if (typeof error === "string") return error.trim();
	if (error instanceof Error) return error.message.trim();
	if (error && typeof error === "object") {
		try {
			return JSON.stringify(error);
		} catch {
			return String(error);
		}
	}
	return error == null ? "" : String(error).trim();
}

function detailOf(error: unknown): string | undefined {
	const raw = redactErrorText(errorText(error)).slice(0, DETAIL_LIMIT);
	return raw || undefined;
}

/**
 * Recover the API error envelope from a failure that crossed a stringifying boundary (the Tauri IPC,
 * an older transport, a nested run). `{"error":{"code":…,"message":…}}` is the only shape the API
 * emits, so finding it anywhere in the text is enough to classify the failure properly.
 */
function parseEnvelope(
	text: string,
): { status?: number; code?: string; message: string; id?: string } | null {
	const start = text.indexOf("{");
	if (start === -1) return null;
	let parsed: unknown;
	try {
		parsed = JSON.parse(text.slice(start));
	} catch {
		return null;
	}
	if (!parsed || typeof parsed !== "object") return null;
	const body = (parsed as { error?: unknown }).error;
	if (!body || typeof body !== "object") return null;
	const candidate = body as Record<string, unknown>;
	const message =
		typeof candidate.message === "string" ? candidate.message.trim() : "";
	if (!message) return null;
	const status = /\((\d{3})\)/.exec(text)?.[1];
	return {
		status: status ? Number(status) : undefined,
		code: typeof candidate.code === "string" ? candidate.code : undefined,
		message,
		id: typeof candidate.id === "string" ? candidate.id : undefined,
	};
}

function isCancellation(error: unknown): boolean {
	if (isAgentBackendCancellation(error)) return true;
	const name = (error as { name?: unknown })?.name;
	if (name === "AbortError") return true;
	const normalized = errorText(error).toLowerCase();
	return (
		normalized.includes("aborted a request") ||
		normalized.includes("the operation was aborted") ||
		normalized.includes("signal is aborted")
	);
}

function isOffline(text: string): boolean {
	const normalized = text.toLowerCase();
	return (
		normalized.includes("failed to fetch") ||
		normalized.includes("networkerror") ||
		normalized.includes("network error") ||
		normalized.includes("load failed") ||
		normalized.includes("err_internet_disconnected") ||
		normalized.includes("connection refused") ||
		normalized.includes("econnrefused")
	);
}

/** Map an API rejection onto the card. Every branch keeps the server's own sentence as the body. */
function fromApi(
	label: string,
	envelope: { status?: number; code?: string; message: string; id?: string },
	detail?: string,
): IChatMessageError {
	const { status, code, message, id } = envelope;
	const base = {
		code,
		status,
		reference: id,
		detail,
	};
	const normalized = message.toLowerCase();

	if (status === 402 || code === "PAYMENT_REQUIRED") {
		return {
			...base,
			kind: "billing",
			title: "Your plan does not cover these models",
			message,
			action: { kind: "upgrade", label: "See plans" },
			retryable: false,
		};
	}

	if (
		normalized.includes("no language model") ||
		normalized.includes("settings → models") ||
		normalized.includes("no profile found")
	) {
		return {
			...base,
			kind: "config",
			title: "No model configured",
			message,
			action: {
				kind: "navigate",
				label: "Open model settings",
				href: MODEL_SETTINGS_ROUTE,
			},
			retryable: false,
		};
	}

	if (status === 401 || code === "UNAUTHORIZED") {
		return {
			...base,
			kind: "auth",
			title: "Sign in again",
			message: message || "This session is no longer signed in.",
			retryable: true,
		};
	}

	if (status === 403 || code === "FORBIDDEN") {
		return {
			...base,
			kind: "permission",
			title: `${label} is not available for this account`,
			message,
			retryable: false,
		};
	}

	if (status === 429 || code === "TOO_MANY_REQUESTS") {
		return {
			...base,
			kind: "rate-limit",
			title: "Too many requests",
			message: message || "Wait a moment before sending the next message.",
			retryable: true,
		};
	}

	if (
		status === 413 ||
		normalized.includes("too large") ||
		normalized.includes("too long")
	) {
		return {
			...base,
			kind: "input",
			title: "This request is too large",
			message,
			retryable: false,
		};
	}

	if (status !== undefined && status >= 500) {
		return {
			...base,
			kind: "server",
			title: `${label} could not complete the request`,
			message:
				"Something failed on our side. Try again in a moment — if it keeps happening, send us the reference below.",
			retryable: true,
		};
	}

	return {
		...base,
		kind: status !== undefined && status < 500 ? "input" : "generic",
		title: `${label} rejected the request`,
		message,
		retryable: true,
	};
}

const BACKEND_KINDS: Record<AgentBackendDiagnostic["kind"], ChatErrorKind> = {
	"cli-missing": "backend",
	"cli-path": "backend",
	"cli-not-executable": "backend",
	"local-permission": "backend",
	"local-environment": "backend",
	mcp: "backend",
	auth: "auth",
	"policy-billing": "permission",
	"rate-limit": "rate-limit",
	"network-timeout": "network",
	"model-version-protocol": "config",
	workflow: "generic",
	input: "input",
	generic: "generic",
};

/**
 * Turn whatever ended a turn into the card the chat renders: a title, one sentence the user can act
 * on, and the raw text kept aside for the details disclosure. Never throws — an unclassifiable
 * failure still gets a usable card.
 */
export function buildChatMessageError(
	provider: AIProvider | string,
	error: unknown,
): IChatMessageError {
	const label = providerLabel(provider);
	const detail = detailOf(error);

	if (isCancellation(error)) {
		return {
			kind: "cancelled",
			title: "Response stopped",
			message: "You stopped this response before it finished.",
			detail,
			retryable: true,
		};
	}

	if (error instanceof ApiResponseError) {
		return fromApi(
			label,
			{
				status: error.status,
				code: error.code,
				message: error.serverMessage,
				id: error.errorId,
			},
			detail,
		);
	}

	const text = errorText(error);
	const envelope = parseEnvelope(text);
	if (envelope) return fromApi(label, envelope, detail);

	if (isOffline(text)) {
		return {
			kind: "network",
			title: `Could not reach ${label}`,
			message:
				"Check your internet connection, then send the message again. A VPN or proxy can also block the connection.",
			detail,
			retryable: true,
		};
	}

	if (isAgentBackendProvider(normalizeAIProvider(provider as AIProvider))) {
		const diagnostic = classifyAgentBackendError(provider, error);
		if (diagnostic) {
			return {
				kind: BACKEND_KINDS[diagnostic.kind],
				title: diagnostic.title,
				message: diagnostic.message,
				code: diagnostic.kind,
				command: diagnostic.command,
				detail,
				retryable: diagnostic.retryable,
			};
		}
	}

	// A short, sentence-shaped failure is worth showing as-is; anything longer (a stack, a dumped
	// payload) belongs behind the details disclosure rather than in the card body.
	const readable = detail && detail.length <= 200 ? detail : undefined;
	return {
		kind: "generic",
		title: `${label} could not finish this response`,
		message:
			readable ??
			"The response ended unexpectedly. Send the message again to retry.",
		detail: readable ? undefined : detail,
		retryable: true,
	};
}
