export interface IncomingCallJoinRequest {
	scope: string;
	userId: string;
	conversationId: string;
	callId: string;
}

interface PendingJoin extends IncomingCallJoinRequest {
	expiresAt: number;
	context: AudioContext | null;
}

const JOIN_TIMEOUT_MS = 30_000;
const listeners = new Set<() => void>();
const scopes = new WeakMap<object, { scope: string }>();
let pending: PendingJoin | undefined;
let expiry: ReturnType<typeof setTimeout> | undefined;

function emit() {
	for (const listener of listeners) listener();
}

export function subscribeIncomingCallJoin(listener: () => void) {
	listeners.add(listener);
	return () => {
		listeners.delete(listener);
	};
}

export function getIncomingCallJoin() {
	return pending;
}

/** The account shell owns the scope; conversation hooks must use that same backend. */
export function registerIncomingCallScope(backend: object, scope: string) {
	const previous = scopes.get(backend);
	if (previous && previous.scope !== scope)
		clearIncomingCallJoin(previous.scope);
	const registration = { scope };
	scopes.set(backend, registration);
	emit();
	return () => {
		if (scopes.get(backend) !== registration) return;
		scopes.delete(backend);
		clearIncomingCallJoin(scope);
	};
}

export function getIncomingCallScope(backend: object) {
	return scopes.get(backend)?.scope;
}

export function clearIncomingCallJoin(scope?: string) {
	if (!pending || (scope !== undefined && pending.scope !== scope)) return;
	const context = pending.context;
	pending = undefined;
	clearTimeout(expiry);
	expiry = undefined;
	void context?.close().catch(() => {});
	emit();
}

/** Only a Join click creates this intent. Following a notification link never answers a call. */
export function requestIncomingCallJoin(
	request: IncomingCallJoinRequest,
): void {
	clearIncomingCallJoin();
	let context: AudioContext | null = null;
	try {
		// Resume during the click, before navigation loses browser user activation.
		context = new AudioContext();
		void context.resume().catch(() => {});
	} catch {
		// The call can still use the microphone without audio analysis.
	}
	pending = { ...request, context, expiresAt: Date.now() + JOIN_TIMEOUT_MS };
	expiry = setTimeout(() => clearIncomingCallJoin(), JOIN_TIMEOUT_MS);
	emit();
}

/** Transfers the click's audio context once, after the destination sees the same live call. */
export function takeIncomingCallJoin(
	scope: string,
	userId: string,
	conversationId: string,
	callId: string,
): PendingJoin | undefined {
	if (pending && pending.expiresAt <= Date.now()) clearIncomingCallJoin();
	if (
		!pending ||
		pending.scope !== scope ||
		pending.userId !== userId ||
		pending.conversationId !== conversationId ||
		pending.callId !== callId
	)
		return;
	const request = pending;
	pending = undefined;
	clearTimeout(expiry);
	expiry = undefined;
	emit();
	return request;
}
