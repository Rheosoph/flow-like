import { RequestTimeoutError } from "../request-deadline";
import type { IChannelHandle } from "../schema/channel";
import type { ILogMetadata } from "../schema/flow/log-metadata";
import { attachExecutionTransportOwner } from "./owners";
import { channelRequestKey, executionChannels } from "./runtime";
import {
	type ExecutionAuthorization,
	type ExecutionCallbacks,
	type ExecutionCancelOptions,
	type ExecutionCommand,
	type ExecutionMessage,
	type ExecutionMetadata,
	type ExecutionTask,
	type PreparedExecutionRequest,
	executionError,
	isExecutionFinished,
} from "./types";

interface ClientPort {
	postMessage(message: ExecutionCommand | ExecutionAuthorization): void;
	onmessage: ((event: MessageEvent<ExecutionMessage>) => void) | null;
	start?(): void;
	close?(): void;
}

interface Observer {
	callbacks: ExecutionCallbacks;
	seen: number;
	runId?: string;
}

type CommandInput = ExecutionCommand extends infer Command
	? Command extends ExecutionCommand
		? Omit<Command, "id">
		: never
	: never;

const channelOwners = new Map<string, ExecutionRuntimeClient>();

/** One scoped connection. Closing it releases a tab, never the worker's executions. */
export class ExecutionRuntimeClient {
	private readonly tasks = new Map<string, ExecutionTask>();
	private readonly listeners = new Set<() => void>();
	private readonly pending = new Map<
		string,
		{ resolve: (value: unknown) => void; reject: (error: Error) => void }
	>();
	private readonly observers = new Map<string, Observer>();
	private readonly cancelAuthorizers = new Map<
		string,
		ExecutionCallbacks["onUnauthorized"]
	>();
	private channels = new Set<string>();
	private closed = false;
	readonly ready: Promise<void>;

	constructor(
		readonly scope: string,
		private readonly port: ClientPort,
		readonly mode: "shared-worker" | "in-tab",
	) {
		port.onmessage = (event) => this.receive(event.data);
		port.start?.();
		this.ready = this.command({ type: "connect", scope }) as Promise<void>;
	}

	getTasks(): ExecutionTask[] {
		return [...this.tasks.values()];
	}
	isOpen(): boolean {
		return !this.closed;
	}
	getTask(taskId: string): ExecutionTask | undefined {
		return this.tasks.get(taskId);
	}
	subscribe(listener: () => void): () => void {
		this.listeners.add(listener);
		return () => this.listeners.delete(listener);
	}

	private notify() {
		for (const listener of this.listeners) {
			try {
				listener();
			} catch (error) {
				console.error("Execution listener failed", error);
			}
		}
	}

	private updateObserver(task: ExecutionTask) {
		const observer = this.observers.get(task.taskId);
		if (!observer) return;
		if (task.runId && observer.runId !== task.runId) {
			observer.runId = task.runId;
			try {
				observer.callbacks.onExecutionStart?.(task.runId);
			} catch (error) {
				console.error("Execution start callback failed", error);
			}
		}
		const events = task.events.slice(observer.seen);
		observer.seen = task.events.length;
		if (events.length) {
			try {
				observer.callbacks.onEvents?.(events);
			} catch (error) {
				console.error("Execution event callback failed", error);
			}
		}
	}

