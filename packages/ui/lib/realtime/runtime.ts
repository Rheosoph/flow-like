import { compareInboxCursors } from "../inbox-cursor";
import { RealtimeProviderSession, backoff, jitter } from "./provider-session";
import type { RealtimeProviderOpener } from "./providers";
import {
	MAX_REALTIME_CONVERSATIONS,
	MAX_REALTIME_CONVERSATION_ENTRIES,
	MAX_REALTIME_EVENTS,
	MAX_REALTIME_RESPONSE_BYTES,
	MAX_REALTIME_RUNS,
	REALTIME_ERROR_UNAVAILABLE,
	type RealtimeActivity,
	type RealtimeCommand,
	type RealtimeConnectionSession,
	type RealtimeConnectionState,
	type RealtimeConversationSubscription,
	type RealtimeNotificationSnapshot,
	type RealtimePollRequest,
	type RealtimePollResponse,
	type RealtimePort,
	type RealtimeRunSubscription,
	type RealtimeSubscription,
	isTerminalRunStatus,
} from "./types";

const BLOCKED_RETRY_MS = 30_000;
const RUN_INTERVAL_MS = 2_000;
const IDLE_INTERVAL_MS = 30_000;
const CHASE_MS = 750;
/** An API that does not hold polls answers well within this; one that holds them, never. */
const HELD_MS = 1_000;
const UNAVAILABLE_RETRY_MS = 2_000;
const UNAVAILABLE_RETRY_MAX_MS = 30_000;

/** A tab's replay position; `has_more` is what the last batch it accepted said about the run. */
interface PortRun extends RealtimeRunSubscription {
	has_more?: boolean;
}

/** The newest entry cursor a tab accepted; unset while it holds none. */
type PortConversation = RealtimeConversationSubscription;

interface PortState {
	id: string;
	port: RealtimePort;
	scope: Scope;
	activity: RealtimeActivity;
	seen: number;
	notifications: boolean;
	backgroundNotifications: boolean;
	transport: "configured" | "http";
	notificationRevision?: string;
	subscriptionVersion?: number;
	runs: Map<string, PortRun>;
	conversations: Map<string, PortConversation>;
	pending?: { id: string; snapshot: RealtimePollResponse; sent: number };
	/** A poll went out without this tab's subscriptions while its last batch was unacknowledged. */
	skipped?: boolean;
}

interface Scope {
	key: string;
	session: RealtimeConnectionSession;
	ports: Set<PortState>;
	state: RealtimeConnectionState;
	notification?: RealtimeNotificationSnapshot;
	epoch: number;
	failures: number;
	rotation: number;
	hiddenSince?: number;
	controller?: AbortController;
	/** The in-flight long-poll left out a port whose previous batch was still unacknowledged. */
	deferred?: boolean;
	timer?: ReturnType<typeof setTimeout>;
	blocked?: "unauthorized" | "unsupported";
	/** Unset while blocked: only new credentials can retry a signed-out account. */
	blockedUntil?: number;
	blockedFailures: number;
	unavailable: Map<string, { failures: number; retryAt: number }>;
	provider?: RealtimeProviderSession;
	wakeTimer?: ReturnType<typeof setTimeout>;
	wakePending?: boolean;
}

interface Authorization {
	port: PortState;
	resolve: (result: {
		headers?: Record<string, string>;
		failed?: boolean;
	}) => void;
	timer: ReturnType<typeof setTimeout>;
}

export type RealtimeFetch = (
	input: RequestInfo | URL,
	init?: RequestInit,
) => Promise<Response>;

export interface RealtimeRuntimeOptions {
	fetch?: RealtimeFetch;
	openProvider?: RealtimeProviderOpener;
	pollTimeoutMs?: number;
	requestTimeoutMs?: number;
	retryMinMs?: number;
	retryMaxMs?: number;
	hiddenGraceMs?: number;
	portLeaseMs?: number;
	sweepMs?: number;
	authorizationTimeoutMs?: number;
	providerRecoveryMs?: number;
	providerRunRecoveryMs?: number;
}

const emptyResponse = (): RealtimePollResponse => ({
	protocol_version: 1,
	runs: [],
	errors: [],
});

function conversationKey(item: {
	conversation_id: string;
	as?: string;
}): string {
	return JSON.stringify([
		"conversation",
		item.conversation_id,
		item.as ?? null,
	]);
}

function newerCursor(a: string | undefined, b: string | undefined) {
	if (a === undefined || b === undefined) return a ?? b;
	return compareInboxCursors(a, b) >= 0 ? a : b;
}

/** One connection per account and API; a changed hub URL is a different connection. */
function scopeKey(session: RealtimeConnectionSession): string {
	return JSON.stringify([session.scope, session.url]);
}

/** Capabilities of one run replay independently; so do their failures. */
function capabilityKey(run: RealtimeRunSubscription): string {
	return JSON.stringify([run.run_id, run.poll_token ?? null]);
}

