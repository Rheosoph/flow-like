import { compareInboxCursors } from "../inbox-cursor";
import { RealtimeRuntime, validRealtimeSubscription } from "./runtime";
import {
	REALTIME_ERROR_UNAVAILABLE,
	type RealtimeActivity,
	type RealtimeClient,
	type RealtimeClientOptions,
	type RealtimeCommand,
	type RealtimeConnectionSession,
	type RealtimeConnectionState,
	type RealtimeListener,
	type RealtimeMessage,
	type RealtimePort,
	type RealtimeSubscription,
	type RealtimeUpdate,
} from "./types";

interface ClientPort {
	onmessage: ((event: MessageEvent<RealtimeMessage>) => void) | null;
	postMessage(command: RealtimeCommand): void;
	start?(): void;
	close?(): void;
}

let fallbackRuntime: RealtimeRuntime | undefined;

function runtimePort(runtime: RealtimeRuntime): ClientPort {
	const port: ClientPort = {
		onmessage: null,
		postMessage(command) {
			queueMicrotask(() => runtime.receive(worker, structuredClone(command)));
		},
	};
	const worker: RealtimePort = {
		postMessage(message) {
			queueMicrotask(() =>
				port.onmessage?.({
					data: structuredClone(message),
				} as MessageEvent<RealtimeMessage>),
			);
		},
	};
	return port;
}

/** Each caller owns a port. The worker shares networking, never observer callbacks. */
export class RealtimePortClient implements RealtimeClient {
	private readonly id = crypto.randomUUID();
	private readonly listeners = new Set<RealtimeListener>();
	private readonly accepted = new Set<string>();
	private readonly processing = new Set<string>();
	private session: RealtimeConnectionSession;
	private subscription: RealtimeSubscription = {
		notifications: false,
		runs: [],
	};
	private subscriptionVersion = 0;
	private activity: RealtimeActivity = { online: true, visible: true };
	private state: RealtimeConnectionState = "connecting";
	private port?: ClientPort;
	private worker?: SharedWorker;
	private closed = false;
	private ready = false;
	private resolveReady?: () => void;
	private rejectReady?: (reason: Error) => void;
	private heartbeat?: ReturnType<typeof setInterval>;
	private fallingBack?: Promise<void>;
	mode: "shared-worker" | "in-tab" = "in-tab";

	constructor(
		session: RealtimeConnectionSession,
		private readonly options: RealtimeClientOptions = {},
		private readonly localRuntime?: RealtimeRuntime,
		private readonly createWorker?: () => SharedWorker | undefined,
	) {
		this.session = structuredClone(session);
	}

	async connect(): Promise<void> {
		if (this.localRuntime) await this.attach(runtimePort(this.localRuntime));
		else {
			let deadline: ReturnType<typeof setTimeout> | undefined;
			let needsFallback = false;
			try {
				this.worker = this.createWorker?.();
				if (this.worker) {
					this.mode = "shared-worker";
					this.worker.onerror = () => {
						if (this.ready) void this.useFallback();
						else
							this.rejectReady?.(new Error("Realtime worker failed to start"));
					};
					await Promise.race([
						this.attach(this.worker.port),
						new Promise<never>((_, reject) => {
							deadline = setTimeout(
								() => reject(new Error("Realtime worker did not start")),
								3_000,
							);
						}),
					]);
				} else needsFallback = true;
			} catch {
				needsFallback = true;
			} finally {
				clearTimeout(deadline);
			}
			if (needsFallback) await this.useFallback();
		}
		if (this.closed) return;
		this.heartbeat = setInterval(
			() =>
				this.send({ type: "heartbeat", id: this.id, activity: this.activity }),
			15_000,
		);
	}

	setSession(session: RealtimeConnectionSession): void {
		if (
			session.scope !== this.session.scope ||
			session.url !== this.session.url
		)
			throw new Error(
				"Create a new realtime client when the account or hub changes",
			);
		this.session = structuredClone(session);
		this.send({ type: "session", id: this.id, session: this.session });
	}

	setSubscriptions(subscription: RealtimeSubscription): void {
		if (!validRealtimeSubscription(subscription))
			throw new Error("Invalid realtime subscriptions");
		this.subscription = structuredClone(subscription);
		this.subscriptionVersion++;
		this.send({
			type: "subscribe",
			id: this.id,
			subscription: this.subscription,
			subscription_version: this.subscriptionVersion,
		});
	}

	setActivity(activity: RealtimeActivity): void {
		this.activity = { ...activity };
		this.send({ type: "heartbeat", id: this.id, activity: this.activity });
	}

	subscribe(listener: RealtimeListener): () => void {
		if (this.closed) return () => {};
		this.listeners.add(listener);
		void Promise.resolve()
			.then(() => {
				if (!this.closed && this.listeners.has(listener))
					return listener({ type: "state", state: this.state });
			})
			.catch(() => {});
		return () => {
			this.listeners.delete(listener);
		};
	}

	close(): void {
		if (this.closed) return;
		this.send({ type: "disconnect", id: this.id });
		this.closed = true;
		clearInterval(this.heartbeat);
		this.detach();
		this.listeners.clear();
		this.accepted.clear();
		this.processing.clear();
		this.subscription = { notifications: false, runs: [] };
		this.session = { ...this.session, headers: {} };
		this.state = "closed";
	}

	private attach(port: ClientPort): Promise<void> {
		this.port = port;
		this.ready = false;
		const ready = new Promise<void>((resolve, reject) => {
			this.resolveReady = resolve;
			this.rejectReady = reject;
		});
		port.onmessage = (event) => {
			if (this.port === port) this.receive(event.data);
		};
		port.start?.();
		this.send({
			type: "connect",
			id: this.id,
			session: this.session,
			activity: this.activity,
		});
		return ready;
	}

