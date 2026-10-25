import type { IBackendState } from "../state/backend-state";
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
	subscribers: Map<string, ISubscriber>;
	accumulatedEvents: IIntercomEvent[];
	lastSentIndex: Map<string, number>;
	seenEventIds: Set<string>;
	executionPromise?: Promise<any>;
	isComplete: boolean;
	/** The run ended, its final save landed and its watchers finished their completion. */
	settled?: boolean;
	failure?: unknown;
	completedAt?: number;
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
	appId: string;
	eventId: string;
	payload: IRunPayload;
	streamState?: boolean;
	onExecutionStart?: (executionId: string) => void;
	/** Receives each new event once, even with no subscribers. Excludes replay. */
	onLiveEvents?: (events: IIntercomEvent[]) => void;
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

	constructor() {}

	setBackend(backend: IBackendState): void {
		this.backend = backend;
	}

	setExecutionScope(scope: string): void {
		this.executionScope = scope;
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
		this.globalListeners.forEach((listener) => listener());
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
	private async settleRun(
		stream: IEventStream,
		onFinalSave: OnIncrementalSaveFn | undefined,
		failed: boolean,
	) {
		const events = stream.accumulatedEvents;
		const watchers = [...stream.subscribers];
		stream.isComplete = true;
		stream.completedAt = Date.now();
		if (onFinalSave && (!failed || events.length > 0)) {
			try {
				await onFinalSave(events, true);
			} catch (error) {
				console.error("[ExecutionEngine] Final save error:", error);
			}
		}
		const completions = watchers.map(([subscriberId, subscriber]) => {
			const onComplete = subscriber.onComplete;
			// One that left meanwhile settles itself when it returns; one that joined got it on joining.
			if (!onComplete || stream.subscribers.get(subscriberId) !== subscriber)
				return undefined;
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
		stream.settled = true;
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
			existing.isComplete = false;
			existing.settled = false;
			existing.failure = undefined;
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

		// Send accumulated events immediately
		const replay = stream.accumulatedEvents;
		stream.lastSentIndex.set(subscriberId, replay.length);
		if (replay.length > 0) this.deliver(subscriberId, () => onEvents(replay));

		// If already complete, notify immediately
		if (stream.isComplete && onComplete) {
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
				(stream.isComplete || !stream.executionPromise)
			) {
				this.eventStreams.delete(streamId);
			}
			this.notifyGlobalListeners();
		}
	}

	async executeEvent(
		streamId: string,
		options: IExecuteEventOptions,
	): Promise<any> {
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
		stream.activeExecution = {
			...(this.executionScope ? { scope: this.executionScope } : {}),
			appId: options.appId,
			eventId: options.eventId,
			title: options.title || "Workflow run",
			startedAt: new Date().toISOString(),
		};

		// Track event count for incremental saves
		const saveInterval = options.saveIntervalEvents ?? 10;
		const saveMinIntervalMs = options.saveMinIntervalMs ?? 1000;
		let eventsSinceLastSave = 0;
		let lastSaveAt = 0;

		const executionPromise = executeEventCall(
			options.appId,
			options.eventId,
			options.payload,
			options.streamState ?? false,
			(executionId: string) => {
				if (stream.activeExecution) stream.activeExecution.runId = executionId;
				this.notifyGlobalListeners();
				options.onExecutionStart?.(executionId);
			},
			(events: IIntercomEvent[]) => {
				// Deduplicate by event_id
				const unique = events.filter((e) => {
					if (stream.seenEventIds.has(e.event_id)) return false;
					stream.seenEventIds.add(e.event_id);
					return true;
				});

				if (unique.length > 0) {
					stream.accumulatedEvents.push(...unique);
					options.onLiveEvents?.(unique);

					// Publish to all subscribers
					for (const [subscriberId, subscriber] of stream.subscribers) {
						const lastSent = stream.lastSentIndex.get(subscriberId) ?? 0;
						const newEvents = stream.accumulatedEvents.slice(lastSent);

						if (newEvents.length > 0) {
							stream.lastSentIndex.set(
								subscriberId,
								stream.accumulatedEvents.length,
							);
							this.deliver(subscriberId, () => subscriber.onEvents(newEvents));
						}
					}

					// Incremental save logic
					if (options.onIncrementalSave) {
						eventsSinceLastSave += unique.length;
						if (
							eventsSinceLastSave >= saveInterval &&
							Date.now() - lastSaveAt >= saveMinIntervalMs
						) {
							eventsSinceLastSave = 0;
							lastSaveAt = Date.now();
							// Fire and forget - don't block event processing
							Promise.resolve(
								options.onIncrementalSave(stream.accumulatedEvents, false),
							).catch((err) =>
								console.error("[ExecutionEngine] Incremental save error:", err),
							);
						}
					}

					this.notifyGlobalListeners();
				}
			},
			options.skipConsentCheck,
			options.pageTrigger,
			options.beforeDispatch,
		);

		stream.settled = false;
		stream.executionPromise = executionPromise;
		this.notifyGlobalListeners();

		// Separate handlers: a failure while settling a successful run must not settle it twice.
		executionPromise.then(
			(metadata) => {
				stream.failure = completedRunFailure(
					stream.accumulatedEvents,
					metadata,
				);
				return this.settleRun(stream, options.onIncrementalSave, false);
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
				return this.settleRun(stream, options.onIncrementalSave, true);
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