/** The API answers a held poll early only with one of these. */
function hasPollChanges(response: RealtimePollResponse): boolean {
	return (
		response.notifications !== undefined ||
		response.runs.length > 0 ||
		response.errors.length > 0 ||
		(response.conversations?.length ?? 0) > 0 ||
		(response.conversation_errors?.length ?? 0) > 0
	);
}

function validCursor(value: unknown): value is number {
	return (
		Number.isInteger(value) &&
		Number(value) >= -1 &&
		Number(value) < 2_147_483_647
	);
}

function validText(value: unknown, maxBytes: number): value is string {
	return (
		typeof value === "string" &&
		value.length > 0 &&
		new TextEncoder().encode(value).byteLength <= maxBytes &&
		!/\p{Cc}/u.test(value)
	);
}

function validRevision(value: unknown): value is string {
	return validText(value, 128);
}

export function validRealtimeSubscription(
	value: RealtimeSubscription,
): boolean {
	if (
		typeof value.notifications !== "boolean" ||
		(value.backgroundNotifications !== undefined &&
			typeof value.backgroundNotifications !== "boolean") ||
		!Array.isArray(value.runs) ||
		value.runs.length > MAX_REALTIME_RUNS
	)
		return false;
	const seen = new Set<string>();
	for (const run of value.runs) {
		if (
			!validText(run.run_id, 256) ||
			seen.has(run.run_id) ||
			!validCursor(run.after_sequence ?? -1) ||
			(run.poll_token !== undefined && !validText(run.poll_token, 8192)) ||
			(run.revision !== undefined && !validRevision(run.revision))
		)
			return false;
		seen.add(run.run_id);
	}
	const conversations = value.conversations ?? [];
	if (
		!Array.isArray(conversations) ||
		conversations.length > MAX_REALTIME_CONVERSATIONS
	)
		return false;
	const watched = new Set<string>();
	for (const item of conversations) {
		const id = conversationKey(item);
		if (
			!validText(item.conversation_id, 256) ||
			(item.as !== undefined && !validText(item.as, 256)) ||
			(item.after !== undefined && !validText(item.after, 512)) ||
			watched.has(id)
		)
			return false;
		watched.add(id);
	}
	return true;
}

class RealtimeProtocolError extends Error {}

function record(value: unknown): value is Record<string, unknown> {
	return value !== null && typeof value === "object" && !Array.isArray(value);
}

async function readResponse(response: Response): Promise<unknown> {
	const limit = MAX_REALTIME_RESPONSE_BYTES;
	if (Number(response.headers.get("Content-Length")) > limit) {
		void response.body?.cancel();
		throw new RealtimeProtocolError("Realtime response exceeds the size limit");
	}
	if (!response.body)
		throw new RealtimeProtocolError("Realtime response is empty");
	const reader = response.body.getReader();
	const decoder = new TextDecoder();
	let bytes = 0;
	let body = "";
	try {
		for (;;) {
			const chunk = await reader.read();
			if (chunk.done) break;
			bytes += chunk.value.byteLength;
			if (bytes > limit) {
				void reader.cancel().catch(() => {});
				throw new RealtimeProtocolError(
					"Realtime response exceeds the size limit",
				);
			}
			body += decoder.decode(chunk.value, { stream: true });
		}
		try {
			return JSON.parse(body + decoder.decode());
		} catch {
			throw new RealtimeProtocolError("Realtime response is not JSON");
		}
	} finally {
		reader.releaseLock();
	}
}

function parseConversationSections(value: Record<string, unknown>): void {
	const { conversations, conversation_errors: errors } = value;
	if (conversations !== undefined) {
		if (
			!Array.isArray(conversations) ||
			conversations.length > MAX_REALTIME_CONVERSATIONS
		)
			throw new RealtimeProtocolError("Invalid realtime conversation updates");
		const seen = new Set<string>();
		for (const item of conversations) {
			if (
				!record(item) ||
				typeof item.conversation_id !== "string" ||
				(item.as !== undefined && typeof item.as !== "string") ||
				typeof item.has_more !== "boolean" ||
				!Array.isArray(item.entries) ||
				item.entries.length > MAX_REALTIME_CONVERSATION_ENTRIES ||
				seen.has(conversationKey(item as { conversation_id: string }))
			)
				throw new RealtimeProtocolError("Invalid realtime conversation update");
			seen.add(conversationKey(item as { conversation_id: string }));
			let previous: string | undefined;
			for (const entry of item.entries) {
				if (
					!record(entry) ||
					typeof entry.cursor !== "string" ||
					typeof entry.message_id !== "string" ||
					(previous !== undefined &&
						compareInboxCursors(previous, entry.cursor) >= 0)
				)
					throw new RealtimeProtocolError(
						"Invalid realtime conversation entry",
					);
				previous = entry.cursor;
			}
		}
	}
	if (errors !== undefined) {
		if (!Array.isArray(errors) || errors.length > MAX_REALTIME_CONVERSATIONS)
			throw new RealtimeProtocolError("Invalid realtime conversation errors");
		for (const error of errors)
			if (
				!record(error) ||
				typeof error.conversation_id !== "string" ||
				(error.as !== undefined && typeof error.as !== "string") ||
				typeof error.code !== "string"
			)
				throw new RealtimeProtocolError("Invalid realtime conversation error");
	}
}

