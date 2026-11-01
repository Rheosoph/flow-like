import {
	type RealtimeProviderConnection,
	type RealtimeProviderOpener,
	RealtimeProviderProtocolError,
	boundedJson,
	openRealtimeProvider,
} from "./providers";
import {
	MAX_REALTIME_TIMEOUT_MS,
	type RealtimeConnectResponse,
	type RealtimeConnectionSession,
	type RealtimeTransport,
} from "./types";

type Fetch = (url: RequestInfo | URL, init?: RequestInit) => Promise<Response>;
export type RealtimeTransportPreference = NonNullable<
	RealtimeConnectionSession["transport"]
>;

const RECONNECT_MIN_MS = 2_000;
const BLOCKED_RETRY_MIN_MS = 30_000;
const RETRY_MAX_MS = 300_000;
const STABLE_CONNECTION_MS = 60_000;
const NEGOTIATION_TIMEOUT_MS = 15_000;

/** ±20 % keeps clients that failed together from retrying together. */
export function jitter(ms: number): number {
	return Math.round(ms * (0.8 + Math.random() * 0.4));
}

export function backoff(
	minMs: number,
	attempt: number,
	maxMs = RETRY_MAX_MS,
): number {
	return jitter(Math.min(maxMs, minMs * 2 ** Math.min(attempt, 20)));
}

export interface RealtimeGrant extends RealtimeConnectResponse {
	/** Local time in milliseconds after which the grant opens no connection. */
	deadline: number;
	/** Server clock minus local clock in milliseconds. */
	clockOffsetMs: number;
}

function nonempty(value: unknown): value is string {
	return typeof value === "string" && value.length > 0 && value.length < 32_768;
}

function nonnegative(value: unknown): value is number {
	return Number.isSafeInteger(value) && Number(value) >= 0;
}

/**
 * Only recognized, expiring read grants can activate a provider connection. `expires_in` dates
 * the grant by the server clock, so a skewed local clock neither rejects nor outlives it.
 */
export function parseRealtimeGrant(
	value: Record<string, unknown>,
	receivedAt = Date.now(),
): RealtimeGrant {
	if (
		value.protocol_version !== 1 ||
		!Number.isSafeInteger(value.expires_at) ||
		(value.expires_in != null && !nonnegative(value.expires_in)) ||
		(value.max_poll_wait_ms != null && !nonnegative(value.max_poll_wait_ms))
	)
		throw new RealtimeProviderProtocolError(
			"Unsupported realtime connection grant",
		);
	const expiresAt = Number(value.expires_at);
	const serverNow =
		value.expires_in == null
			? receivedAt
			: (expiresAt - Number(value.expires_in)) * 1000;
	const transport = value.transport as Partial<RealtimeTransport> | undefined;
	if (!transport || typeof transport !== "object")
		throw new RealtimeProviderProtocolError("Invalid realtime transport");
	let fields: string[];
	let expiry = expiresAt;
	switch (transport.type) {
		case "http":
			fields = [];
			break;
		case "nats":
			fields = ["url", "subject", "token"];
			break;
		case "azure_web_pubsub":
			fields = ["url", "group"];
			break;
		case "gcp_firebase_rtdb":
			fields = [
				"database_url",
				"api_key",
				"project_id",
				"custom_token",
				"path",
			];
			break;
		case "aws_mqtt": {
			fields = ["endpoint", "region", "client_id", "topic"];
			const credentials = transport.credentials;
			if (
				!credentials ||
				![
					credentials.access_key_id,
					credentials.secret_access_key,
					credentials.session_token,
				].every(nonempty) ||
				!Number.isSafeInteger(credentials.expiration) ||
				credentials.expiration * 1000 <= serverNow
			)
				throw new RealtimeProviderProtocolError(
					"Invalid AWS realtime credentials",
				);
			expiry = Math.min(expiresAt, credentials.expiration);
			break;
		}
		default:
			throw new RealtimeProviderProtocolError("Unsupported realtime transport");
	}
	if (
		fields.some((key) => !nonempty((transport as Record<string, unknown>)[key]))
	)
		throw new RealtimeProviderProtocolError(
			"Incomplete realtime transport grant",
		);
	const lifetime = expiry * 1000 - serverNow;
	if (lifetime <= 0)
		throw new RealtimeProviderProtocolError("Expired realtime connection grant");
	return {
		...(value as unknown as RealtimeConnectResponse),
		max_poll_wait_ms:
			value.max_poll_wait_ms == null
				? MAX_REALTIME_TIMEOUT_MS
				: Number(value.max_poll_wait_ms),
		deadline: receivedAt + lifetime,
		clockOffsetMs: serverNow - receivedAt,
	};
}

function loopback(hostname: string): boolean {
	return (
		hostname === "localhost" ||
		hostname.endsWith(".localhost") ||
		hostname === "[::1]" ||
		/^127(?:\.\d{1,3}){3}$/.test(hostname)
	);
}

/** Wakeups for an https API never travel over a cleartext socket that leaves this machine. */
export function refusesCleartextGrant(
	api: URL,
	transport: RealtimeTransport,
): boolean {
	if (api.protocol !== "https:" || !("url" in transport)) return false;
	try {
		const url = new URL(transport.url);
		return (
			url.protocol !== "wss:" &&
			url.protocol !== "https:" &&
			!loopback(url.hostname)
		);
	} catch {
		return true;
	}
}

/**
 * Negotiates the account's read grant: a provider connection that wakes the authoritative HTTP
 * replay, or an HTTP grant that tells the replay how long the API holds a poll. Credentials
 * change in place; only a new API or transport preference needs a new grant.
 */
