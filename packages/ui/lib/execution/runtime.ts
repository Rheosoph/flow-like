import { isChannelHandle } from "../channel/handle";
import type { RealtimeRunSnapshot } from "../realtime/types";
import type { IChannelHandle, IChannelPush } from "../schema/channel";
import type { IIntercomEvent } from "../schema/events/intercom-event";
import {
	type ExecutionRealtimeClientFactory,
	followRealtimeExecution,
	readExecutionReceipt,
	realtimeExecutionEvent,
} from "./realtime";
import {
	type ExecutionAuthorization,
	type ExecutionCommand,
	ExecutionDetachedError,
	type ExecutionErrorData,
	type ExecutionMetadata,
	type ExecutionPort,
	type ExecutionTask,
	type PreparedExecutionRequest,
	executionError,
	isExecutionFinished,
} from "./types";

const RETAINED_FINISHED_RUNS = 50;
const RETAINED_UNDELIVERED_EVENT_BYTES = 8 * 1024 * 1024;

interface RuntimeOptions {
	fetch?: (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;
	persist?: (task: ExecutionTask, final: boolean) => Promise<void>;
	realtime?: ExecutionRealtimeClientFactory;
	push: (
		handle: IChannelHandle,
		push: Omit<IChannelPush, "channel_id">,
	) => Promise<void>;
}

interface Run {
	task: ExecutionTask;
	request: PreparedExecutionRequest;
	controller: AbortController;
	seen: Set<string>;
	channels: Map<string, IChannelHandle>;
	claims: Map<string, ExecutionPort>;
	claimedEvents: Set<string>;
	pendingRequests: Set<string>;
	replies: Map<string, Promise<void>>;
	saves: Promise<void>;
	lastSave: number;
	finished: boolean;
	failure?: ExecutionErrorData;
	promise: Promise<unknown>;
	expiryTimer?: ReturnType<typeof setTimeout>;
	cancelPort?: ExecutionPort;
	cancelHeaders?: Record<string, string>;
	cancellation?: Promise<void>;
	cancelConfirmed?: boolean;
	/** The server accepted a stop, but the run has not reported its end yet. */
	stopAccepted?: boolean;
	replaying?: boolean;
	endedAt?: number;
	/** Event bytes kept for a tab that has not yet received this finished run. */
	retainedBytes: number;
}

export function channelRequestKey(handle: IChannelHandle): string {
	return `${handle.channel_id}\u0000${handle.request_id ?? ""}`;
}

/** Channel capabilities occur on interactions, device requests, and nested tool payloads. */
export function executionChannels(value: unknown): IChannelHandle[] {
	const channels: IChannelHandle[] = [];
	const visited = new Set<object>();
	const walk = (candidate: unknown, depth: number) => {
		if (
			!candidate ||
			typeof candidate !== "object" ||
			depth > 20 ||
			visited.has(candidate)
		)
			return;
		visited.add(candidate);
		if (isChannelHandle(candidate)) {
			channels.push(candidate);
			return;
		}
		for (const child of Object.values(candidate)) walk(child, depth + 1);
	};
	walk(value, 0);
	return channels;
}

function snapshot(task: ExecutionTask, offset = 0): ExecutionTask {
	return {
		...task,
		events: task.events.slice(offset),
		pendingInteractions: task.pendingInteractions.map((request) => ({
			...request,
		})),
	};
}

function messageText(value: unknown): string | undefined {
	if (typeof value === "string") return value.trim() || undefined;
	if (!value || typeof value !== "object") return undefined;
	for (const key of ["error_message", "message", "error"]) {
		const text = (value as Record<string, unknown>)[key];
		if (typeof text === "string" && text.trim()) return text.trim();
	}
	return undefined;
}

function abortError() {
	return Object.assign(new Error("Execution cancelled"), {
		name: "AbortError",
	});
}

/** The same coordinator runs in a SharedWorker and in the single-tab fallback. */
export class ExecutionRuntime {
	private readonly ports = new Map<ExecutionPort, string>();
	private readonly runs = new Map<string, Run>();
	private readonly authorizations = new Map<
		string,
		{
			port: ExecutionPort;
			resolve: (headers?: Record<string, string>) => void;
			reject: (error: Error) => void;
		}
	>();
	private readonly fetcher: NonNullable<RuntimeOptions["fetch"]>;

