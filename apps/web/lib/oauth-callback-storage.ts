import type {
	IOAuthPendingAuth,
	IStoredOAuthToken,
} from "@tm9657/flow-like-ui";

export const OAUTH_CALLBACK_CHANNEL = "flow-like-oauth";
export const OAUTH_CALLBACK_PENDING_KEY = "oauth-callback-pending";
export const OAUTH_CALLBACK_MAX_AGE_MS = 10 * 60 * 1000;

export interface IStoredOAuthCallback {
	url: string;
	code: string | null;
	state: string | null;
	id_token: string | null;
	access_token: string | null;
	token_type: string | null;
	expires_in: string | null;
	scope: string | null;
	timestamp: number;
}

export interface IStoredOAuthCallbackCompletion {
	pending: IOAuthPendingAuth;
	token: IStoredOAuthToken;
	timestamp: number;
}

type OAuthCallbackPayload = Omit<IStoredOAuthCallback, "timestamp">;

function getStorage(type: "local" | "session"): Storage | null {
	if (typeof window === "undefined") {
		return null;
	}

	try {
		return type === "local" ? window.localStorage : window.sessionStorage;
	} catch {
		return null;
	}
}

function createBroadcastChannel(): BroadcastChannel | null {
	if (
		typeof window === "undefined" ||
		typeof BroadcastChannel === "undefined"
	) {
		return null;
	}

	try {
		return new BroadcastChannel(OAUTH_CALLBACK_CHANNEL);
	} catch {
		return null;
	}
}

export function clearPendingOAuthCallback() {
	getStorage("local")?.removeItem(OAUTH_CALLBACK_PENDING_KEY);
	getStorage("session")?.removeItem(OAUTH_CALLBACK_PENDING_KEY);
}

export function storePendingOAuthCallback(payload: OAuthCallbackPayload) {
	const serialized = JSON.stringify({
		...payload,
		timestamp: Date.now(),
	} satisfies IStoredOAuthCallback);

	try {
		getStorage("session")?.setItem(OAUTH_CALLBACK_PENDING_KEY, serialized);
	} catch {
		// Ignore storage write failures; the callback event still carries the payload.
	}
}

export function broadcastOAuthCallbackCompletion(
	pending: IOAuthPendingAuth,
	token: IStoredOAuthToken,
) {
	const payload = {
		pending,
		token,
		timestamp: Date.now(),
	} satisfies IStoredOAuthCallbackCompletion;

	const channel = createBroadcastChannel();
	try {
		channel?.postMessage({
			type: "oauth-complete",
			payload,
		});
	} finally {
		channel?.close();
	}
}

export function readPendingOAuthCallback(): IStoredOAuthCallback | null {
	const rawValue = getStorage("session")?.getItem(OAUTH_CALLBACK_PENDING_KEY);
	if (!rawValue) {
		return null;
	}

	try {
		const parsed = JSON.parse(rawValue) as IStoredOAuthCallback;
		if (Date.now() - parsed.timestamp > OAUTH_CALLBACK_MAX_AGE_MS) {
			clearPendingOAuthCallback();
			return null;
		}

		return parsed;
	} catch {
		clearPendingOAuthCallback();
		return null;
	}
}

export function parseOAuthCallbackCompletion(
	rawValue: string,
): IStoredOAuthCallbackCompletion | null {
	try {
		const parsed = JSON.parse(rawValue) as IStoredOAuthCallbackCompletion;
		if (Date.now() - parsed.timestamp > OAUTH_CALLBACK_MAX_AGE_MS) {
			return null;
		}

		return parsed;
	} catch {
		return null;
	}
}
