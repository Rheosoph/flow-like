import type { IBackendState } from "../state/backend-state";
import type { ExecutionChatSeed } from "./execution/chat-persistence";
import { registerExecutionTransportOwner } from "./execution/owners";
import type { ExecutionRuntimeClient } from "./execution/runtime-client";
import {
	EXECUTION_METADATA,
	type ExecutionTask,
	isExecutionFinished,
} from "./execution/types";
import type { ILogMetadata, PageTrigger } from "./schema";
import type { IRunPayload } from "./schema";
import type { IIntercomEvent } from "./schema/events/intercom-event";
import { captureTelemetryError } from "./telemetry/errors";

/** `failure` describes a rejected request or a failed run; absent on success. */
type OnCompleteFn = (
	events: IIntercomEvent[],
	failure?: unknown,
) => void | Promise<void>;

interface ISubscriber {
	onEvents: (events: IIntercomEvent[]) => void;
	onComplete?: OnCompleteFn;
}

export interface ActiveEventExecution {
	scope?: string;
	streamId: string;
	appId: string;
	eventId: string;
	runId?: string;
	title: string;
	startedAt: string;
}

interface IEventStream {
	taskId?: string;
	chat?: ExecutionChatSeed;
	persistenceManaged?: boolean;
	initiatedHere?: boolean;
	settlementPromise?: Promise<void>;
	saveQueue?: Promise<void>;
	onLiveEvents?: (events: IIntercomEvent[]) => void;
	subscribers: Map<string, ISubscriber>;
	accumulatedEvents: IIntercomEvent[];
	lastSentIndex: Map<string, number>;
	seenEventIds: Set<string>;
	executionPromise?: Promise<unknown>;
	isComplete: boolean;
	/** The run ended, its final save landed and its watchers finished their completion. */
	settled?: boolean;
	failure?: unknown;
	completedAt?: number;
	completionObserved?: boolean;
	finalizingUnobserved?: boolean;
	activeExecution?: Omit<ActiveEventExecution, "streamId">;
	owner?: IBackendState;
	path?: string;
	title?: string;
	interfaceType?: string;
}

/**
 * Callback function for handling incremental saves during streaming.
 * Called periodically with accumulated events to allow saving intermediate state.
 * @param events - All accumulated events up to this point
 * @param isFinal - Whether this is the final save (stream completed)
 */
export type OnIncrementalSaveFn = (
	events: IIntercomEvent[],
	isFinal: boolean,
) => void | Promise<void>;

interface IExecuteEventOptions {
	chat?: ExecutionChatSeed;
	appId: string;
	eventId: string;
	payload: IRunPayload;
	streamState?: boolean;
	onExecutionStart?: (executionId: string) => void;
	/** Receives each new event once, even with no subscribers. Excludes replay. */
	onLiveEvents?: (events: IIntercomEvent[]) => void;
	/** Finalize detached chat replies when no mounted subscriber handles completion. */
	onUnobservedComplete?: OnCompleteFn;
	path?: string;
	title?: string;
	interfaceType?: string;
	skipConsentCheck?: boolean;
	pageTrigger?: PageTrigger;
	/** Revalidate the initiating request after asynchronous setup and immediately before dispatch. */
	beforeDispatch?: () => void;
	/**
	 * Optional callback for incremental saves during streaming.
	 * Called every `saveIntervalEvents` events and on completion.
	 * Useful for persisting chat messages to Dexie during long streams.
	 */
	onIncrementalSave?: OnIncrementalSaveFn;
	/**
	 * Number of events between incremental saves. Defaults to 10.
	 * Only used if onIncrementalSave is provided.
	 */
	saveIntervalEvents?: number;
	/**
	 * Minimum milliseconds between incremental saves. Defaults to 1000.
	 * Every Dexie write re-materializes the whole liveQuery result, so a fast
	 * token stream must not save on event count alone. Completion always saves
	 * regardless.
	 */
	saveMinIntervalMs?: number;
}