	constructor(private readonly options: RuntimeOptions) {
		// WebIDL fetch rejects any receiver except the global ("Illegal invocation").
		this.fetcher = (input, init) =>
			(options.fetch ?? globalThis.fetch)(input, init);
	}

	private key(scope: string, taskId: string) {
		return `${scope}\u0000${taskId}`;
	}

	private send(
		port: ExecutionPort,
		message: Parameters<ExecutionPort["postMessage"]>[0],
	) {
		try {
			port.postMessage(message);
		} catch {
			this.disconnect(port);
		}
	}

	private broadcast(run: Run, offset = run.task.events.length) {
		for (const [port, scope] of this.ports) {
			if (scope === run.task.scope)
				this.send(port, {
					type: "update",
					task: snapshot(run.task, offset),
					offset,
				});
		}
	}

	private connected(scope: string): boolean {
		return [...this.ports.values()].includes(scope);
	}

	private disconnect(port: ExecutionPort) {
		const scope = this.ports.get(port);
		this.ports.delete(port);
		// Replay authorizes through a connected tab of its account; without one it detaches.
		if (scope !== undefined && !this.connected(scope)) {
			for (const run of this.runs.values())
				if (run.task.scope === scope && run.replaying && !run.finished)
					run.controller.abort(
						new ExecutionDetachedError(
							run.task.runId,
							"No tab of this account follows the run any more; it continues on the server.",
						),
					);
		}
		// A claimed device action may already have run when its tab disconnects. Keep the claim
		// rather than repeat a side effect whose result was lost with that tab.
		for (const [id, authorization] of this.authorizations) {
			if (authorization.port !== port) continue;
			authorization.resolve(undefined);
			this.authorizations.delete(id);
		}
	}

	async receive(
		port: ExecutionPort,
		command: ExecutionCommand | ExecutionAuthorization,
	): Promise<void> {
		if (command.type === "authorize") {
			const pending = this.authorizations.get(command.id);
			if (pending?.port === port) {
				this.authorizations.delete(command.id);
				if (command.error)
					pending.reject(
						Object.assign(new Error(command.error.message), command.error),
					);
				else pending.resolve(command.headers);
			}
			return;
		}
		try {
			if (command.type === "connect") {
				this.disconnect(port);
				this.ports.set(port, command.scope);
				const runs = [...this.runs.values()].filter(
					(run) => run.task.scope === command.scope,
				);
				this.send(port, {
					type: "snapshot",
					tasks: runs.map((run) => snapshot(run.task)),
				});
				for (const run of runs) if (run.retainedBytes) this.release(run);
				this.send(port, { type: "result", id: command.id });
				return;
			}
			if (command.type === "disconnect") {
				this.disconnect(port);
				return;
			}
			const scope = this.ports.get(port);
			if (scope === undefined)
				throw new Error("Execution session is disconnected");
			let value: unknown;
			if (command.type === "execute") {
				if (command.metadata.scope !== scope)
					throw new Error("Execution scope changed before dispatch");
				value = await this.start(port, command.metadata, command.request);
			} else if (command.type === "claim-event") {
				const run = this.findRun(scope, command.taskId);
				if (!run || (!run.finished && !run.seen.has(command.eventId)))
					throw new Error("Execution event is no longer available");
				value = !run.claimedEvents.has(command.eventId);
				if (value) run.claimedEvents.add(command.eventId);
			} else if (command.type === "cancel" || command.type === "dismiss") {
				const run = this.findRun(scope, command.taskId);
				if (!run) throw new Error("Execution is no longer available");
				if (command.type === "cancel") {
					run.cancelPort = port;
					if (command.headers) run.cancelHeaders = command.headers;
					await this.cancel(run);
				} else {
					if (!run.finished)
						throw new Error("Only finished executions can be dismissed");
					this.drop(run);
					this.publishScope(scope);
				}
			} else {
				const key = channelRequestKey(command.handle);
				const run = [...this.runs.values()].find(
					(candidate) =>
						candidate.task.scope === scope && candidate.channels.has(key),
				);
				if (!run)
					throw new Error(
						"This request does not belong to the current execution session",
					);
				const handle = run.channels.get(key);
				if (!handle?.request_id)
					throw new Error("The channel does not identify a request");
				if (command.type === "claim") {
					if (
						isExecutionFinished(run.task) ||
						handle.expires_at * 1_000 <= Date.now() ||
						run.replies.has(key) ||
						run.claims.has(key)
					)
						value = false;
					else {
						run.claims.set(key, port);
						value = true;
					}
				} else value = await this.reply(run, handle, command.value);
			}
			if (this.ports.get(port) === scope)
				this.send(port, { type: "result", id: command.id, value });
		} catch (error) {
			this.send(port, {
				type: "result",
				id: command.id,
				error: executionError(error),
			});
		}
	}

