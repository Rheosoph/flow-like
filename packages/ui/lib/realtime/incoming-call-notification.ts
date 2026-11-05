import type {
	IConversation,
	IInboxEntry,
} from "../../state/backend-state/inbox-state";
import type { INotification } from "../../state/backend-state/types";

export const INCOMING_CALL_MAX_AGE_MS = 90_000;

export interface IncomingCallNotice {
	notification: INotification;
	conversationId: string;
	callId: string;
}

/** Call links carry an identity; ordinary chat notifications never trigger the prompt. */
export function incomingCallNotice(
	notification: INotification,
	userId: string,
	now = Date.now(),
): IncomingCallNotice | null {
	const age = now - Date.parse(notification.created_at);
	if (
		notification.user_id !== userId ||
		notification.read ||
		notification.notification_type !== "SYSTEM" ||
		!Number.isFinite(age) ||
		age < -5_000 ||
		age > INCOMING_CALL_MAX_AGE_MS ||
		!notification.link?.startsWith("/bots?")
	)
		return null;
	try {
		const url = new URL(notification.link, "https://flow-like.invalid");
		const conversationId = url.searchParams.get("conversation");
		const callId = url.searchParams.get("call");
		if (
			url.pathname !== "/bots" ||
			url.hash ||
			url.searchParams.getAll("conversation").length !== 1 ||
			url.searchParams.getAll("call").length !== 1 ||
			!conversationId ||
			!callId ||
			!/^[A-Za-z0-9_-]{1,160}$/.test(conversationId) ||
			!/^[A-Za-z0-9_-]{1,80}$/.test(callId)
		)
			return null;
		return { notification, conversationId, callId };
	} catch {
		return null;
	}
}

function callBody(entry: IInboxEntry): Record<string, unknown> | null {
	if (entry.kind !== "call" || entry.envelope.enc !== "none") return null;
	const body = "body" in entry.envelope ? entry.envelope.body : null;
	return body && typeof body === "object" && !Array.isArray(body)
		? (body as Record<string, unknown>)
		: null;
}

export function incomingCallEnded(
	entries: readonly IInboxEntry[],
	callId: string,
): boolean {
	return entries.some((entry) => {
		const body = callBody(entry);
		return body?.op === "ended" && body.call === callId;
	});
}

/** Verify a notification against the recipient's conversation before opening any peer room. */
export function incomingCallSender(
	notice: IncomingCallNotice,
	conversation: IConversation,
	entries: readonly IInboxEntry[],
	userId: string,
): string | null {
	if (
		conversation.id !== notice.conversationId ||
		conversation.local ||
		conversation.muted ||
		conversation.encryption !== "plain" ||
		!conversation.members.some(
			(member) => member.kind === "user" && member.principal_id === userId,
		) ||
		incomingCallEnded(entries, notice.callId)
	)
		return null;
	const start = entries.find(
		(entry) =>
			entry.conversation_id === notice.conversationId &&
			entry.message_id === notice.callId &&
			callBody(entry)?.op === "started",
	);
	if (
		!start ||
		start.sender_id === userId ||
		Date.now() - Date.parse(start.created_at) > INCOMING_CALL_MAX_AGE_MS ||
		!Number.isFinite(Date.parse(start.created_at)) ||
		!conversation.members.some(
			(member) =>
				member.kind === "user" && member.principal_id === start.sender_id,
		)
	)
		return null;
	return start.sender_id;
}

const RESOLVED_EVENT = "flow-like:incoming-call-resolved";
const resolvedInMemory = new Map<string, Record<string, number>>();

export function incomingCallResolutionKey(
	scope: string,
	userId: string,
): string {
	return `flow-like:incoming-calls:${JSON.stringify([scope, userId])}`;
}

function resolvedCalls(key: string): Record<string, number> {
	const result = { ...resolvedInMemory.get(key) };
	try {
		const stored: unknown = JSON.parse(localStorage.getItem(key) ?? "{}");
		if (stored && typeof stored === "object" && !Array.isArray(stored)) {
			for (const [id, expiry] of Object.entries(stored)) {
				if (typeof expiry === "number" && expiry > Date.now())
					result[id] = expiry;
			}
		}
	} catch {}
	for (const [id, expiry] of Object.entries(result)) {
		if (expiry <= Date.now()) delete result[id];
	}
	return result;
}

export function isIncomingCallResolved(key: string, callId: string): boolean {
	return resolvedCalls(key)[callId] !== undefined;
}

export function resolveIncomingCall(key: string, callId: string): void {
	const resolved = {
		...resolvedCalls(key),
		[callId]: Date.now() + INCOMING_CALL_MAX_AGE_MS,
	};
	resolvedInMemory.set(key, resolved);
	try {
		localStorage.setItem(key, JSON.stringify(resolved));
	} catch {}
	window.dispatchEvent(new CustomEvent(RESOLVED_EVENT, { detail: key }));
}

/** Storage events synchronize tabs; the custom event covers providers in the same window. */
export function subscribeIncomingCallResolutions(
	key: string,
	onChange: () => void,
): () => void {
	const storage = (event: StorageEvent) => {
		if (event.key === key) onChange();
	};
	const local = (event: Event) => {
		if ((event as CustomEvent).detail === key) onChange();
	};
	window.addEventListener("storage", storage);
	window.addEventListener(RESOLVED_EVENT, local);
	return () => {
		window.removeEventListener("storage", storage);
		window.removeEventListener(RESOLVED_EVENT, local);
	};
}