	private receive(message: ExecutionMessage) {
		if (this.closed) return;
		if (message.type === "result") {
			const pending = this.pending.get(message.id);
			if (!pending) return;
			this.pending.delete(message.id);
			if (message.error) {
				const data = message.error;
				const error =
					data.name === "RequestTimeoutError"
						? new RequestTimeoutError(data.url ?? "", data.timeoutMs ?? 900_000)
						: data.name === "TypeError"
							? new TypeError(data.message)
							: new Error(data.message);
				pending.reject(Object.assign(error, data));
			} else pending.resolve(message.value);
			return;
		}
		if (message.type === "unauthorized") {
			const callback =
				this.cancelAuthorizers.get(message.taskId) ??
				this.observers.get(message.taskId)?.callbacks.onUnauthorized;
			void Promise.resolve()
				.then(() => callback?.(message.rejectedToken))
				.then(
					(headers) => {
						if (!this.closed)
							this.port.postMessage({
								type: "authorize",
								id: message.id,
								headers,
							});
					},
					(error) => {
						if (!this.closed)
							this.port.postMessage({
								type: "authorize",
								id: message.id,
								error: executionError(error),
							});
					},
				);
			return;
		}
		if (message.type === "response") {
			try {
				this.observers.get(message.taskId)?.callbacks.onResponse?.();
			} catch (error) {
				console.error("Execution response callback failed", error);
			}
			return;
		}
		if (message.type === "snapshot") {
			this.tasks.clear();
			this.channels.clear();
			for (const task of message.tasks)
				if (task.scope === this.scope) {
					this.tasks.set(task.taskId, task);
					for (const handle of executionChannels(task.events))
						this.channels.add(channelRequestKey(handle));
				}
		} else {
			if (message.task.scope !== this.scope) return;
			const previous = this.tasks.get(message.task.taskId);
			if (message.offset > (previous?.events.length ?? 0)) {
				this.fail(
					new Error(
						"Execution replay lost its event order. Reconnect to recover the run.",
					),
				);
				return;
			}
			this.tasks.set(message.task.taskId, {
				...message.task,
				events: [
					...(previous?.events.slice(0, message.offset) ?? []),
					...message.task.events,
				],
			});
			for (const handle of executionChannels(message.task.events))
				this.channels.add(channelRequestKey(handle));
		}
		for (const key of this.channels) channelOwners.set(key, this);
		// The engine learns persistence ownership before transport callbacks can save the same reply.
		this.notify();
		for (const task of this.tasks.values()) this.updateObserver(task);
	}

	private command(command: CommandInput): Promise<unknown> {
		if (this.closed)
			return Promise.reject(new Error("Execution session is disconnected"));
		const id = crypto.randomUUID();
		return new Promise((resolve, reject) => {
			this.pending.set(id, { resolve, reject });
			try {
				this.port.postMessage({ ...command, id } as ExecutionCommand);
			} catch (error) {
				this.pending.delete(id);
				reject(error);
			}
		});
	}

	async execute(
		metadata: ExecutionMetadata,
		request: PreparedExecutionRequest,
		callbacks: ExecutionCallbacks = {},
	): Promise<ILogMetadata | undefined> {
		await this.ready;
		if (metadata.scope !== this.scope)
			throw new Error("Execution scope changed before dispatch");
		attachExecutionTransportOwner(metadata.taskId, this);
		this.observers.set(metadata.taskId, { callbacks, seen: 0 });
		try {
			return (await this.command({ type: "execute", metadata, request })) as
				| ILogMetadata
				| undefined;
		} finally {
			this.observers.delete(metadata.taskId);
		}
	}

	cancel(taskId: string, options: ExecutionCancelOptions = {}): Promise<void> {
		const task =
			this.tasks.get(taskId) ??
			this.getTasks().find((item) => item.runId === taskId);
		if (task && options.onUnauthorized)
			this.cancelAuthorizers.set(task.taskId, options.onUnauthorized);
		return this.command({
			type: "cancel",
			taskId,
			headers: options.headers,
		}) as Promise<void>;
	}
	dismiss(taskId: string): Promise<void> {
		return this.command({ type: "dismiss", taskId }) as Promise<void>;
	}
	submitReply(handle: IChannelHandle, value: unknown): Promise<void> {
		return this.command({ type: "reply", handle, value }) as Promise<void>;
	}
	claimRequest(handle: IChannelHandle): Promise<boolean> {
		return this.command({ type: "claim", handle }) as Promise<boolean>;
	}
	claimEvent(taskId: string, eventId: string): Promise<boolean> {
		return this.command({
			type: "claim-event",
			taskId,
			eventId,
		}) as Promise<boolean>;
	}
	ownsChannel(handle: IChannelHandle): boolean {
		return !this.closed && this.channels.has(channelRequestKey(handle));
	}

	fail(error: Error) {
		if (this.closed) return;
		for (const task of this.tasks.values()) {
			if (!isExecutionFinished(task)) {
				task.status = "failed";
				task.error = error.message;
			}
		}
		this.notify();
		for (const pending of this.pending.values()) pending.reject(error);
		this.pending.clear();
		this.close();
	}

	close() {
		if (this.closed) return;
		try {
			this.port.postMessage({ type: "disconnect", id: crypto.randomUUID() });
		} catch {
			/* The worker may have already stopped. */
		}
		this.closed = true;
		this.port.close?.();
		for (const pending of this.pending.values())
			pending.reject(new Error("Execution session changed"));
		this.pending.clear();
		this.observers.clear();
		this.listeners.clear();
		evictClient(this);
	}
}