	private findRun(scope: string, taskId: string): Run | undefined {
		return (
			this.runs.get(this.key(scope, taskId)) ??
			[...this.runs.values()].find(
				(run) => run.task.scope === scope && run.task.runId === taskId,
			)
		);
	}

	private start(
		port: ExecutionPort,
		metadata: ExecutionMetadata,
		request: PreparedExecutionRequest,
	): Promise<unknown> {
		const existing = this.runs.get(this.key(metadata.scope, metadata.taskId));
		if (existing) return existing.promise;
		if (
			[...this.runs.values()].some(
				(run) =>
					run.task.scope === metadata.scope &&
					run.task.streamId === metadata.streamId &&
					!run.finished,
			)
		) {
			throw Object.assign(
				new Error("A stream is already active for this session"),
				{ name: "ActiveStreamError" },
			);
		}
		const run: Run = {
			task: {
				...metadata,
				status: "starting",
				events: [],
				pendingInteractions: [],
				persistenceManaged: true,
			},
			request,
			controller: new AbortController(),
			seen: new Set(),
			channels: new Map(),
			claims: new Map(),
			claimedEvents: new Set(),
			pendingRequests: new Set(),
			replies: new Map(),
			saves: Promise.resolve(),
			lastSave: 0,
			finished: false,
			promise: Promise.resolve(),
			retainedBytes: 0,
		};
		this.runs.set(this.key(metadata.scope, metadata.taskId), run);
		this.broadcast(run, 0);
		run.promise = this.drive(port, run);
		return run.promise;
	}

	private async authorize(
		port: ExecutionPort,
		run: Run,
		headers = run.request.headers,
		rejectedToken?: string,
	): Promise<Record<string, string> | undefined> {
		const id = `${run.task.taskId}:auth:${crypto.randomUUID()}`;
		let timer: ReturnType<typeof setTimeout> | undefined;
		try {
			return await new Promise((resolve, reject) => {
				timer = setTimeout(() => resolve(undefined), 30_000);
				this.authorizations.set(id, { port, resolve, reject });
				const auth = Object.entries(headers ?? {}).find(
					([key]) => key.toLowerCase() === "authorization",
				)?.[1];
				this.send(port, {
					type: "unauthorized",
					id,
					taskId: run.task.taskId,
					rejectedToken: rejectedToken ?? auth?.replace(/^Bearer\s+/i, ""),
				});
			});
		} finally {
			clearTimeout(timer);
			this.authorizations.delete(id);
		}
	}