/** Reject malformed batches before they can advance any subscription cursor. */
export function parseRealtimeResponse(value: unknown): RealtimePollResponse {
	if (
		!record(value) ||
		value.protocol_version !== 1 ||
		!Array.isArray(value.runs) ||
		!Array.isArray(value.errors)
	)
		throw new RealtimeProtocolError("Unsupported realtime response");
	if (
		value.runs.length > MAX_REALTIME_RUNS ||
		value.errors.length > MAX_REALTIME_RUNS
	)
		throw new RealtimeProtocolError(
			"Realtime response exceeds subscription limits",
		);
	if (value.notifications !== undefined) {
		const notification = value.notifications;
		if (
			!record(notification) ||
			!validRevision(notification.revision) ||
			!record(notification.overview)
		)
			throw new RealtimeProtocolError("Invalid notification snapshot");
		for (const key of ["invites_count", "notifications_count", "unread_count"])
			if (
				!Number.isSafeInteger(notification.overview[key]) ||
				Number(notification.overview[key]) < 0
			)
				throw new RealtimeProtocolError("Invalid notification overview");
	}
	let count = 0;
	const seen = new Set<string>();
	for (const run of value.runs) {
		if (
			!record(run) ||
			typeof run.run_id !== "string" ||
			!validRevision(run.revision) ||
			typeof run.status !== "string" ||
			!Number.isInteger(run.progress) ||
			!validCursor(run.after_sequence) ||
			typeof run.has_more !== "boolean" ||
			!Array.isArray(run.events) ||
			seen.has(run.run_id)
		)
			throw new RealtimeProtocolError("Invalid realtime run snapshot");
		seen.add(run.run_id);
		let previous = -1;
		for (const event of run.events) {
			if (
				!record(event) ||
				!validCursor(event.sequence) ||
				event.sequence <= previous ||
				event.sequence > run.after_sequence ||
				(event.event_id !== undefined && !validText(event.event_id, 512)) ||
				typeof event.event_type !== "string" ||
				typeof event.created_at !== "string"
			)
				throw new RealtimeProtocolError("Invalid realtime event sequence");
			previous = event.sequence;
			if (++count > MAX_REALTIME_EVENTS)
				throw new RealtimeProtocolError("Realtime event batch is too large");
		}
	}
	for (const error of value.errors)
		if (
			!record(error) ||
			typeof error.run_id !== "string" ||
			typeof error.code !== "string"
		)
			throw new RealtimeProtocolError("Invalid realtime subscription error");
	parseConversationSections(value);
	return value as unknown as RealtimePollResponse;
}

/** One account connection serves independently acknowledged tab subscriptions. */
export class RealtimeRuntime {
	private readonly ports = new Map<string, PortState>();
	private readonly scopes = new Map<string, Scope>();
	private readonly authorizations = new Map<string, Authorization>();
	private readonly fetcher: RealtimeFetch;
	private readonly options: Required<
		Omit<RealtimeRuntimeOptions, "fetch" | "openProvider">
	>;
	private readonly openProvider?: RealtimeProviderOpener;
	private sweep?: ReturnType<typeof setInterval>;

	constructor(options: RealtimeRuntimeOptions = {}) {
		// WebIDL fetch rejects any receiver except the global ("Illegal invocation").
		this.fetcher = (input, init) =>
			(options.fetch ?? globalThis.fetch)(input, init);
		this.openProvider = options.openProvider;
		this.options = {
			pollTimeoutMs: 20_000,
			requestTimeoutMs: 30_000,
			retryMinMs: 1_000,
			retryMaxMs: 30_000,
			hiddenGraceMs: 60_000,
			portLeaseMs: 90_000,
			sweepMs: 15_000,
			authorizationTimeoutMs: 5_000,
			providerRecoveryMs: 30_000,
			providerRunRecoveryMs: 5_000,
			...options,
		};
	}