/**
 * A finished stream nobody listens to waits for its chat to come back and replay it. Beyond this
 * many the oldest are dropped; their chats then show the saved draft, as after a reload.
 */
const MAX_UNCLAIMED_STREAMS = 8;

/** A completion handler that never returns must not keep its session busy forever. */
const COMPLETION_GRACE_MS = 15_000;

export type ExecuteEventFn = (
	appId: string,
	eventId: string,
	payload: IRunPayload,
	streamState?: boolean,
	onEventId?: (id: string) => void,
	cb?: (event: IIntercomEvent[]) => void,
	skipConsentCheck?: boolean,
	pageTrigger?: PageTrigger,
	beforeDispatch?: () => void,
) => Promise<ILogMetadata | undefined>;

function failureText(payload: unknown): string | undefined {
	if (typeof payload === "string") return payload.trim() || undefined;
	if (!payload || typeof payload !== "object") return undefined;
	const record = payload as Record<string, unknown>;
	for (const key of ["error_message", "message", "error"]) {
		const value = record[key];
		if (typeof value === "string" && value.trim()) return value.trim();
	}
	return undefined;
}

/** Transports can resolve even when the workflow failed. Read its terminal outcome as well. */
function completedRunFailure(
	events: IIntercomEvent[],
	metadata: ILogMetadata | undefined,
): Error | undefined {
	let completed: Record<string, unknown> | undefined;
	let hasError = false;
	let errorMessage: string | undefined;
	for (const event of events) {
		if (event.event_type === "error") {
			hasError = true;
			errorMessage ??= failureText(event.payload);
		} else if (
			event.event_type === "completed" &&
			event.payload &&
			typeof event.payload === "object"
		) {
			completed = event.payload;
		}
	}
	const status =
		typeof completed?.status === "string"
			? completed.status.trim().toLowerCase()
			: undefined;
	// A handled node error can leave error logs on a successful run.
	if (status === "completed" || status === "success" || status === "succeeded")
		return undefined;
	if (status === "cancelled" || status === "canceled") {
		return Object.assign(new Error("Execution cancelled"), {
			name: "AbortError",
		});
	}
	const reason =
		metadata?.error_message?.trim() || failureText(completed) || errorMessage;
	const timedOut = status === "timeout" || status === "timed_out";
	if (
		status === "failed" ||
		timedOut ||
		hasError ||
		reason ||
		(metadata?.log_level ?? 0) >= 3 ||
		(typeof completed?.log_level === "number" && completed.log_level >= 3)
	) {
		return new Error(
			reason ||
				(timedOut
					? "The workflow timed out before it could finish this response."
					: "The workflow failed before it could finish this response. Check the run logs for details."),
		);
	}
	return undefined;
}

export class ExecutionEngineProvider {
	private eventStreams: Map<string, IEventStream> = new Map();
	private backend: IBackendState | null = null;
	private globalListeners: Set<() => void> = new Set();
	private executeEventFn: ExecuteEventFn | null = null;
	private executionScope?: string;
	private runtimeClient?: ExecutionRuntimeClient;
	private unsubscribeRuntime?: () => void;
	private localTasks = new Map<string, ExecutionTask>();

	setBackend(backend: IBackendState): void {
		this.backend = backend;
		this.notifyGlobalListeners();
	}

	isRuntimeReady(): boolean {
		return (
			!!this.backend &&
			(!this.backend.eventState.alwaysRemote || !!this.runtimeClient)
		);
	}

	setExecutionScope(scope: string): void {
		if (this.executionScope !== undefined && this.executionScope !== scope) {
			this.unsubscribeRuntime?.();
			this.runtimeClient = undefined;
			this.eventStreams.clear();
		}
		this.executionScope = scope;
		this.notifyGlobalListeners();
	}