	private save(run: Run, final: boolean): Promise<void> {
		const persist = this.options.persist;
		if (!persist || !run.task.chat) return run.saves;
		const task = snapshot(run.task);
		run.saves = run.saves
			.catch(() => undefined)
			.then(() => persist(task, final));
		return run.saves;
	}

	private async drive(port: ExecutionPort, run: Run): Promise<unknown> {
		let terminal = false;
		let replay: RealtimeRunSnapshot | undefined;
		let lastError: string | undefined;
		let hasError = false;
		let deadline: ReturnType<typeof setTimeout> | undefined;
		const observe = (event: IIntercomEvent) => {
			this.acceptEvent(run, event);
			if (event.event_type === "error") {
				hasError = true;
				lastError = messageText(event.payload) ?? lastError;
			}
		};
		try {
			const timeoutMs = run.request.timeoutMs ?? 900_000;
			const deadlineFailure = new Promise<never>((_, reject) => {
				deadline = setTimeout(() => {
					const error = Object.assign(
						new Error("Execution dispatch timed out"),
						{ name: "RequestTimeoutError", timeoutMs, url: run.request.url },
					);
					reject(error);
					run.controller.abort(error);
				}, timeoutMs);
			});
			const send = () =>
				this.fetcher(run.request.url, {
					method: "POST",
					headers: run.request.headers,
					body:
						typeof run.request.body === "string"
							? run.request.body
							: JSON.stringify(run.request.body),
					credentials: run.request.credentials,
					signal: run.controller.signal,
				});
			let response = await Promise.race([send(), deadlineFailure]);
			if (response.status === 401) {
				const headers = await Promise.race([
					this.authorize(port, run),
					deadlineFailure,
				]);
				if (headers) {
					await response.body?.cancel();
					run.request.headers = headers;
					response = await Promise.race([send(), deadlineFailure]);
				}
			}
			clearTimeout(deadline);
			if (!response.ok) {
				const body = await response.text();
				throw Object.assign(
					new Error(`Execution request failed (${response.status})`),
					{
						status: response.status,
						statusText: response.statusText,
						headers: Object.fromEntries(response.headers.entries()),
						body,
						url: run.request.url,
					},
				);
			}
			if (!response.body)
				throw new Error("The server did not return an execution stream.");
			if (this.ports.has(port))
				this.send(port, { type: "response", taskId: run.task.taskId });
			run.task.status = "running";
			this.broadcast(run);
			if (
				response.headers
					.get("Content-Type")
					?.toLowerCase()
					.includes("application/json")
			) {
				deadline = setTimeout(
					() => run.controller.abort(new Error("Execution receipt timed out")),
					15_000,
				);
				const receipt = await readExecutionReceipt(response);
				clearTimeout(deadline);
				run.replaying = true;
				// The run id is known before replay starts, so a detached run can still be stopped.
				observe({
					event_id: `${receipt.run_id}:receipt`,
					event_type: "run_initiated",
					payload: { run_id: receipt.run_id },
					timestamp: {
						secs_since_epoch: Math.floor(Date.now() / 1_000),
						nanos_since_epoch: 0,
					},
				});
				const realtime = run.request.realtime;
				if (!this.options.realtime || !realtime)
					throw new ExecutionDetachedError(
						receipt.run_id,
						"This execution was queued without an account replay session; it continues on the server.",
					);
				if (!this.connected(run.task.scope))
					throw new ExecutionDetachedError(receipt.run_id);
				replay = await followRealtimeExecution({
					createClient: this.options.realtime,
					session: {
						...realtime,
						headers: run.request.headers ?? realtime.headers,
					},
					receipt,
					signal: run.controller.signal,
					onUnauthorized: async (rejectedToken) => {
						for (const [candidate, scope] of this.ports) {
							if (scope !== run.task.scope) continue;
							const headers = await this.authorize(
								candidate,
								run,
								run.request.headers,
								rejectedToken,
							);
							if (headers) {
								run.request.headers = headers;
								return headers;
							}
						}
					},
					onRun: (update) => {
						for (const event of update.events)
							observe(realtimeExecutionEvent(update.run_id, event));
					},
				});
				terminal = true;
				if (
					!run.task.events.some((event) =>
						["completed", "execution_complete"].includes(event.event_type),
					)
				)
					observe({
						event_id: `${receipt.run_id}:terminal`,
						event_type: "completed",
						payload: { status: replay.status, error_message: replay.error },
						timestamp: {
							secs_since_epoch: Math.floor(Date.now() / 1_000),
							nanos_since_epoch: 0,
						},
					});
			} else {
				const reader = response.body.getReader();
				const decoder = new TextDecoder();
				let buffer = "";
				const frame = (raw: string) => {
					let name = "message";
					const data: string[] = [];
					for (const line of raw.split(/\r?\n/)) {
						if (line.startsWith("event:")) name = line.slice(6).trim();
						if (line.startsWith("data:"))
							data.push(line.slice(5).replace(/^ /, ""));
					}
					if (data.length === 0 || data.join("\n") === "keep-alive") return;
					let event: IIntercomEvent;
					try {
						event = JSON.parse(data.join("\n"));
					} catch {
						throw new Error("The execution stream contained an invalid event.");
					}
					if (!event || typeof event.event_type !== "string")
						throw new Error("The execution stream contained an invalid event.");
					if (!event.event_id)
						event.event_id = `${run.task.taskId}:${run.task.events.length}`;
					observe(event);
					if (
						name === "done" ||
						name === "completed" ||
						event.event_type === "completed" ||
						event.event_type === "execution_complete"
					)
						terminal = true;
				};
				const drain = () => {
					while (true) {
						const separator = /\r?\n\r?\n/.exec(buffer);
						if (!separator) break;
						const raw = buffer.slice(0, separator.index);
						buffer = buffer.slice(separator.index + separator[0].length);
						frame(raw);
					}
				};
				try {
					while (true) {
						const { value, done } = await reader.read();
						buffer += done
							? decoder.decode()
							: decoder.decode(value, { stream: true });
						drain();
						if (done || terminal) {
							if (buffer.trim()) frame(buffer);
							break;
						}
					}
				} finally {
					await reader.cancel().catch(() => undefined);
					reader.releaseLock();
				}
			}
			if (run.cancelConfirmed) throw abortError();
			if (!terminal)
				throw new Error(
					lastError ?? "The execution stream ended before the run completed.",
				);
			const completed = [...run.task.events]
				.reverse()
				.find(
					(event) =>
						event.event_type === "completed" ||
						event.event_type === "execution_complete",
				);
			const status = String(
				replay?.status ?? completed?.payload?.status ?? "",
			).toLowerCase();
			if (status === "cancelled" || status === "canceled") throw abortError();
			const succeeded = ["completed", "success", "succeeded"].includes(status);
			const timedOut = status === "timeout" || status === "timed_out";
			if (
				!succeeded &&
				(status === "failed" ||
					timedOut ||
					completed?.payload?.error_message ||
					hasError ||
					(completed?.payload?.log_level ?? 0) >= 3)
			) {
				throw new Error(
					replay?.error?.trim() ||
						messageText(completed?.payload) ||
						lastError ||
						(timedOut ? "The workflow timed out." : "The workflow failed."),
				);
			}
			run.task.status = "completed";
			run.task.error = undefined;
		} catch (error) {
			const detached = error instanceof ExecutionDetachedError;
			// Nothing replays a detached run's end, so a stop the server already accepted is its outcome.
			const stopped = run.cancelConfirmed || (detached && run.stopAccepted);
			run.failure = executionError(stopped ? abortError() : error);
			if (detached && !stopped) run.task.detached = true;
			else {
				run.task.status =
					run.failure.name === "AbortError" ? "cancelled" : "failed";
				run.task.error = run.failure.message;
			}
		} finally {
			clearTimeout(deadline);
			// A detached run keeps waiting on the server: its questions stay answerable until they expire,
			// and its reply stays incomplete until a stop saves it again.
			if (!run.task.detached) this.conclude(run);
			try {
				await this.save(run, !run.task.detached);
			} catch (error) {
				if (!run.task.detached) {
					run.failure = executionError(error);
					run.task.status = "failed";
					run.task.error = `The reply could not be saved: ${run.failure.message}`;
				}
			}
			run.finished = true;
			run.endedAt = Date.now();
			this.broadcast(run);
			// Prepared requests contain OAuth and account tokens. Finished tasks keep display data,
			// and a detached run keeps only what stopping it needs.
			run.request = run.task.detached
				? {
						url: "",
						body: null,
						cancelUrl: run.request.cancelUrl,
						credentials: run.request.credentials,
					}
				: { url: "", body: null };
			this.retain(run);
		}
		if (run.failure)
			throw Object.assign(new Error(run.failure.message), run.failure);
		return run.task.result;
	}