	receive(port: RealtimePort, command: RealtimeCommand): void {
		if (command.type === "connect") {
			if (this.ports.has(command.id)) return;
			const key = scopeKey(command.session);
			let scope = this.scopes.get(key);
			if (!scope) {
				scope = {
					key,
					session: command.session,
					ports: new Set(),
					state: "paused",
					epoch: 0,
					failures: 0,
					rotation: 0,
					blockedFailures: 0,
					unavailable: new Map(),
				};
				this.scopes.set(key, scope);
			}
			const connected: PortState = {
				id: command.id,
				port,
				scope,
				activity: command.activity,
				seen: Date.now(),
				notifications: false,
				backgroundNotifications: false,
				transport: command.session.transport ?? "configured",
				runs: new Map(),
				conversations: new Map(),
			};
			this.ports.set(command.id, connected);
			scope.ports.add(connected);
			if (
				scope.blocked === "unauthorized" &&
				JSON.stringify(scope.session.headers) !==
					JSON.stringify(command.session.headers)
			) {
				scope.session = command.session;
				this.unblock(scope);
			}
			this.send(connected, { type: "ready" });
			this.send(connected, { type: "state", state: scope.state });
			this.sweep ??= setInterval(
				() => this.expirePorts(),
				this.options.sweepMs,
			);
			return;
		}
		const connected = this.ports.get(command.id);
		if (!connected || connected.port !== port) {
			if (command.type === "heartbeat") port.postMessage({ type: "expired" });
			return;
		}
		connected.seen = Date.now();
		const scope = connected.scope;
		switch (command.type) {
			case "disconnect":
				this.remove(connected);
				return;
			case "session": {
				if (scopeKey(command.session) !== scope.key) {
					this.remove(connected);
					this.receive(port, {
						type: "connect",
						id: command.id,
						session: command.session,
						activity: connected.activity,
					});
					return;
				}
				const preference = command.session.transport ?? "configured";
				const preferenceChanged = connected.transport !== preference;
				connected.transport = preference;
				const changed =
					JSON.stringify(command.session.headers) !==
					JSON.stringify(scope.session.headers);
				scope.session = command.session;
				if (changed && scope.blocked === "unauthorized") {
					this.unblock(scope);
					this.restart(scope);
				} else if (preferenceChanged) this.restart(scope);
				else if (changed) this.syncProvider(scope);
				return;
			}
			case "heartbeat": {
				const changed =
					connected.activity.online !== command.activity.online ||
					connected.activity.visible !== command.activity.visible;
				connected.activity = command.activity;
				if (
					connected.pending &&
					Date.now() - connected.pending.sent >= 10_000
				) {
					connected.pending.sent = Date.now();
					this.send(connected, {
						type: "snapshot",
						delivery_id: connected.pending.id,
						snapshot: connected.pending.snapshot,
						subscription_version: connected.subscriptionVersion,
					});
				}
				if (changed) this.restart(scope);
				return;
			}
			case "subscribe": {
				const desired = command.subscription;
				if (!validRealtimeSubscription(desired)) return;
				const runs = new Map<string, PortRun>();
				for (const {
					run_id,
					poll_token,
					after_sequence = -1,
					revision,
				} of desired.runs) {
					const old = connected.runs.get(run_id);
					runs.set(
						run_id,
						old &&
							old.poll_token === poll_token &&
							(old.after_sequence ?? -1) >= after_sequence
							? old
							: { run_id, poll_token, after_sequence, revision },
					);
				}
				connected.runs = runs;
				const conversations = new Map<string, PortConversation>();
				for (const item of desired.conversations ?? []) {
					const id = conversationKey(item);
					const old = connected.conversations.get(id);
					conversations.set(
						id,
						old?.after !== undefined &&
							(item.after === undefined ||
								compareInboxCursors(old.after, item.after) >= 0)
							? old
							: { ...item },
					);
				}
				connected.conversations = conversations;
				connected.subscriptionVersion = command.subscription_version;
				connected.notifications = desired.notifications;
				connected.backgroundNotifications =
					desired.notifications && desired.backgroundNotifications === true;
				connected.pending = undefined;
				connected.skipped = false;
				this.offerNotification(connected);
				this.restart(scope);
				return;
			}
			case "accepted": {
				const pending = connected.pending;
				if (!pending || pending.id !== command.delivery_id) return;
				connected.pending = undefined;
				if (pending.snapshot.notifications)
					connected.notificationRevision =
						pending.snapshot.notifications.revision;
				for (const run of pending.snapshot.runs) {
					const current = connected.runs.get(run.run_id);
					if (current)
						connected.runs.set(run.run_id, {
							...current,
							after_sequence: Math.max(
								current.after_sequence ?? -1,
								run.after_sequence,
							),
							revision: run.revision,
							has_more: run.has_more,
						});
				}
				for (const error of pending.snapshot.errors)
					if (error.code !== REALTIME_ERROR_UNAVAILABLE)
						connected.runs.delete(error.run_id);
				for (const update of pending.snapshot.conversations ?? []) {
					const id = conversationKey(update);
					const current = connected.conversations.get(id);
					const newest = update.entries.at(-1)?.cursor;
					if (current && newest)
						connected.conversations.set(id, {
							...current,
							after: newerCursor(current.after, newest),
						});
				}
				for (const error of pending.snapshot.conversation_errors ?? [])
					if (error.code !== REALTIME_ERROR_UNAVAILABLE)
						connected.conversations.delete(conversationKey(error));
				if (this.offerNotification(connected)) return;
				const skipped = connected.skipped;
				connected.skipped = false;
				if (skipped) this.pollNow(scope);
				else this.schedule(scope, 0);
				return;
			}
			case "authorize": {
				const pending = this.authorizations.get(command.request_id);
				if (pending?.port !== connected) return;
				this.authorizations.delete(command.request_id);
				clearTimeout(pending.timer);
				pending.resolve({ headers: command.headers, failed: command.failed });
			}
		}
	}

	dispose(): void {
		for (const port of [...this.ports.values()]) {
			this.send(port, { type: "state", state: "closed" });
			this.remove(port);
		}
	}