	/** Restore remote executions and their ordered event history from the session worker. */
	setRuntimeClient(client: ExecutionRuntimeClient | undefined): void {
		this.unsubscribeRuntime?.();
		this.runtimeClient = client;
		if (!client) {
			this.notifyGlobalListeners();
			return;
		}
		const update = () => {
			if (client.scope !== (this.executionScope ?? "")) return;
			const latest = new Map<string, ExecutionTask>();
			for (const task of client.getTasks()) {
				this.localTasks.delete(task.taskId);
				const previous = latest.get(task.streamId);
				if (!previous || previous.startedAt <= task.startedAt)
					latest.set(task.streamId, task);
			}
			for (const task of latest.values()) {
				let stream = this.eventStreams.get(task.streamId);
				if (stream?.taskId && stream.taskId !== task.taskId && !stream.settled)
					continue;
				if (!stream || (stream.taskId && stream.taskId !== task.taskId)) {
					const subscribers = stream?.subscribers ?? new Map();
					stream = {
						subscribers,
						accumulatedEvents: [],
						lastSentIndex: new Map(),
						seenEventIds: new Set(),
						isComplete: false,
					};
					this.eventStreams.set(task.streamId, stream);
				}
				stream.taskId = task.taskId;
				if (!isExecutionFinished(task) && stream.settled) {
					stream.isComplete = false;
					stream.settled = false;
					stream.settlementPromise = undefined;
					stream.failure = undefined;
					stream.initiatedHere = false;
				}
				stream.chat = task.chat;
				stream.persistenceManaged = true;
				stream.owner = this.backend ?? undefined;
				stream.path = task.path;
				stream.title = task.title;
				stream.interfaceType = task.interfaceType;
				stream.activeExecution = {
					scope: task.scope,
					appId: task.appId,
					eventId: task.eventId,
					title: task.title,
					startedAt: task.startedAt,
					runId: task.runId,
				};
				stream.executionPromise ??= Promise.resolve();
				this.publishEvents(stream, task.events);
				if (
					isExecutionFinished(task) &&
					!stream.initiatedHere &&
					!stream.settlementPromise
				) {
					stream.failure = task.error
						? Object.assign(new Error(task.error), {
								name: task.status === "cancelled" ? "AbortError" : "Error",
							})
						: undefined;
					void this.settleRun(stream, undefined, Boolean(task.error));
				}
			}
			this.notifyGlobalListeners();
		};
		this.unsubscribeRuntime = client.subscribe(update);
		update();
	}

	isPersistenceManaged(streamId: string): boolean {
		return this.eventStreams.get(streamId)?.persistenceManaged ?? false;
	}
	getChatSeed(streamId: string): ExecutionChatSeed | undefined {
		return this.eventStreams.get(streamId)?.chat;
	}
	getTasks(): ExecutionTask[] {
		return [
			...(this.runtimeClient?.getTasks() ?? []),
			...this.localTasks.values(),
		]
			.filter((task) => task.scope === (this.executionScope ?? ""))
			.sort((a, b) => b.startedAt.localeCompare(a.startedAt));
	}
	async cancelTask(taskId: string): Promise<void> {
		const remote = this.runtimeClient?.getTask(taskId);
		if (remote) {
			if (remote.runId && this.backend)
				return this.backend.eventState.cancelExecution(remote.runId);
			return this.runtimeClient?.cancel(taskId);
		}
		const task = this.localTasks.get(taskId);
		if (
			!task ||
			task.scope !== (this.executionScope ?? "") ||
			isExecutionFinished(task)
		)
			return;
		task.cancellationRequested = true;
		this.notifyGlobalListeners();
		if (task.runId) {
			try {
				await this.backend?.eventState.cancelExecution(task.runId);
			} catch (error) {
				task.cancellationRequested = false;
				task.error = error instanceof Error ? error.message : String(error);
				this.notifyGlobalListeners();
				throw error;
			}
		}
	}
	dismissTask(taskId: string): void {
		if (this.runtimeClient?.getTask(taskId)) {
			void this.runtimeClient
				.dismiss(taskId)
				.catch((error) => console.error("Could not dismiss execution", error));
			return;
		}
		const task = this.localTasks.get(taskId);
		if (task && isExecutionFinished(task)) {
			this.localTasks.delete(taskId);
			this.notifyGlobalListeners();
		}
	}
	async replyToTask(
		taskId: string,
		requestId: string,
		value: unknown,
	): Promise<void> {
		const task = this.getTasks().find(
			(candidate) => candidate.taskId === taskId,
		);
		const request = task?.pendingInteractions.find(
			(candidate) => candidate.requestId === requestId,
		);
		if (!request?.channel)
			throw new Error("The request does not have a reply channel");
		if (this.runtimeClient?.getTask(taskId))
			return this.runtimeClient.submitReply(request.channel, value);
		const { replyToChannel } = await import("./channel");
		await replyToChannel(request.channel, value);
		request.status = "submitted";
		request.response_value = value;
		this.notifyGlobalListeners();
	}