export class RealtimeProviderSession {
	connected = false;
	maxPollWaitMs = MAX_REALTIME_TIMEOUT_MS;
	private session?: RealtimeConnectionSession;
	private preference: RealtimeTransportPreference = "configured";
	private key?: string;
	private controller?: AbortController;
	private connection?: RealtimeProviderConnection;
	private connectedAt = 0;
	private timer?: ReturnType<typeof setTimeout>;
	private epoch = 0;
	private failures = 0;
	private blockedFailures = 0;
	private notBefore = 0;
	private rejected = false;
	private nonces = new Set<string>();

	constructor(
		private readonly options: {
			fetch: Fetch;
			changed(): void;
			wakeup(): void;
			open?: RealtimeProviderOpener;
		},
	) {}

	sync(
		session: RealtimeConnectionSession,
		preference: RealtimeTransportPreference,
	): void {
		const key = JSON.stringify([session.url, preference]);
		if (this.key === key) {
			const renewed =
				this.rejected &&
				JSON.stringify(session.headers) !==
					JSON.stringify(this.session?.headers);
			this.session = session;
			if (renewed) {
				this.rejected = false;
				this.notBefore = 0;
				void this.connect();
			}
			return;
		}
		this.close();
		this.key = key;
		this.session = session;
		this.preference = preference;
		void this.connect();
	}

	close(): void {
		this.epoch++;
		this.key = undefined;
		this.session = undefined;
		this.rejected = false;
		clearTimeout(this.timer);
		this.timer = undefined;
		this.controller?.abort();
		this.controller = undefined;
		this.release();
		this.nonces.clear();
	}

	/** A connection that stayed up long enough proves the provider healthy again. */
	private release(): boolean {
		const live = this.connected;
		if (live && Date.now() - this.connectedAt >= STABLE_CONNECTION_MS)
			this.failures = 0;
		this.connection?.close();
		this.connection = undefined;
		this.connected = false;
		return live;
	}

	/** Every grant costs the API a provider credential, so no retry outpaces this schedule. */
	private retry(epoch: number, blocked = false): void {
		if (this.epoch !== epoch || !this.session) return;
		this.controller?.abort();
		const lost = this.release();
		const delay = blocked
			? backoff(BLOCKED_RETRY_MIN_MS, this.blockedFailures++)
			: backoff(RECONNECT_MIN_MS, this.failures++);
		this.notBefore = Date.now() + delay;
		clearTimeout(this.timer);
		this.timer = setTimeout(() => void this.connect(), delay);
		if (lost) this.options.changed();
	}

	private async connect(): Promise<void> {
		const session = this.session;
		if (!session) return;
		clearTimeout(this.timer);
		this.timer = undefined;
		const wait = this.notBefore - Date.now();
		if (wait > 0) {
			this.timer = setTimeout(() => void this.connect(), wait);
			return;
		}
		const epoch = ++this.epoch;
		const preference = this.preference;
		this.controller?.abort();
		if (this.release()) this.options.changed();
		const controller = new AbortController();
		this.controller = controller;
		const deadline = setTimeout(
			() => controller.abort(),
			NEGOTIATION_TIMEOUT_MS,
		);
		const current = () => this.epoch === epoch && !controller.signal.aborted;
		try {
			const api = new URL(session.url);
			if (!api.pathname.endsWith("/realtime/poll")) return;
			const url = new URL(api);
			url.pathname = `${api.pathname.slice(0, -4)}connect`;
			const { fetch: fetcher } = this.options;
			const response = await fetcher(url, {
				method: "POST",
				headers: { ...session.headers, "Content-Type": "application/json" },
				body: JSON.stringify({ transport: preference }),
				signal: controller.signal,
				credentials: "omit",
				cache: "no-store",
				redirect: "error",
			});
			const receivedAt = Date.now();
			if (!current()) {
				await response.body?.cancel();
				return;
			}
			if (!response.ok) {
				await response.body?.cancel();
				this.rejected = response.status === 401 || response.status === 403;
				this.retry(epoch, [400, 403, 404, 405, 501].includes(response.status));
				return;
			}
			let grant: RealtimeGrant;
			try {
				grant = parseRealtimeGrant(await boundedJson(response), receivedAt);
			} catch (error) {
				this.retry(epoch, error instanceof RealtimeProviderProtocolError);
				return;
			}
			if (!current()) return;
			this.rejected = false;
			this.blockedFailures = 0;
			const waitChanged = this.maxPollWaitMs !== grant.max_poll_wait_ms;
			this.maxPollWaitMs = grant.max_poll_wait_ms;
			if (grant.transport.type !== "http" && preference === "configured") {
				if (refusesCleartextGrant(api, grant.transport))
					throw new Error("Realtime provider grant is not encrypted");
				const connection = await (this.options.open ?? openRealtimeProvider)(
					grant.transport,
					{
						wakeup: (message) => {
							if (!current() || this.nonces.has(message.nonce)) return;
							this.nonces.add(message.nonce);
							if (this.nonces.size > 256)
								this.nonces.delete(this.nonces.values().next().value as string);
							this.options.wakeup();
						},
						disconnected: () => this.retry(epoch),
					},
					controller.signal,
					{ clockOffsetMs: grant.clockOffsetMs },
				);
				if (!current()) {
					connection.close();
					return;
				}
				this.connection = connection;
				this.connectedAt = Date.now();
				this.connected = true;
				this.options.changed();
			} else if (waitChanged) this.options.changed();
			const lifetime = grant.deadline - Date.now();
			this.timer = setTimeout(
				() => void this.connect(),
				Math.max(
					1_000,
					Math.min(2_147_483_647, lifetime - Math.min(30_000, lifetime / 5)),
				),
			);
		} catch {
			this.retry(epoch);
		} finally {
			clearTimeout(deadline);
		}
	}
}