	private send(
		port: PortState,
		message: Parameters<RealtimePort["postMessage"]>[0],
	): void {
		try {
			port.port.postMessage(message);
		} catch {
			this.remove(port);
		}
	}

	private remove(port: PortState): void {
		if (!this.ports.delete(port.id)) return;
		port.scope.ports.delete(port);
		port.runs.clear();
		port.conversations.clear();
		port.pending = undefined;
		for (const [id, pending] of this.authorizations) {
			if (pending.port !== port) continue;
			clearTimeout(pending.timer);
			this.authorizations.delete(id);
			pending.resolve({});
		}
		if (port.scope.ports.size === 0) {
			port.scope.epoch++;
			port.scope.controller?.abort();
			clearTimeout(port.scope.timer);
			clearTimeout(port.scope.wakeTimer);
			port.scope.provider?.close();
			this.scopes.delete(port.scope.key);
		} else this.restart(port.scope);
		if (this.ports.size === 0) {
			clearInterval(this.sweep);
			this.sweep = undefined;
		}
	}

	private expirePorts(): void {
		for (const port of [...this.ports.values()]) {
			if (Date.now() - port.seen <= this.options.portLeaseMs) continue;
			this.remove(port);
			try {
				port.port.postMessage({ type: "expired" });
			} catch {
				/* The tab already closed. */
			}
		}
		for (const scope of this.scopes.values()) this.syncProvider(scope);
	}

	private state(scope: Scope, state: RealtimeConnectionState): void {
		if (scope.state === state) return;
		scope.state = state;
		for (const port of scope.ports) this.send(port, { type: "state", state });
		if (state === "unsupported" || state === "unauthorized")
			scope.provider?.close();
	}

	/** A refused account keeps reporting its state and probes again on a doubling schedule. */
	private block(scope: Scope, state: "unauthorized" | "unsupported"): void {
		scope.blocked = state;
		scope.blockedUntil =
			Date.now() + backoff(BLOCKED_RETRY_MS, scope.blockedFailures++);
		this.state(scope, state);
	}

	private unblock(scope: Scope): void {
		scope.blocked = undefined;
		scope.blockedUntil = undefined;
	}

	private restart(scope: Scope): void {
		scope.epoch++;
		scope.controller?.abort();
		scope.controller = undefined;
		clearTimeout(scope.timer);
		scope.timer = undefined;
		this.schedule(scope, 0);
	}

	/** Milliseconds until a blocked account may probe again; undefined while it needs new credentials. */
	private blockedFor(scope: Scope): number | undefined {
		if (!scope.blocked) return 0;
		return scope.blockedUntil === undefined
			? undefined
			: scope.blockedUntil - Date.now();
	}

	/** A tab left out of a poll replays at once instead of waiting out a recovery or interval timer. */
	private pollNow(scope: Scope): void {
		if (scope.controller) {
			if (scope.deferred) this.restart(scope);
			else scope.wakePending = true;
			return;
		}
		if (!scope.failures) {
			clearTimeout(scope.timer);
			scope.timer = undefined;
		}
		this.schedule(scope, 0);
	}

	private schedule(scope: Scope, delay: number): void {
		if (!scope.ports.size || scope.controller || scope.timer) return;
		const blocked = this.blockedFor(scope);
		if (blocked === undefined) return;
		scope.timer = setTimeout(
			() => {
				scope.timer = undefined;
				void this.poll(scope);
			},
			Math.max(0, delay, blocked),
		);
	}

	private activePorts(scope: Scope): PortState[] {
		const online = [...scope.ports].filter((port) => port.activity.online);
		const visible = online.some((port) => port.activity.visible);
		if (visible) scope.hiddenSince = undefined;
		else scope.hiddenSince ??= Date.now();
		const activeRuns = online.some((port) => port.runs.size > 0);
		const backgroundNotifications = online.some(
			(port) => port.backgroundNotifications,
		);
		if (
			!online.length ||
			(!activeRuns &&
				!backgroundNotifications &&
				!visible &&
				Date.now() - (scope.hiddenSince ?? Date.now()) >=
					this.options.hiddenGraceMs)
		)
			return [];
		return online;
	}

	/** Every active account negotiates a grant; an HTTP grant still says how long a poll may wait. */
	private syncProvider(scope: Scope): void {
		const active = scope.blocked
			? []
			: this.activePorts(scope).filter(
					(port) =>
						port.notifications || port.runs.size || port.conversations.size,
				);
		if (!active.length) {
			if (scope.blocked) scope.provider?.close();
			else scope.provider?.pause();
			return;
		}
		scope.provider ??= new RealtimeProviderSession({
			fetch: this.fetcher,
			open: this.openProvider,
			changed: () => {
				if (scope.ports.size && !scope.blocked) this.restart(scope);
			},
			wakeup: () => {
				if (scope.wakeTimer || !scope.ports.size || scope.blocked) return;
				scope.wakeTimer = setTimeout(() => {
					scope.wakeTimer = undefined;
					if (scope.controller) scope.wakePending = true;
					else {
						clearTimeout(scope.timer);
						scope.timer = undefined;
						this.schedule(scope, 0);
					}
				}, 100);
			},
		});
		scope.provider.sync(
			scope.session,
			active.some((port) => port.transport !== "http") ? "configured" : "http",
		);
	}