	/** Terminal state: nothing can answer the run's open questions any more. */
	private conclude(run: Run) {
		clearTimeout(run.expiryTimer);
		run.task.completedAt = Date.now();
		for (const request of run.task.pendingInteractions)
			if (request.status === "pending") request.status = "cancelled";
	}

	private acceptEvent(run: Run, event: IIntercomEvent) {
		if (run.seen.has(event.event_id)) return;
		run.seen.add(event.event_id);
		const offset = run.task.events.length;
		run.task.events.push(event);
		for (const handle of executionChannels(event.payload)) {
			const key = channelRequestKey(handle);
			run.channels.set(key, handle);
			if (handle.request_id && !run.replies.has(key))
				run.pendingRequests.add(key);
		}
		if (
			event.event_type === "run_initiated" &&
			typeof event.payload?.run_id === "string"
		) {
			run.task.runId = event.payload.run_id;
		}
		for (const channel of executionChannels(event.payload)) {
			const requestId = channel.request_id;
			if (
				typeof requestId === "string" &&
				!run.task.pendingInteractions.some(
					(request) => request.requestId === requestId,
				)
			) {
				run.task.pendingInteractions.push({
					requestId,
					event,
					channel,
					status: "pending",
				});
			}
		}
		this.scheduleExpiry(run);
		if (event.event_type === "chat_out" || event.event_type === "chat_stream") {
			const response = event.payload?.response;
			const content = response?.choices?.[0]?.message?.content;
			run.task.preview =
				(typeof content === "string" ? content : messageText(response))?.slice(
					-180,
				) ?? run.task.preview;
		}
		const delta = event.payload?.chunk?.choices?.[0]?.delta?.content;
		if (typeof delta === "string")
			run.task.preview = `${run.task.preview ?? ""}${delta}`.slice(-180);
		if (!isExecutionFinished(run.task))
			run.task.status = run.pendingRequests.size > 0 ? "waiting" : "running";
		this.broadcast(run, offset);
		if (event.event_type === "run_initiated" && run.task.cancellationRequested)
			void this.cancel(run).catch(() => undefined);
		if (
			event.event_type === "chat_reaction" ||
			Date.now() - run.lastSave >= 1_000
		) {
			run.lastSave = Date.now();
			void this.save(run, false).catch(() => undefined);
		}
	}