	private detach(): void {
		if (this.worker) this.worker.onerror = null;
		this.worker = undefined;
		if (this.port) {
			this.port.onmessage = null;
			this.port.close?.();
		}
		this.port = undefined;
	}

	private useFallback(): Promise<void> {
		if (this.fallingBack) return this.fallingBack;
		this.fallingBack = (async () => {
			if (this.closed) return;
			this.send({ type: "disconnect", id: this.id });
			this.detach();
			this.mode = "in-tab";
			fallbackRuntime ??= new RealtimeRuntime();
			await this.attach(runtimePort(fallbackRuntime));
		})().finally(() => {
			this.fallingBack = undefined;
		});
		return this.fallingBack;
	}

	private send(command: RealtimeCommand): void {
		if (this.closed) return;
		try {
			this.port?.postMessage(command);
		} catch {
			if (this.ready && this.mode === "shared-worker" && !this.fallingBack)
				void this.useFallback();
		}
	}

	private receive(message: RealtimeMessage): void {
		if (this.closed) return;
		if (message.type === "ready") {
			this.ready = true;
			this.resolveReady?.();
			this.send({
				type: "subscribe",
				id: this.id,
				subscription: this.subscription,
				subscription_version: this.subscriptionVersion,
			});
			return;
		}
		if (message.type === "expired") {
			this.ready = false;
			this.send({
				type: "connect",
				id: this.id,
				session: this.session,
				activity: this.activity,
			});
			return;
		}
		if (message.type === "state") {
			this.state = message.state;
			this.notify(message);
			return;
		}
		if (message.type === "unauthorized") {
			const currentPort = this.port;
			void Promise.resolve()
				.then(() => this.options.onUnauthorized?.(message.rejectedToken))
				.then(
					(headers) => {
						if (this.closed || this.port !== currentPort) return;
						if (headers) this.session = { ...this.session, headers };
						this.send({
							type: "authorize",
							id: this.id,
							request_id: message.request_id,
							headers,
						});
					},
					() => {
						if (!this.closed && this.port === currentPort)
							this.send({
								type: "authorize",
								id: this.id,
								request_id: message.request_id,
								failed: true,
							});
					},
				);
			return;
		}
		if (message.subscription_version !== this.subscriptionVersion) return;
		const subscriptionVersion = this.subscriptionVersion;
		if (this.accepted.has(message.delivery_id)) {
			this.send({
				type: "accepted",
				id: this.id,
				delivery_id: message.delivery_id,
			});
			return;
		}
		if (this.processing.has(message.delivery_id)) return;
		this.processing.add(message.delivery_id);
		const currentPort = this.port;
		void Promise.all(
			[...this.listeners].map((listener) =>
				Promise.resolve().then(() => {
					if (
						!this.closed &&
						this.subscriptionVersion === subscriptionVersion &&
						this.port === currentPort &&
						this.listeners.has(listener)
					)
						return listener({ type: "snapshot", snapshot: message.snapshot });
				}),
			),
		)
			.then(() => {
				if (
					this.closed ||
					this.port !== currentPort ||
					this.subscriptionVersion !== subscriptionVersion
				)
					return;
				for (const run of message.snapshot.runs) {
					const desired = this.subscription.runs.find(
						(item) => item.run_id === run.run_id,
					);
					if (desired) {
						desired.after_sequence = Math.max(
							desired.after_sequence ?? -1,
							run.after_sequence,
						);
						desired.revision = run.revision;
					}
				}
				const conversations = this.subscription.conversations;
				if (conversations) {
					for (const update of message.snapshot.conversations ?? []) {
						const newest = update.entries.at(-1)?.cursor;
						const desired = conversations.find(
							(item) =>
								item.conversation_id === update.conversation_id &&
								item.as === update.as,
						);
						if (desired && newest)
							desired.after =
								desired.after !== undefined &&
								compareInboxCursors(desired.after, newest) >= 0
									? desired.after
									: newest;
					}
					const dropped = (message.snapshot.conversation_errors ?? []).filter(
						(error) => error.code !== REALTIME_ERROR_UNAVAILABLE,
					);
					this.subscription.conversations = conversations.filter(
						(item) =>
							!dropped.some(
								(error) =>
									error.conversation_id === item.conversation_id &&
									error.as === item.as,
							),
					);
				}
				const failed = new Set(
					message.snapshot.errors
						.filter((error) => error.code !== REALTIME_ERROR_UNAVAILABLE)
						.map((error) => error.run_id),
				);
				this.subscription.runs = this.subscription.runs.filter(
					(run) => !failed.has(run.run_id),
				);
				this.accepted.add(message.delivery_id);
				const oldest = this.accepted.values().next().value;
				if (this.accepted.size > 64 && oldest) this.accepted.delete(oldest);
				this.send({
					type: "accepted",
					id: this.id,
					delivery_id: message.delivery_id,
				});
			})
			.catch(() => {
				// The worker retains this batch and resends it on the next heartbeat.
			})
			.finally(() => {
				this.processing.delete(message.delivery_id);
			});
	}

	private notify(update: RealtimeUpdate): void {
		for (const listener of this.listeners)
			void Promise.resolve()
				.then(() => {
					if (!this.closed && this.listeners.has(listener))
						return listener(update);
				})
				.catch(() => {});
	}
}

/** Worker-owned consumers attach to the same bus as browser-tab ports. */
export async function getRealtimeClientForRuntime(
	runtime: RealtimeRuntime,
	session: RealtimeConnectionSession,
	options?: RealtimeClientOptions,
): Promise<RealtimeClient> {
	const client = new RealtimePortClient(session, options, runtime);
	await client.connect();
	return client;
}