const clients = new Map<string, Promise<ExecutionRuntimeClient>>();
const connected = new Set<ExecutionRuntimeClient>();
const scopedClients = new Map<string, ExecutionRuntimeClient>();
let fallbackRuntime: Promise<import("./runtime").ExecutionRuntime> | undefined;

function evictClient(client: ExecutionRuntimeClient) {
	connected.delete(client);
	const pending = clients.get(client.scope);
	void pending
		?.then((current) => {
			if (current === client && clients.get(client.scope) === pending)
				clients.delete(client.scope);
		})
		.catch(() => undefined);
}

async function fallback(scope: string): Promise<ExecutionRuntimeClient> {
	if (!fallbackRuntime)
		fallbackRuntime = Promise.all([
			import("./runtime"),
			import("../channel/transport"),
			import("./chat-persistence"),
		]).then(
			([{ ExecutionRuntime }, { rawPushChannel }, { persistExecutionChat }]) =>
				new ExecutionRuntime({
					push: rawPushChannel,
					persist: persistExecutionChat,
				}),
		);
	const runtime = await fallbackRuntime;
	const port: ClientPort = {
		onmessage: null,
		postMessage: (command) => {
			const cloned = structuredClone(command);
			queueMicrotask(() => {
				void runtime.receive(workerPort, cloned);
			});
		},
	};
	const workerPort = {
		postMessage: (message: ExecutionMessage) => {
			const cloned = structuredClone(message);
			queueMicrotask(() =>
				port.onmessage?.({ data: cloned } as MessageEvent<ExecutionMessage>),
			);
		},
	};
	return new ExecutionRuntimeClient(scope, port, "in-tab");
}

async function connect(scope: string): Promise<ExecutionRuntimeClient> {
	let client: ExecutionRuntimeClient;
	if (typeof SharedWorker !== "undefined" && typeof window !== "undefined") {
		let workerClient: ExecutionRuntimeClient | undefined;
		let timer: ReturnType<typeof setTimeout> | undefined;
		try {
			const worker = new SharedWorker(
				new URL("./runtime.shared-worker.ts", import.meta.url),
				{ type: "module", name: "flow-like-executions-v1" },
			);
			workerClient = new ExecutionRuntimeClient(
				scope,
				worker.port,
				"shared-worker",
			);
			worker.onerror = () =>
				workerClient?.fail(
					new Error(
						"The execution worker disconnected. Reopen the conversation to recover saved replies.",
					),
				);
			await Promise.race([
				workerClient.ready,
				new Promise((_, reject) => {
					timer = setTimeout(
						() => reject(new Error("Execution worker did not start")),
						3_000,
					);
				}),
			]);
			client = workerClient;
		} catch (error) {
			workerClient?.close();
			console.warn(
				"Shared execution worker unavailable; runs remain in this tab.",
				executionError(error).message,
			);
			client = await fallback(scope);
		} finally {
			clearTimeout(timer);
		}
	} else client = await fallback(scope);
	await client.ready;
	connected.add(client);
	scopedClients.set(scope, client);
	return client;
}

export function getExecutionRuntime(
	scope: string,
): Promise<ExecutionRuntimeClient> {
	if (scopedClients.get(scope)?.isOpen() === false) {
		clients.delete(scope);
		scopedClients.delete(scope);
	}
	let client = clients.get(scope);
	if (!client) {
		client = connect(scope);
		clients.set(scope, client);
		void client.catch(() => clients.delete(scope));
	}
	return client;
}

/** Disconnect this account's tab connection without cancelling executions in other tabs. */
export function releaseExecutionRuntime(scope: string) {
	const pending = clients.get(scope);
	clients.delete(scope);
	scopedClients.delete(scope);
	void pending?.then((client) => {
		connected.delete(client);
		client.close();
	});
}

export function getExecutionClientForChannel(
	handle: IChannelHandle,
): ExecutionRuntimeClient | undefined {
	return (
		[...connected].find((client) => client.ownsChannel(handle)) ??
		channelOwners.get(channelRequestKey(handle))
	);
}

/** Isolated test lifetimes must not keep earlier mocked transports or finished task registries. */
export async function resetExecutionRuntimes(): Promise<void> {
	const previous = [...clients.values()];
	clients.clear();
	fallbackRuntime = undefined;
	for (const pending of previous) {
		try {
			(await pending).close();
		} catch {}
	}
	connected.clear();
	scopedClients.clear();
	channelOwners.clear();
}

if (typeof window !== "undefined") {
	window.addEventListener("pagehide", (event: PageTransitionEvent) => {
		if (event.persisted) return;
		for (const client of [...connected]) client.close();
	});
}