	private scheduleExpiry(run: Run) {
		clearTimeout(run.expiryTimer);
		let next = Number.POSITIVE_INFINITY;
		for (const request of run.task.pendingInteractions) {
			if (request.status !== "pending" || !request.channel) continue;
			const expiresAt = request.channel.expires_at * 1_000;
			if (expiresAt <= Date.now()) {
				request.status = "expired";
				run.pendingRequests.delete(channelRequestKey(request.channel));
			} else next = Math.min(next, expiresAt);
		}
		if (Number.isFinite(next)) {
			run.expiryTimer = setTimeout(
				() => {
					this.scheduleExpiry(run);
					if (!isExecutionFinished(run.task))
						run.task.status = run.pendingRequests.size ? "waiting" : "running";
					this.broadcast(run);
				},
				Math.min(next - Date.now(), 2_147_483_647),
			);
		}
	}

	private publishScope(scope: string) {
		const tasks = [...this.runs.values()]
			.filter((run) => run.task.scope === scope)
			.map((run) => snapshot(run.task));
		for (const [port, owner] of this.ports)
			if (owner === scope) this.send(port, { type: "snapshot", tasks });
	}

	private drop(run: Run) {
		clearTimeout(run.expiryTimer);
		this.runs.delete(this.key(run.task.scope, run.task.taskId));
		// A detached run nobody stopped gets no other last save to release its reply.
		if (run.task.detached && !isExecutionFinished(run.task))
			void this.save(run, true).catch(() => undefined);
	}