	private pollWaitMs(scope: Scope): number {
		return Math.min(
			this.options.pollTimeoutMs,
			scope.provider?.maxPollWaitMs ?? this.options.pollTimeoutMs,
		);
	}

	private request(scope: Scope): RealtimePollRequest | undefined {
		const online = this.activePorts(scope);
		scope.deferred = false;
		if (!online.length) return;
		const now = Date.now();
		const subscribed = new Set(
			[...scope.ports].flatMap((port) => [
				...[...port.runs.values()].map(capabilityKey),
				...port.conversations.keys(),
			]),
		);
		for (const id of scope.unavailable.keys())
			if (!subscribed.has(id)) scope.unavailable.delete(id);
		const runs = new Map<string, RealtimeRunSubscription>();
		const conversations = new Map<
			string,
			{ item: RealtimeConversationSubscription; withoutCursor: boolean }
		>();
		const unacknowledged: PortState[] = [];
		let notifications = false;
		for (const port of online) {
			if (port.pending) {
				unacknowledged.push(port);
				continue;
			}
			notifications ||= port.notifications;
			for (const [id, item] of port.conversations) {
				if ((scope.unavailable.get(id)?.retryAt ?? 0) > now) continue;
				const current = conversations.get(id);
				if (!current)
					conversations.set(id, {
						item: { ...item },
						withoutCursor: item.after === undefined,
					});
				else if (!current.withoutCursor) {
					if (item.after === undefined) {
						current.withoutCursor = true;
						current.item.after = undefined;
					} else if (
						current.item.after !== undefined &&
						compareInboxCursors(item.after, current.item.after) < 0
					)
						current.item.after = item.after;
				}
			}
			for (const run of port.runs.values()) {
				const id = capabilityKey(run);
				if ((scope.unavailable.get(id)?.retryAt ?? 0) > now) continue;
				// A revision from a page with more to come can already describe the finished run;
				// the API would then leave out the closing page this tab still waits for.
				const revision = run.has_more === false ? run.revision : undefined;
				const current = runs.get(id);
				if (!current)
					runs.set(id, {
						run_id: run.run_id,
						poll_token: run.poll_token,
						after_sequence: run.after_sequence ?? -1,
						revision,
					});
				else {
					current.after_sequence = Math.min(
						current.after_sequence ?? -1,
						run.after_sequence ?? -1,
					);
					if (current.revision !== revision) current.revision = undefined;
				}
			}
		}
		const deferred = unacknowledged.filter(
			(port) =>
				port.runs.size > 0 ||
				port.conversations.size > 0 ||
				(port.notifications && !notifications),
		);
		for (const port of deferred) port.skipped = true;
		const all = [...runs.values()];
		const offset = all.length ? scope.rotation % all.length : 0;
		const selected: RealtimeRunSubscription[] = [];
		const selectedRuns = new Set<string>();
		for (const run of [...all.slice(offset), ...all.slice(0, offset)]) {
			if (selectedRuns.has(run.run_id)) continue;
			selected.push(run);
			selectedRuns.add(run.run_id);
			if (selected.length === MAX_REALTIME_RUNS) break;
		}
		scope.rotation = offset + 1;
		const selectedConversations = [...conversations.values()]
			.slice(0, MAX_REALTIME_CONVERSATIONS)
			.map(({ item }) => item);
		if (!notifications && !selected.length && !selectedConversations.length)
			return;
		const timeout = scope.provider?.connected ? 0 : this.pollWaitMs(scope);
		scope.deferred = deferred.length > 0 && timeout > 0;
		return {
			...(notifications
				? { notifications: { revision: scope.notification?.revision } }
				: {}),
			runs: selected,
			...(selectedConversations.length
				? { conversations: selectedConversations }
				: {}),
			timeout_ms: timeout,
		};
	}

	/** The earliest retry of a run the API reported as temporarily unavailable. */
	private nextRunRetry(scope: Scope): number | undefined {
		const now = Date.now();
		const retries = [...scope.unavailable.values()]
			.map((entry) => entry.retryAt)
			.filter((retryAt) => retryAt > now);
		return retries.length ? Math.min(...retries) : undefined;
	}

	private trackUnavailable(
		scope: Scope,
		request: RealtimePollRequest,
		response: RealtimePollResponse,
	): void {
		const unavailable = new Set(
			response.errors
				.filter((error) => error.code === REALTIME_ERROR_UNAVAILABLE)
				.map((error) => error.run_id),
		);
		const unavailableConversations = new Set(
			(response.conversation_errors ?? [])
				.filter((error) => error.code === REALTIME_ERROR_UNAVAILABLE)
				.map(conversationKey),
		);
		const requested = [
			...request.runs.map((run) => ({
				id: capabilityKey(run),
				failed: unavailable.has(run.run_id),
			})),
			...(request.conversations ?? []).map((item) => ({
				id: conversationKey(item),
				failed: unavailableConversations.has(conversationKey(item)),
			})),
		];
		for (const { id, failed } of requested) {
			if (!failed) {
				scope.unavailable.delete(id);
				continue;
			}
			const failures = (scope.unavailable.get(id)?.failures ?? 0) + 1;
			scope.unavailable.set(id, {
				failures,
				retryAt:
					Date.now() +
					Math.min(
						UNAVAILABLE_RETRY_MAX_MS,
						UNAVAILABLE_RETRY_MS * 2 ** (failures - 1),
					),
			});
		}
	}

