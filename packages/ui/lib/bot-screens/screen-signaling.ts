import type { IBotScreenAccess } from "../../state/backend-state/bot-state";
import { scopedSignalingUrl } from "../realtime/authenticated-websocket";
import { randomHex } from "./base64";
import { openEnvelope, sealEnvelope } from "./screen-crypto";
import {
	type ScreenSignal,
	type ScreenSignalDraft,
	parseSignal,
} from "./screen-protocol";

const REALTIME_PROTOCOL = "flowlike.realtime.v1";
const TOKEN_PROTOCOL_PREFIX = "flowlike.jwt.";
const PING_MS = 25_000;
const RENEW_BEFORE_MS = 60_000;
const KEY_REFRESH_MIN_MS = 30_000;
const SEND_WAIT_MS = 10_000;
const MAX_BACKOFF_MS = 10_000;

export interface SocketLike {
	onopen: ((event: unknown) => void) | null;
	onmessage: ((event: { data: unknown }) => void) | null;
	onclose: ((event: unknown) => void) | null;
	onerror: ((event: unknown) => void) | null;
	send(data: string): void;
	close(): void;
}

export interface SignalingOptions {
	access: () => Promise<IBotScreenAccess>;
	viewerId?: string;
	now?: () => number;
	createSocket?: (url: string, protocols: string[]) => SocketLike;
}

interface Deferred {
	promise: Promise<IBotScreenAccess>;
	resolve: (access: IBotScreenAccess) => void;
	reject: (error: Error) => void;
}

function deferred(): Deferred {
	let resolve!: Deferred["resolve"];
	let reject!: Deferred["reject"];
	const promise = new Promise<IBotScreenAccess>((ok, fail) => {
		resolve = ok;
		reject = fail;
	});
	promise.catch(() => {});
	return { promise, resolve, reject };
}

const defaultSocket = (url: string, protocols: string[]): SocketLike =>
	new WebSocket(url, protocols) as unknown as SocketLike;

/**
 * One signaling socket on the account's bot-screens topic: subscribes, seals every message with
 * the newest key, drops what it cannot open, pings, and reconnects with fresh access before the
 * token ends or the socket drops. The first connection failing rejects `ready`; later drops retry.
 */
export class ScreenSignaling {
	readonly viewerId: string;
	private readonly now: () => number;
	private readonly createSocket: NonNullable<SignalingOptions["createSocket"]>;
	private readonly listeners = new Set<(signal: ScreenSignal) => void>();
	private access: IBotScreenAccess | null = null;
	private socket: SocketLike | null = null;
	private current = deferred();
	private connected = false;
	private everConnected = false;
	private settled = false;
	private closed = false;
	private ping: ReturnType<typeof setInterval> | null = null;
	private renew: ReturnType<typeof setTimeout> | null = null;
	private retry: ReturnType<typeof setTimeout> | null = null;
	private backoff = 1000;
	private lastKeyRefresh = 0;

	constructor(private readonly options: SignalingOptions) {
		this.viewerId = options.viewerId ?? randomHex(16);
		this.now = options.now ?? Date.now;
		this.createSocket = options.createSocket ?? defaultSocket;
		void this.connect();
	}

	get iceServers() {
		return this.access?.ice_servers;
	}

	get iceExpiresAt() {
		return this.access?.ice_expires_at;
	}

	subscribe(listener: (signal: ScreenSignal) => void): () => void {
		this.listeners.add(listener);
		return () => {
			this.listeners.delete(listener);
		};
	}

	ready(): Promise<IBotScreenAccess> {
		return this.current.promise;
	}

	async send(message: ScreenSignalDraft): Promise<void> {
		let timer: ReturnType<typeof setTimeout> | undefined;
		const timeout = new Promise<never>((_, reject) => {
			timer = setTimeout(
				() => reject(new Error("The signaling server did not answer")),
				SEND_WAIT_MS,
			);
		});
		const access = await Promise.race([this.current.promise, timeout]).finally(
			() => clearTimeout(timer),
		);
		const envelope = await sealEnvelope(
			access.keys[0],
			access.room,
			JSON.stringify({ ...message, at: this.now() }),
		);
		if (!this.socket || !this.connected)
			throw new Error("The signaling connection is closed");
		this.socket.send(
			JSON.stringify({ type: "publish", topic: access.topic, data: envelope }),
		);
	}