	private publishEvents(
		stream: IEventStream,
		events: IIntercomEvent[],
	): IIntercomEvent[] {
		const unique = events.filter((event) => {
			if (stream.seenEventIds.has(event.event_id)) return false;
			stream.seenEventIds.add(event.event_id);
			return true;
		});
		if (!unique.length) return unique;
		stream.accumulatedEvents.push(...unique);
		if (stream.onLiveEvents)
			void this.deliver("live-events", () => stream.onLiveEvents?.(unique));
		for (const [subscriberId, subscriber] of stream.subscribers) {
			const start = stream.lastSentIndex.get(subscriberId) ?? 0;
			const pending = stream.accumulatedEvents.slice(start);
			stream.lastSentIndex.set(subscriberId, stream.accumulatedEvents.length);
			if (pending.length)
				void this.deliver(subscriberId, () => subscriber.onEvents(pending));
		}
		return unique;
	}

	setExecuteEventFn(fn: ExecuteEventFn): void {
		this.executeEventFn = fn;
	}

	subscribeToGlobalUpdates(listener: () => void): () => void {
		this.globalListeners.add(listener);
		return () => {
			this.globalListeners.delete(listener);
		};
	}

	private notifyGlobalListeners() {
		for (const listener of this.globalListeners) {
			try {
				listener();
			} catch (error) {
				console.error("Execution listener failed", error);
			}
		}
	}

	// Subscribers run inside the transport's read loop: one that throws must not end the run for
	// everyone else, nor surface as an unhandled rejection from an async completion handler.
	private deliver(subscriberId: string, run: () => unknown): Promise<void> {
		const report = (error: unknown) => {
			console.error(
				`[ExecutionEngine] Subscriber ${subscriberId} failed:`,
				error,
			);
			captureTelemetryError(error, {
				culprit: "execution-engine.subscriber",
				context: { subscriberId },
			});
		};
		try {
			return Promise.resolve(run()).then(() => undefined, report);
		} catch (error) {
			report(error);
			return Promise.resolve();
		}
	}

	/**
	 * Ends a run: the final save, then completion for those who watched it end. Until their handlers
	 * finished the stream stays active, so the session's next run can neither reset the events under
	 * them nor feed them its own.
	 */
	private settleRun(
		stream: IEventStream,
		onFinalSave: OnIncrementalSaveFn | undefined,
		failed: boolean,
		onUnobservedComplete?: OnCompleteFn,
	): Promise<void> {
		if (stream.settlementPromise) return stream.settlementPromise;
		stream.settlementPromise = this.finishRun(
			stream,
			onFinalSave,
			failed,
			onUnobservedComplete,
		);
		return stream.settlementPromise;
	}

