import type { IChannelClientDescriptor, IChannelPush } from "../schema/channel";
import {
	type ChannelPushOptions,
	errorMessage,
	readBodyExcerpt,
	timeoutSignal,
	unixSeconds,
} from "./util";

export const FIREBASE_PUSH_TIMEOUT_MS = 30_000;
const ID_TOKEN_REFRESH_MARGIN_MS = 60_000;
const SIGN_IN_URL =
	"https://identitytoolkit.googleapis.com/v1/accounts:signInWithCustomToken";

export type FirebaseRtdbChannelDescriptor = Extract<
	IChannelClientDescriptor,
	{ type: "gcp_firebase_rtdb" }
>;

interface FirebaseSession {
	customToken: string;
	idToken: string;
	expiresAtMs: number;
}

const sessions = new Map<string, FirebaseSession>();
const exchanges = new Map<string, Promise<FirebaseSession>>();

function normalizePath(path: string): string {
	const trimmed = path.replace(/\/+$/, "");
	return trimmed.startsWith("/") ? trimmed : `/${trimmed}`;
}

function databaseOrigin(url: string): string {
	return url.replace(/\/+$/, "");
}

async function exchangeCustomToken(
	channelId: string,
	descriptor: FirebaseRtdbChannelDescriptor,
	signal: AbortSignal,
): Promise<FirebaseSession> {
	if (descriptor.expires_at <= unixSeconds()) {
		throw new Error(
			`Firebase custom token for channel '${channelId}' expired at ${descriptor.expires_at}; use the fallback transport.`,
		);
	}
	const response = await fetch(
		`${SIGN_IN_URL}?key=${encodeURIComponent(descriptor.api_key)}`,
		{
			method: "POST",
			headers: { "content-type": "application/json" },
			body: JSON.stringify({
				token: descriptor.custom_token,
				returnSecureToken: true,
			}),
			signal,
		},
	);
	if (!response.ok) {
		const excerpt = await readBodyExcerpt(response);
		throw new Error(
			`Firebase sign-in for channel '${channelId}' failed (${response.status}): ${excerpt || response.statusText}`,
		);
	}
	const body = (await response.json()) as {
		idToken?: unknown;
		expiresIn?: unknown;
	};
	if (typeof body.idToken !== "string" || !body.idToken) {
		throw new Error(
			`Firebase sign-in for channel '${channelId}' returned no idToken.`,
		);
	}
	const expiresInSeconds = Number(body.expiresIn);
	const session: FirebaseSession = {
		customToken: descriptor.custom_token,
		idToken: body.idToken,
		expiresAtMs:
			Date.now() +
			(Number.isFinite(expiresInSeconds) && expiresInSeconds > 0
				? expiresInSeconds * 1000
				: 3600_000),
	};
	sessions.set(channelId, session);
	return session;
}

async function idTokenFor(
	channelId: string,
	descriptor: FirebaseRtdbChannelDescriptor,
	signal: AbortSignal,
): Promise<string> {
	const cached = sessions.get(channelId);
	if (
		cached &&
		cached.customToken === descriptor.custom_token &&
		cached.expiresAtMs - ID_TOKEN_REFRESH_MARGIN_MS > Date.now()
	) {
		return cached.idToken;
	}
	let exchange = exchanges.get(channelId);
	if (!exchange) {
		exchange = exchangeCustomToken(channelId, descriptor, signal).finally(() =>
			exchanges.delete(channelId),
		);
		exchanges.set(channelId, exchange);
	}
	return (await exchange).idToken;
}

function targetFor(
	descriptor: FirebaseRtdbChannelDescriptor,
	push: IChannelPush,
): { method: "PUT" | "POST"; path: string } {
	const kind = push.kind ?? "reply";
	if (kind === "reply") {
		if (!push.request_id) {
			throw new Error(
				`Firebase reply for channel '${push.channel_id}' has no request_id.`,
			);
		}
		return {
			method: "PUT",
			path: `${normalizePath(descriptor.inbox_path)}/${encodeURIComponent(push.request_id)}`,
		};
	}
	return { method: "POST", path: normalizePath(descriptor.inbound_path) };
}

async function write(
	descriptor: FirebaseRtdbChannelDescriptor,
	push: IChannelPush,
	idToken: string,
	signal: AbortSignal,
): Promise<Response> {
	const { method, path } = targetFor(descriptor, push);
	const url = `${databaseOrigin(descriptor.database_url)}${path}.json?auth=${encodeURIComponent(idToken)}`;
	return fetch(url, {
		method,
		headers: { "content-type": "application/json" },
		body: JSON.stringify({ payload: JSON.stringify(push) }),
		signal,
	});
}

export async function pushFirebaseRtdb(
	descriptor: FirebaseRtdbChannelDescriptor,
	push: IChannelPush,
	options: ChannelPushOptions = {},
): Promise<void> {
	const timeout = timeoutSignal(FIREBASE_PUSH_TIMEOUT_MS, options.signal);
	try {
		let response: Response;
		try {
			const idToken = await idTokenFor(
				push.channel_id,
				descriptor,
				timeout.signal,
			);
			response = await write(descriptor, push, idToken, timeout.signal);
			if (response.status === 401) {
				// The cached id token may have been revoked or lost its validity early; re-exchange once.
				sessions.delete(push.channel_id);
				const fresh = await idTokenFor(
					push.channel_id,
					descriptor,
					timeout.signal,
				);
				response = await write(descriptor, push, fresh, timeout.signal);
			}
		} catch (error) {
			if (timeout.timedOut()) {
				throw new Error(
					`Firebase push for channel '${push.channel_id}' timed out after ${FIREBASE_PUSH_TIMEOUT_MS} ms.`,
				);
			}
			if (timeout.signal.aborted) {
				throw new Error(
					`Firebase push for channel '${push.channel_id}' was aborted.`,
				);
			}
			throw error instanceof Error
				? error
				: new Error(
						`Firebase push for channel '${push.channel_id}' failed: ${errorMessage(error)}`,
					);
		}
		if (!response.ok) {
			const excerpt = await readBodyExcerpt(response);
			const denied =
				response.status === 401 || response.status === 403
					? " (rules denied)"
					: "";
			throw new Error(
				`Firebase push for channel '${push.channel_id}' failed (${response.status})${denied}: ${excerpt || response.statusText}`,
			);
		}
	} finally {
		timeout.dispose();
	}
}

export function resetFirebaseSessions(): void {
	sessions.clear();
	exchanges.clear();
}