	/**
	 * Live providers wake replay; their dropped wakeups are covered by a chase poll while a run
	 * streams. Without one, the API's poll wait decides between long polls and an interval. A
	 * long poll that comes back early without changes was not held, so it paces like an interval.
	 */
	private nextDelay(
		scope: Scope,
		request: RealtimePollRequest,
		response: RealtimePollResponse,
		elapsedMs: number,
	): number {
		if (
			response.runs.some((run) => run.has_more) ||
			response.conversations?.some((item) => item.has_more)
		)
			return 50;
		const runs = [...scope.ports].some(
			(port) => port.activity.online && port.runs.size > 0,
		);
		if (scope.provider?.connected) {
			if (
				response.runs.some(
					(run) => run.events.length && !isTerminalRunStatus(run.status),
				)
			)
				return CHASE_MS;
			return runs
				? this.options.providerRunRecoveryMs
				: this.options.providerRecoveryMs;
		}
		const held = elapsedMs >= Math.min(request.timeout_ms / 2, HELD_MS);
		if (request.timeout_ms > 0 && (held || hasPollChanges(response))) return 50;
		return jitter(runs ? RUN_INTERVAL_MS : IDLE_INTERVAL_MS);
	}

	/** Hands a tab the account's newest known inbox snapshot when it holds an older one. */
	private offerNotification(port: PortState): boolean {
		const notification = port.scope.notification;
		if (
			port.pending ||
			!port.notifications ||
			!notification ||
			port.notificationRevision === notification.revision
		)
			return false;
		this.deliver(port, { ...emptyResponse(), notifications: notification });
		return true;
	}

	private deliver(port: PortState, snapshot: RealtimePollResponse): void {
		if (
			port.pending ||
			(!snapshot.notifications &&
				!snapshot.runs.length &&
				!snapshot.errors.length &&
				!snapshot.conversations?.length &&
				!snapshot.conversation_errors?.length)
		)
			return;
		const id = crypto.randomUUID();
		port.pending = { id, snapshot, sent: Date.now() };
		this.send(port, {
			type: "snapshot",
			delivery_id: id,
			snapshot,
			subscription_version: port.subscriptionVersion,
		});
	}

	private publish(
		scope: Scope,
		response: RealtimePollResponse,
		request: RealtimePollRequest,
	): void {
		const requested = new Map(request.runs.map((run) => [run.run_id, run]));
		const requestedConversations = new Set(
			(request.conversations ?? []).map(conversationKey),
		);
		if (request.notifications && response.notifications)
			scope.notification = response.notifications;
		for (const port of scope.ports) {
			if (port.pending || !port.activity.online) continue;
			const snapshot = emptyResponse();
			if (
				port.notifications &&
				scope.notification &&
				port.notificationRevision !== scope.notification.revision
			)
				snapshot.notifications = scope.notification;
			for (const run of response.runs) {
				const subscribed = port.runs.get(run.run_id);
				if (
					!requested.has(run.run_id) ||
					!subscribed ||
					requested.get(run.run_id)?.poll_token !== subscribed.poll_token
				)
					continue;
				const events = run.events.filter(
					(event) => event.sequence > (subscribed.after_sequence ?? -1),
				);
				if (
					events.length ||
					subscribed.revision !== run.revision ||
					(subscribed.has_more !== false && !run.has_more)
				)
					snapshot.runs.push({
						...run,
						events,
						after_sequence: Math.max(
							subscribed.after_sequence ?? -1,
							run.after_sequence,
						),
					});
			}
			snapshot.errors = response.errors.filter(
				(error) =>
					requested.has(error.run_id) &&
					port.runs.has(error.run_id) &&
					requested.get(error.run_id)?.poll_token ===
						port.runs.get(error.run_id)?.poll_token,
			);
			const conversations = (response.conversations ?? []).flatMap((update) => {
				const id = conversationKey(update);
				const subscribed = port.conversations.get(id);
				if (!subscribed || !requestedConversations.has(id)) return [];
				const entries = update.entries.filter(
					(entry) =>
						subscribed.after === undefined ||
						compareInboxCursors(entry.cursor, subscribed.after) > 0,
				);
				return entries.length ? [{ ...update, entries }] : [];
			});
			if (conversations.length) snapshot.conversations = conversations;
			const conversationErrors = (response.conversation_errors ?? []).filter(
				(error) =>
					requestedConversations.has(conversationKey(error)) &&
					port.conversations.has(conversationKey(error)),
			);
			if (conversationErrors.length)
				snapshot.conversation_errors = conversationErrors;
			this.deliver(port, snapshot);
		}
	}

