import { isChannelHandle } from "../channel/handle";
import type { IChannelHandle, IChannelPush } from "../schema/channel";
import type { IIntercomEvent } from "../schema/events/intercom-event";
import {
	type ExecutionAuthorization,
	type ExecutionCommand,
	type ExecutionErrorData,
	type ExecutionMetadata,
	type ExecutionPort,
	type ExecutionTask,
	type PreparedExecutionRequest,
	executionError,
	isExecutionFinished,
} from "./types";

interface RuntimeOptions {
	fetch?: (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;
	persist?: (task: ExecutionTask, final: boolean) => Promise<void>;
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
		this.fetcher = options.fetch ?? ((...args) => globalThis.fetch(...args));
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

	private disconnect(port: ExecutionPort) {
		this.ports.delete(port);
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
				this.send(port, {
					type: "snapshot",
					tasks: [...this.runs.values()]
						.filter((run) => run.task.scope === command.scope)
						.map((run) => snapshot(run.task)),
				});
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
				if (!run || !run.seen.has(command.eventId))
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
					this.runs.delete(this.key(scope, run.task.taskId));
					for (const [target, owner] of this.ports)
						if (owner === scope)
							this.send(target, {
								type: "snapshot",
								tasks: [...this.runs.values()]
									.filter((item) => item.task.scope === scope)
									.map((item) => snapshot(item.task)),
							});
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
						run.finished ||
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
					rejectedToken: auth?.replace(/^Bearer\s+/i, ""),
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
		let lastError: string | undefined;
		let hasError = false;
		let deadline: ReturnType<typeof setTimeout> | undefined;
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
				this.acceptEvent(run, event);
				if (event.event_type === "error") {
					hasError = true;
					lastError = messageText(event.payload);
				}
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
			const status = String(completed?.payload?.status ?? "").toLowerCase();
			if (status === "cancelled" || status === "canceled") throw abortError();
			const succeeded = ["completed", "success", "succeeded"].includes(status);
			if (
				!succeeded &&
				(["failed", "timeout", "timed_out"].includes(status) ||
					completed?.payload?.error_message ||
					hasError ||
					(completed?.payload?.log_level ?? 0) >= 3)
			) {
				throw new Error(
					messageText(completed?.payload) ??
						lastError ??
						"The workflow failed.",
				);
			}
			run.task.status = "completed";
			run.task.error = undefined;
		} catch (error) {
			run.failure = executionError(run.cancelConfirmed ? abortError() : error);
			run.task.status =
				run.failure.name === "AbortError" ? "cancelled" : "failed";
			run.task.error = run.failure.message;
		} finally {
			clearTimeout(deadline);
			clearTimeout(run.expiryTimer);
			run.task.completedAt = Date.now();
			for (const request of run.task.pendingInteractions)
				if (request.status === "pending") request.status = "cancelled";
			try {
				await this.save(run, true);
			} catch (error) {
				run.failure = executionError(error);
				run.task.status = "failed";
				run.task.error = `The reply could not be saved: ${run.failure.message}`;
			}
			run.finished = true;
			this.broadcast(run);
			// Prepared requests contain OAuth and account tokens. Terminal tasks only retain display data.
			run.request = { url: "", body: null };
			this.pruneFinished(run.task.scope);
		}
		if (run.failure)
			throw Object.assign(new Error(run.failure.message), run.failure);
		return run.task.result;
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

	private pruneFinished(scope: string) {
		const finished = [...this.runs.values()]
			.filter((run) => run.finished && run.task.scope === scope)
			.sort((a, b) => (b.task.completedAt ?? 0) - (a.task.completedAt ?? 0));
		if (finished.length <= 50) return;
		for (const run of finished.slice(50))
			this.runs.delete(this.key(scope, run.task.taskId));
		for (const [port, owner] of this.ports)
			if (owner === scope)
				this.send(port, {
					type: "snapshot",
					tasks: [...this.runs.values()]
						.filter((run) => run.task.scope === scope)
						.map((run) => snapshot(run.task)),
				});
	}

	private async cancel(run: Run) {
		if (run.cancellation) return run.cancellation;
		if (run.finished) return;
		run.task.cancellationRequested = true;
		this.broadcast(run);
		// Keep the dispatch alive until its run id arrives so cancellation can reach the server.
		if (!run.task.runId) return;
		run.cancellation = this.cancelRemote(run).catch((error) => {
			run.cancellation = undefined;
			run.task.cancellationRequested = false;
			if (!run.finished)
				run.task.error = `Stop could not be confirmed: ${executionError(error).message}`;
			this.broadcast(run);
			throw error;
		});
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
		if (run.finished)
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