	/**
	 * Finished runs stay listed so a tab that attaches later (reload, new tab, worker fallback)
	 * still learns their outcome. Its events only matter to a tab that has not seen the finish:
	 * chat replies are saved before it is published. Once a connected tab received it, the
	 * summary is enough; undelivered events share a byte budget, oldest released first.
	 */
	private retain(run: Run) {
		if (this.connected(run.task.scope)) this.release(run);
		else run.retainedBytes = JSON.stringify(run.task.events).length;
		const scope = run.task.scope;
		const finished = [...this.runs.values()]
			.filter((item) => item.finished && item.task.scope === scope)
			.sort((a, b) => (b.endedAt ?? 0) - (a.endedAt ?? 0));
		for (const item of finished.slice(RETAINED_FINISHED_RUNS)) this.drop(item);
		let bytes = 0;
		for (const item of [...this.runs.values()]
			.filter((candidate) => candidate.retainedBytes)
			.sort((a, b) => (b.endedAt ?? 0) - (a.endedAt ?? 0))) {
			bytes += item.retainedBytes;
			if (bytes > RETAINED_UNDELIVERED_EVENT_BYTES) this.release(item);
		}
		if (finished.length > RETAINED_FINISHED_RUNS) this.publishScope(scope);
	}

	private release(run: Run) {
		run.task.events = [];
		run.seen.clear();
		run.retainedBytes = 0;
	}

	private async cancel(run: Run) {
		if (run.cancellation) return run.cancellation;
		if (run.finished && !run.task.detached) return;
		if (isExecutionFinished(run.task)) return;
		run.task.cancellationRequested = true;
		this.broadcast(run);
		// Keep the dispatch alive until its run id arrives so cancellation can reach the server.
		if (!run.task.runId) return;
		run.cancellation = this.cancelRemote(run).then(
			async () => {
				run.stopAccepted = true;
				// Nothing replays a detached run's end; the accepted stop is its outcome here.
				if (!run.task.detached) return;
				run.task.status = "cancelled";
				run.task.error = undefined;
				this.conclude(run);
				await this.save(run, true).catch(() => undefined);
				this.broadcast(run);
				this.retain(run);
			},
			(error) => {
				run.cancellation = undefined;
				run.task.cancellationRequested = false;
				if (!run.finished || run.task.detached)
					run.task.error = `Stop could not be confirmed: ${executionError(error).message}`;
				this.broadcast(run);
				throw error;
			},
		);
		return run.cancellation;
	}