	private async bounded(work: Promise<unknown>): Promise<void> {
		let timer: ReturnType<typeof setTimeout> | undefined;
		try {
			await Promise.race([
				work,
				new Promise<void>((_, reject) => {
					timer = setTimeout(
						() => reject(new Error("Execution completion timed out")),
						COMPLETION_GRACE_MS,
					);
				}),
			]);
		} finally {
			clearTimeout(timer);
		}
	}

	private async finishRun(
		stream: IEventStream,
		onFinalSave: OnIncrementalSaveFn | undefined,
		failed: boolean,
		onUnobservedComplete?: OnCompleteFn,
	) {
		const events = stream.accumulatedEvents;
		stream.completedAt = Date.now();
		if (
			!stream.persistenceManaged &&
			onFinalSave &&
			(!failed || events.length > 0)
		) {
			try {
				await this.bounded(
					(stream.saveQueue ?? Promise.resolve()).then(() =>
						onFinalSave([...events], true),
					),
				);
			} catch (error) {
				console.error("[ExecutionEngine] Final save error:", error);
			}
		}
		stream.isComplete = true;
		const watchers = [...stream.subscribers];
		const completions = watchers.map(([subscriberId, subscriber]) => {
			const onComplete = subscriber.onComplete;
			// One that left meanwhile settles itself when it returns; one that joined got it on joining.
			if (!onComplete || stream.subscribers.get(subscriberId) !== subscriber)
				return undefined;
			stream.completionObserved = true;
			return this.deliver(subscriberId, () =>
				onComplete(events, stream.failure),
			);
		});
		let grace: ReturnType<typeof setTimeout> | undefined;
		await Promise.race([
			Promise.all(completions),
			new Promise((resolve) => {
				grace = setTimeout(resolve, COMPLETION_GRACE_MS);
			}),
		]);
		clearTimeout(grace);
		if (
			!stream.persistenceManaged &&
			!stream.completionObserved &&
			onUnobservedComplete
		) {
			stream.finalizingUnobserved = true;
			let finalized = false;
			await this.deliver("unobserved-completion", async () => {
				await this.bounded(
					Promise.resolve().then(() =>
						onUnobservedComplete(events, stream.failure),
					),
				);
				finalized = true;
			});
			stream.finalizingUnobserved = false;
			// The detached reply is durable now. Replaying it on return would assign a new
			// message id after its checkpoint was removed and duplicate the saved answer.
			if (finalized) {
				for (const [streamId, candidate] of this.eventStreams) {
					if (candidate === stream) this.eventStreams.delete(streamId);
				}
			} else {
				for (const [streamId, candidate] of this.eventStreams) {
					if (candidate !== stream) continue;
					for (const [subscriberId, subscriber] of stream.subscribers) {
						this.subscribeToEventStream(
							streamId,
							subscriberId,
							subscriber.onEvents,
							subscriber.onComplete,
						);
					}
				}
			}
		}
		stream.settled = true;
		if (stream.completionObserved && stream.subscribers.size === 0) {
			for (const [streamId, candidate] of this.eventStreams)
				if (candidate === stream) this.eventStreams.delete(streamId);
		}
		const task = stream.taskId ? this.localTasks.get(stream.taskId) : undefined;
		if (task) {
			task.status = stream.failure
				? (stream.failure as Error)?.name === "AbortError"
					? "cancelled"
					: "failed"
				: "completed";
			task.error =
				stream.failure instanceof Error
					? stream.failure.message
					: stream.failure
						? String(stream.failure)
						: undefined;
			task.completedAt = stream.completedAt;
		}
		this.dropUnclaimedStreams();
		this.notifyGlobalListeners();
	}

	private dropUnclaimedStreams() {
		const unclaimed = [...this.eventStreams]
			.filter(
				([, candidate]) =>
					candidate.settled && candidate.subscribers.size === 0,
			)
			.sort(([, a], [, b]) => (a.completedAt ?? 0) - (b.completedAt ?? 0));
		for (const [streamId] of unclaimed.slice(0, -MAX_UNCLAIMED_STREAMS)) {
			this.eventStreams.delete(streamId);
		}
	}