	close(): void {
		this.closed = true;
		this.current.reject(new Error("The signaling connection is closed"));
		this.teardown();
	}

	private teardown(): void {
		for (const timer of [this.renew, this.retry])
			if (timer) clearTimeout(timer);
		if (this.ping) clearInterval(this.ping);
		this.renew = this.retry = this.ping = null;
		const socket = this.socket;
		this.socket = null;
		this.connected = false;
		if (socket) {
			socket.onopen = socket.onmessage = socket.onclose = socket.onerror = null;
			try {
				socket.close();
			} catch {
				// already closed
			}
		}
	}

	private async connect(): Promise<void> {
		try {
			const access = await this.options.access();
			if (this.closed) return;
			if (access.keys.length === 0 || access.signaling_servers.length === 0)
				throw new Error("The hub returned no signaling access");
			const url = await scopedSignalingUrl(
				access.signaling_servers[0],
				access.topic,
			);
			if (this.closed) return;
			this.open(access, url);
		} catch (error) {
			this.failed(error instanceof Error ? error : new Error(String(error)));
		}
	}

	private open(access: IBotScreenAccess, url: string): void {
		this.access = access;
		const socket = this.createSocket(url, [
			REALTIME_PROTOCOL,
			`${TOKEN_PROTOCOL_PREFIX}${access.jwt}`,
		]);
		this.socket = socket;
		socket.onopen = () => {
			socket.send(
				JSON.stringify({ type: "subscribe", topics: [access.topic] }),
			);
			this.connected = true;
			this.everConnected = true;
			this.settled = true;
			this.backoff = 1000;
			this.current.resolve(access);
			this.ping = setInterval(() => {
				socket.send(JSON.stringify({ type: "ping" }));
			}, PING_MS);
			const wait = Math.max(
				5000,
				access.expires_at * 1000 - this.now() - RENEW_BEFORE_MS,
			);
			this.renew = setTimeout(() => this.reconnect(), wait);
		};
		socket.onmessage = (event) => void this.receive(access, event.data);
		socket.onclose = () =>
			this.failed(new Error("The signaling socket closed"));
		socket.onerror = () => {};
	}

	private reconnect(): void {
		if (this.closed) return;
		this.renewCurrent();
		this.teardown();
		void this.connect();
	}

	private renewCurrent(): void {
		if (!this.settled) return;
		this.settled = false;
		this.current = deferred();
	}

	private failed(error: Error): void {
		if (this.closed) return;
		this.teardown();
		if (!this.everConnected) {
			this.closed = true;
			this.current.reject(error);
			return;
		}
		this.renewCurrent();
		const wait = this.backoff;
		this.backoff = Math.min(this.backoff * 2, MAX_BACKOFF_MS);
		this.retry = setTimeout(() => void this.connect(), wait);
	}

	private async receive(
		access: IBotScreenAccess,
		data: unknown,
	): Promise<void> {
		if (typeof data !== "string") return;
		let frame: { type?: unknown; topic?: unknown; data?: unknown };
		try {
			frame = JSON.parse(data);
		} catch {
			return;
		}
		if (frame?.type !== "publish" || frame.topic !== access.topic) return;
		let opened = await openEnvelope(frame.data, access.room, access.keys);
		if (opened.status === "unknown_key") {
			const now = this.now();
			if (now - this.lastKeyRefresh < KEY_REFRESH_MIN_MS) return;
			this.lastKeyRefresh = now;
			try {
				const fresh = await this.options.access();
				if (this.closed) return;
				this.access = fresh;
				opened = await openEnvelope(frame.data, fresh.room, fresh.keys);
			} catch {
				return;
			}
		}
		if (opened.status !== "ok") return;
		const signal = parseSignal(opened.text, this.now());
		if (signal) for (const listener of this.listeners) listener(signal);
	}
}