	private async renew(
		scope: Scope,
		rejectedToken: string | undefined,
		epoch: number,
	): Promise<boolean> {
		let transientFailure = false;
		for (const port of [...scope.ports].filter(
			(candidate) => candidate.activity.online,
		)) {
			if (!scope.ports.has(port)) continue;
			const requestId = crypto.randomUUID();
			const result = await new Promise<{
				headers?: Record<string, string>;
				failed?: boolean;
			}>((resolve) => {
				const timer = setTimeout(() => {
					this.authorizations.delete(requestId);
					resolve({ failed: true });
				}, this.options.authorizationTimeoutMs);
				this.authorizations.set(requestId, { port, resolve, timer });
				this.send(port, {
					type: "unauthorized",
					request_id: requestId,
					rejectedToken,
				});
			});
			if (scope.epoch !== epoch || !scope.ports.size) return false;
			const authorization =
				result.headers && new Headers(result.headers).get("Authorization");
			if (
				result.headers &&
				authorization?.startsWith("Bearer ") &&
				authorization !== `Bearer ${rejectedToken}`
			) {
				scope.session = { ...scope.session, headers: result.headers };
				return true;
			}
			transientFailure ||= Boolean(result.failed);
		}
		if (transientFailure)
			throw new Error("Realtime session renewal is temporarily unavailable");
		scope.blocked = "unauthorized";
		scope.blockedUntil = undefined;
		this.state(scope, "unauthorized");
		return false;
	}

	private async poll(scope: Scope): Promise<void> {
		if (scope.controller || !scope.ports.size) return;
		const blocked = this.blockedFor(scope);
		if (blocked === undefined) return;
		if (blocked > 0) {
			this.schedule(scope, 0);
			return;
		}
		this.syncProvider(scope);
		const request = this.request(scope);
		if (!request) {
			const retry = this.nextRunRetry(scope);
			if (retry === undefined) this.state(scope, "paused");
			else this.schedule(scope, retry - Date.now());
			return;
		}
		const controller = new AbortController();
		scope.controller = controller;
		const epoch = scope.epoch;
		const headers = new Headers(scope.session.headers);
		headers.set("Content-Type", "application/json");
		const deadline = setTimeout(
			() => controller.abort(),
			this.options.requestTimeoutMs,
		);
		if (scope.state !== "connected" && !scope.blocked)
			this.state(scope, scope.failures ? "reconnecting" : "connecting");
		let delay = 50;
		const sentAt = Date.now();
		try {
			const response = await this.fetcher(scope.session.url, {
				method: "POST",
				headers,
				body: JSON.stringify(request),
				signal: controller.signal,
				cache: "no-store",
				credentials: "omit",
				redirect: "error",
			});
			if (scope.epoch !== epoch || !scope.ports.size) {
				void response.body?.cancel().catch(() => {});
				return;
			}
			if ([404, 405, 501].includes(response.status)) {
				await response.body?.cancel();
				this.block(scope, "unsupported");
				return;
			}
			if (response.status === 401) {
				await response.body?.cancel();
				await this.renew(
					scope,
					headers.get("Authorization")?.replace(/^Bearer /, ""),
					epoch,
				);
				return;
			}
			if (response.status === 403) {
				await response.body?.cancel();
				this.block(scope, "unauthorized");
				return;
			}
			if (!response.ok) {
				await response.body?.cancel();
				throw new Error(`Realtime poll failed (${response.status})`);
			}
			const parsed = parseRealtimeResponse(await readResponse(response));
			for (const run of parsed.runs) {
				const subscribed = request.runs.find(
					(item) => item.run_id === run.run_id,
				);
				const last =
					run.events.at(-1)?.sequence ?? subscribed?.after_sequence ?? -1;
				if (subscribed && run.after_sequence !== last)
					throw new RealtimeProtocolError(
						"Realtime response skipped an event cursor",
					);
			}
			if (
				scope.epoch !== epoch ||
				!scope.ports.size ||
				controller.signal.aborted
			)
				return;
			scope.failures = 0;
			scope.blockedFailures = 0;
			this.unblock(scope);
			this.state(scope, "connected");
			this.trackUnavailable(scope, request, parsed);
			this.publish(scope, parsed, request);
			delay = this.nextDelay(scope, request, parsed, Date.now() - sentAt);
		} catch (error) {
			if (scope.epoch !== epoch || !scope.ports.size) return;
			if (error instanceof RealtimeProtocolError) {
				this.block(scope, "unsupported");
				return;
			}
			scope.failures++;
			delay = Math.min(
				this.options.retryMaxMs,
				this.options.retryMinMs * 2 ** Math.min(scope.failures - 1, 8),
			);
			if (!scope.blocked) this.state(scope, "reconnecting");
		} finally {
			clearTimeout(deadline);
			if (scope.epoch === epoch) {
				scope.controller = undefined;
				scope.deferred = false;
				if (scope.wakePending) {
					scope.wakePending = false;
					delay = 0;
				}
				this.schedule(scope, delay);
			}
		}
	}
}