	/** The stream a run writes into: new, or reset once the previous run on it settled. */
	private streamForRun(
		streamId: string,
		options: IExecuteEventOptions,
	): IEventStream {
		const existing = this.eventStreams.get(streamId);
		if (!existing) {
			const stream: IEventStream = {
				subscribers: new Map(),
				accumulatedEvents: [],
				lastSentIndex: new Map(),
				seenEventIds: new Set(),
				isComplete: false,
				path: options.path,
				title: options.title,
				interfaceType: options.interfaceType,
			};
			this.eventStreams.set(streamId, stream);
			return stream;
		}
		if (existing.settled) {
			existing.settlementPromise = undefined;
			existing.persistenceManaged = false;
			existing.saveQueue = undefined;
			existing.isComplete = false;
			existing.settled = false;
			existing.failure = undefined;
			existing.completionObserved = false;
			existing.accumulatedEvents = [];
			existing.lastSentIndex.clear();
			existing.seenEventIds.clear();
			existing.executionPromise = undefined;
			// Subscribers that stayed receive the new run from its first event.
			for (const subscriberId of existing.subscribers.keys()) {
				existing.lastSentIndex.set(subscriberId, 0);
			}
		}
		if (options.path) existing.path = options.path;
		if (options.title) existing.title = options.title;
		if (options.interfaceType) existing.interfaceType = options.interfaceType;
		return existing;
	}

	subscribeToEventStream(
		streamId: string,
		subscriberId: string,
		onEvents: (events: IIntercomEvent[]) => void,
		onComplete?: OnCompleteFn,
	): void {
		let stream = this.eventStreams.get(streamId);
		if (!stream) {
			stream = {
				subscribers: new Map(),
				accumulatedEvents: [],
				lastSentIndex: new Map(),
				seenEventIds: new Set(),
				isComplete: false,
			};
			this.eventStreams.set(streamId, stream);
		}

		// Register subscriber
		stream.subscribers.set(subscriberId, { onEvents, onComplete });
		// A detached finalizer owns these messages. Returning chat surfaces read the saved
		// transcript; replay here would create another response while that save is pending.
		if (stream.finalizingUnobserved) return;

		// Send accumulated events immediately
		const replay = stream.accumulatedEvents;
		stream.lastSentIndex.set(subscriberId, replay.length);
		if (replay.length > 0) this.deliver(subscriberId, () => onEvents(replay));

		// If already complete, notify immediately
		if (stream.isComplete && onComplete) {
			stream.completionObserved = true;
			this.deliver(subscriberId, () => onComplete(replay, stream.failure));
		}

		this.notifyGlobalListeners();
	}

	unsubscribeFromEventStream(streamId: string, subscriberId: string): void {
		const stream = this.eventStreams.get(streamId);
		if (stream) {
			stream.subscribers.delete(subscriberId);
			stream.lastSentIndex.delete(subscriberId);

			// Nobody left and nothing running: a finished run, or one that never started.
			if (
				stream.subscribers.size === 0 &&
				(stream.settled || !stream.executionPromise)
			) {
				this.eventStreams.delete(streamId);
			}
			this.notifyGlobalListeners();
		}
	}