	private async cancelRemote(run: Run) {
		const request = {
			...run.request,
			headers: run.cancelHeaders ?? run.request.headers,
		};
		const runId = run.task.runId;
		if (!runId) return;
		const operations: Promise<unknown>[] = [];
		if (request.cancelUrl) {
			const url = request.cancelUrl.replaceAll(
				"{runId}",
				encodeURIComponent(runId),
			);
			operations.push(
				(async () => {
					const send = () =>
						this.fetcher(url, {
							method: "DELETE",
							headers: request.headers,
							credentials: request.credentials,
							signal: AbortSignal.timeout(15_000),
						});
					let response = await send();
					if (response.status === 401 && run.cancelPort) {
						const headers = await this.authorize(
							run.cancelPort,
							run,
							request.headers,
						);
						if (headers) {
							request.headers = headers;
							response = await send();
						}
					}
					if (!response.ok)
						throw Object.assign(
							new Error(`Cancellation failed (${response.status})`),
							{
								status: response.status,
								statusText: response.statusText,
								headers: Object.fromEntries(response.headers.entries()),
								body: await response.text(),
								url,
							},
						);
					const body = await response.text();
					let acknowledgement:
						| { cancelled?: boolean; status?: string }
						| undefined;
					try {
						acknowledgement = body ? JSON.parse(body) : undefined;
					} catch {
						/* Legacy successful stop endpoints return an empty body. */
					}
					// Cloud cancellation first records a request. Keep listening until its terminal
					// event confirms the outcome; a cooperative channel push has the same boundary.
					if (
						acknowledgement?.cancelled === false ||
						acknowledgement?.status === "cancellation_requested"
					)
						return;
					run.cancelConfirmed = true;
					run.task.error = undefined;
					run.controller.abort(abortError());
				})(),
			);
		}
		const channels = new Map(
			[...run.channels.values()].map((handle) => [handle.channel_id, handle]),
		);
		for (const handle of channels.values())
			operations.push(
				this.boundedPush(
					this.options.push(handle, { kind: "cancel", value: null }),
				),
			);
		if (operations.length === 0)
			throw new Error("This run has no available cancellation endpoint");
		try {
			await Promise.any(operations);
		} catch (error) {
			throw error instanceof AggregateError ? error.errors[0] : error;
		}
	}

	private async boundedPush(operation: Promise<void>): Promise<void> {
		let timer: ReturnType<typeof setTimeout> | undefined;
		try {
			await Promise.race([
				operation,
				new Promise<never>((_, reject) => {
					timer = setTimeout(
						() => reject(new Error("Channel cancellation timed out")),
						15_000,
					);
				}),
			]);
		} finally {
			clearTimeout(timer);
		}
	}

	private reply(
		run: Run,
		handle: IChannelHandle,
		value: unknown,
	): Promise<void> {
		const key = channelRequestKey(handle);
		const previous = run.replies.get(key);
		if (previous) return previous;
		if (isExecutionFinished(run.task))
			return Promise.reject(new Error("The execution has already finished"));
		const request = run.task.pendingInteractions.find(
			(candidate) => candidate.requestId === handle.request_id,
		);
		if (handle.expires_at * 1_000 <= Date.now()) {
			if (request) request.status = "expired";
			this.broadcast(run);
			return Promise.reject(new Error("This request has expired"));
		}
		if (request) {
			request.status = "submitting";
			request.error = undefined;
		}
		this.broadcast(run);
		const promise = Promise.resolve()
			.then(() =>
				this.options.push(handle, {
					kind: "reply",
					request_id: handle.request_id,
					value,
				}),
			)
			.then(
				() => {
					run.pendingRequests.delete(key);
					if (request) {
						request.status = "submitted";
						request.response_value = value;
					}
					if (!isExecutionFinished(run.task))
						run.task.status =
							run.pendingRequests.size > 0 ? "waiting" : "running";
					this.broadcast(run);
				},
				(error) => {
					// A failed delivery is retryable; successful request ids stay claimed for the run's lifetime.
					run.replies.delete(key);
					if (request) {
						request.status = "pending";
						request.error = executionError(error).message;
					}
					this.broadcast(run);
					throw error;
				},
			);
		run.replies.set(key, promise);
		return promise;
	}
}