	async executeEvent(
		streamId: string,
		options: IExecuteEventOptions,
	): Promise<ILogMetadata | undefined> {
		options.beforeDispatch?.();
		if (!this.backend) {
			throw new Error("Backend not initialized in ExecutionEngineProvider");
		}

		const stream = this.streamForRun(streamId, options);

		this.notifyGlobalListeners();

		if (stream.executionPromise) {
			// A run that has not settled still owns the stream — reject so callers know the send was dropped
			throw Object.assign(
				new Error("A stream is already active for this session"),
				{ isActiveStreamError: true },
			);
		}

		// Use the executeEventFn if set (handles runtime variables), otherwise fall back to direct call
		const executeEventCall =
			this.executeEventFn ??
			this.backend.eventState.executeEvent.bind(this.backend.eventState);

		stream.owner = this.backend;
		stream.taskId = crypto.randomUUID();
		stream.initiatedHere = true;
		stream.chat = options.chat;
		stream.onLiveEvents = options.onLiveEvents;
		stream.activeExecution = {
			...(this.executionScope ? { scope: this.executionScope } : {}),
			appId: options.appId,
			eventId: options.eventId,
			title: options.title || "Workflow run",
			startedAt: new Date().toISOString(),
		};
		const task: ExecutionTask = {
			...stream.activeExecution,
			scope: this.executionScope ?? "",
			taskId: stream.taskId,
			streamId,
			path: options.path,
			interfaceType: options.interfaceType,
			chat: options.chat,
			status: "starting",
			events: stream.accumulatedEvents,
			pendingInteractions: [],
		};
		this.localTasks.set(task.taskId, task);
		const releaseOwner = registerExecutionTransportOwner(
			task.taskId,
			(client) => {
				if ((this.executionScope ?? "") !== task.scope)
					throw new Error("Execution session changed before dispatch");
				stream.persistenceManaged = true;
				if (this.runtimeClient !== client) this.setRuntimeClient(client);
			},
		);
		const payload = {
			...options.payload,
			[EXECUTION_METADATA]: {
				taskId: task.taskId,
				streamId,
				scope: task.scope,
				appId: task.appId,
				eventId: task.eventId,
				title: task.title,
				startedAt: task.startedAt,
				path: task.path,
				interfaceType: task.interfaceType,
				chat: task.chat,
			},
		};

		// Track event count for incremental saves
		const saveInterval = options.saveIntervalEvents ?? 10;
		const saveMinIntervalMs = options.saveMinIntervalMs ?? 1000;
		let eventsSinceLastSave = 0;
		let lastSaveAt = 0;

		// Reserve ownership before a transport can synchronously emit events or throw.
		stream.executionPromise = Promise.resolve();
		let executionPromise: Promise<ILogMetadata | undefined>;
		try {
			executionPromise = Promise.resolve(
				executeEventCall(
					options.appId,
					options.eventId,
					payload,
					options.streamState ?? false,
					(executionId: string) => {
						if (stream.activeExecution)
							stream.activeExecution.runId = executionId;
						task.runId = executionId;
						task.status = "running";
						if (task.cancellationRequested)
							void stream.owner?.eventState
								.cancelExecution(executionId)
								.catch((error) => {
									task.cancellationRequested = false;
									task.error =
										error instanceof Error ? error.message : String(error);
									this.notifyGlobalListeners();
								});
						this.notifyGlobalListeners();
						void this.deliver("execution-start", () =>
							options.onExecutionStart?.(executionId),
						);
					},
					(events: IIntercomEvent[]) => {
						const unique = this.publishEvents(stream, events);

						if (unique.length > 0) {
							for (const event of unique) {
								if (
									event.event_type === "interaction_request" &&
									event.payload?.channel
								) {
									task.pendingInteractions.push({
										requestId:
											event.payload.channel.request_id ?? event.payload.id,
										event,
										channel: event.payload.channel,
										status: "pending",
									});
									task.status = "waiting";
								}
							}

							// Incremental save logic
							if (options.onIncrementalSave && !stream.persistenceManaged) {
								eventsSinceLastSave += unique.length;
								if (
									eventsSinceLastSave >= saveInterval &&
									Date.now() - lastSaveAt >= saveMinIntervalMs
								) {
									eventsSinceLastSave = 0;
									lastSaveAt = Date.now();
									const snapshot = [...stream.accumulatedEvents];
									stream.saveQueue = (stream.saveQueue ?? Promise.resolve())
										.then(() =>
											this.bounded(
												Promise.resolve().then(() =>
													options.onIncrementalSave?.(snapshot, false),
												),
											),
										)
										.catch((error) =>
											console.error(
												"[ExecutionEngine] Incremental save error:",
												error,
											),
										);
								}
							}

							this.notifyGlobalListeners();
						}
					},
					options.skipConsentCheck,
					options.pageTrigger,
					options.beforeDispatch,
				),
			);
		} catch (error) {
			executionPromise = Promise.reject(error);
		}

		stream.settled = false;
		stream.executionPromise = executionPromise;
		void executionPromise.then(releaseOwner, releaseOwner);
		this.notifyGlobalListeners();

		// Separate handlers: a failure while settling a successful run must not settle it twice.
		executionPromise.then(
			(metadata) => {
				stream.failure ??= completedRunFailure(
					stream.accumulatedEvents,
					metadata,
				);
				return this.settleRun(
					stream,
					options.onIncrementalSave,
					false,
					options.onUnobservedComplete,
				);
			},
			(error) => {
				console.error("Execution error:", error);
				captureTelemetryError(error, {
					culprit: "execution-engine.execute_event",
					context: {
						accumulatedEvents: stream.accumulatedEvents.length,
					},
				});
				stream.failure = error;
				// Still save what arrived, to preserve partial state
				return this.settleRun(
					stream,
					options.onIncrementalSave,
					true,
					options.onUnobservedComplete,
				);
			},
		);

		return executionPromise;
	}

	/** A subscriber alone creates a stream entry; only a run that started and has not settled is active. */
	isStreamActive(streamId: string): boolean {
		const stream = this.eventStreams.get(streamId);
		return !!stream?.executionPromise && !stream.settled;
	}

	hasStream(streamId: string): boolean {
		return this.eventStreams.has(streamId);
	}

	isStreamComplete(streamId: string): boolean {
		const stream = this.eventStreams.get(streamId);
		return stream?.isComplete ?? false;
	}

	getAccumulatedEvents(streamId: string): IIntercomEvent[] {
		return this.eventStreams.get(streamId)?.accumulatedEvents ?? [];
	}

	/** Running Event metadata only. Message content and completed streams are excluded. */
	getActiveExecutions(scope = this.executionScope): ActiveEventExecution[] {
		return Array.from(this.eventStreams.entries()).flatMap(
			([streamId, stream]) =>
				stream.executionPromise &&
				!stream.isComplete &&
				stream.activeExecution &&
				stream.activeExecution.scope === scope &&
				stream.owner === this.backend
					? [{ streamId, ...stream.activeExecution }]
					: [],
		);
	}

	getBackgroundStreams(): {
		streamId: string;
		path?: string;
		title?: string;
		preview?: string;
		interfaceType?: string;
	}[] {
		const backgroundStreams: {
			streamId: string;
			path?: string;
			title?: string;
			preview?: string;
			interfaceType?: string;
		}[] = [];

		for (const [streamId, stream] of this.eventStreams.entries()) {
			// Finished streams wait here for their chat to come back; they are not running.
			if (stream.subscribers.size === 0 && this.isStreamActive(streamId)) {
				let preview = "";
				for (let i = stream.accumulatedEvents.length - 1; i >= 0; i--) {
					const ev = stream.accumulatedEvents[i];
					if (
						ev.event_type === "chat_stream_partial" &&
						ev.payload.chunk?.choices?.[0]?.delta?.content
					) {
						preview = ev.payload.chunk.choices[0].delta.content;
						break;
					}
					if (
						ev.event_type === "chat_stream" &&
						ev.payload.response?.choices?.[0]?.message?.content
					) {
						preview = ev.payload.response.choices[0].message.content;
						break;
					}
				}

				backgroundStreams.push({
					streamId,
					path: stream.path,
					title: stream.title,
					preview:
						preview.substring(0, 100) + (preview.length > 100 ? "..." : ""),
					interfaceType: stream.interfaceType,
				});
			}
		}

		return backgroundStreams;
	}
}
