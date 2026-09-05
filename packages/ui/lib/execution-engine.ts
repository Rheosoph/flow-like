import type { IBackendState } from "../state/backend-state";
import type { ILogMetadata, PageTrigger } from "./schema";
import type { IRunPayload } from "./schema";
import type { IIntercomEvent } from "./schema/events/intercom-event";
import { captureTelemetryError } from "./telemetry/errors";

interface ISubscriber {
	onEvents: (events: IIntercomEvent[]) => void;
	onComplete?: (events: IIntercomEvent[]) => void;
}

interface IEventStream {
	subscribers: Map<string, ISubscriber>;
	accumulatedEvents: IIntercomEvent[];
	lastSentIndex: Map<string, number>;
	seenEventIds: Set<string>;
	executionPromise?: Promise<any>;
	isComplete: boolean;
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
	path?: string;
	title?: string;
	interfaceType?: string;
	skipConsentCheck?: boolean;
	pageTrigger?: PageTrigger;
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

export type ExecuteEventFn = (
	appId: string,
	eventId: string,
	payload: IRunPayload,
	streamState?: boolean,
	onEventId?: (id: string) => void,
	cb?: (event: IIntercomEvent[]) => void,
	skipConsentCheck?: boolean,
	pageTrigger?: PageTrigger,
) => Promise<ILogMetadata | undefined>;

export class ExecutionEngineProvider {
	private eventStreams: Map<string, IEventStream> = new Map();
	private backend: IBackendState | null = null;
	private globalListeners: Set<() => void> = new Set();
	private executeEventFn: ExecuteEventFn | null = null;

	constructor() {}

	setBackend(backend: IBackendState): void {
		this.backend = backend;
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

	subscribeToEventStream(
		streamId: string,
		subscriberId: string,
		onEvents: (events: IIntercomEvent[]) => void,
		onComplete?: (events: IIntercomEvent[]) => void,
	): void {
		if (!this.eventStreams.has(streamId)) {
			this.eventStreams.set(streamId, {
				subscribers: new Map(),
				accumulatedEvents: [],
				lastSentIndex: new Map(),
				seenEventIds: new Set(),
				isComplete: false,
			});
		}

		const stream = this.eventStreams.get(streamId)!;

		// Register subscriber
		stream.subscribers.set(subscriberId, { onEvents, onComplete });

		// Send accumulated events immediately
		if (stream.accumulatedEvents.length > 0) {
			onEvents(stream.accumulatedEvents);
			stream.lastSentIndex.set(subscriberId, stream.accumulatedEvents.length);
		} else {
			stream.lastSentIndex.set(subscriberId, 0);
		}

		// If already complete, notify immediately
		if (stream.isComplete && onComplete) {
			onComplete(stream.accumulatedEvents);
		}

		this.notifyGlobalListeners();
	}

	unsubscribeFromEventStream(streamId: string, subscriberId: string): void {
		const stream = this.eventStreams.get(streamId);
		if (stream) {
			stream.subscribers.delete(subscriberId);
			stream.lastSentIndex.delete(subscriberId);

			// Only delete if complete AND no subscribers
			if (stream.subscribers.size === 0 && stream.isComplete) {
				this.eventStreams.delete(streamId);
			}
			this.notifyGlobalListeners();
		}
	}

	async executeEvent(
		streamId: string,
		options: IExecuteEventOptions,
	): Promise<any> {
		if (!this.backend) {
			throw new Error("Backend not initialized in ExecutionEngineProvider");
		}

		let stream = this.eventStreams.get(streamId);
		if (!stream) {
			stream = {
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
		} else {
			// If the stream was previously complete, reset it for the new execution
			if (stream.isComplete) {
				stream.isComplete = false;
				stream.accumulatedEvents = [];
				stream.lastSentIndex.clear();
				stream.seenEventIds.clear();
				stream.executionPromise = undefined; // Clear old promise to allow new execution
				// We keep existing subscribers, but reset their sent index
				for (const subscriberId of stream.subscribers.keys()) {
					stream.lastSentIndex.set(subscriberId, 0);
				}
			}

			// Update metadata if provided
			if (options.path) stream.path = options.path;
			if (options.title) stream.title = options.title;
			if (options.interfaceType) stream.interfaceType = options.interfaceType;
		}

		this.notifyGlobalListeners();

		if (stream.executionPromise) {
			// Stream is already executing — reject so callers know the send was dropped
			throw Object.assign(
				new Error("A stream is already active for this session"),
				{ isActiveStreamError: true },
			);
		}

		// Use the executeEventFn if set (handles runtime variables), otherwise fall back to direct call
		const executeEventCall =
			this.executeEventFn ??
			this.backend.eventState.executeEvent.bind(this.backend.eventState);

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
				options.onExecutionStart?.(executionId);
			},
			(events: IIntercomEvent[]) => {
				// Deduplicate by event_id
				const unique = events.filter((e) => {
					if (stream!.seenEventIds.has(e.event_id)) return false;
					stream!.seenEventIds.add(e.event_id);
					return true;
				});

				if (unique.length > 0) {
					stream!.accumulatedEvents.push(...unique);

					// Publish to all subscribers
					for (const [
						subscriberId,
						subscriber,
					] of stream!.subscribers.entries()) {
						const lastSent = stream!.lastSentIndex.get(subscriberId) ?? 0;
						const newEvents = stream!.accumulatedEvents.slice(lastSent);

						if (newEvents.length > 0) {
							subscriber.onEvents(newEvents);
							stream!.lastSentIndex.set(
								subscriberId,
								stream!.accumulatedEvents.length,
							);
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
								options.onIncrementalSave(stream!.accumulatedEvents, false),
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
		);

		stream.executionPromise = executionPromise;

		executionPromise
			.then(async () => {
				stream!.isComplete = true;

				// Final incremental save
				if (options.onIncrementalSave) {
					try {
						await options.onIncrementalSave(stream!.accumulatedEvents, true);
					} catch (err) {
						console.error("[ExecutionEngine] Final save error:", err);
					}
				}

				// Notify all subscribers of completion
				for (const subscriber of stream!.subscribers.values()) {
					if (subscriber.onComplete) {
						subscriber.onComplete(stream!.accumulatedEvents);
					}
				}

				this.notifyGlobalListeners();
			})
			.catch(async (error) => {
				console.error("Execution error:", error);
				captureTelemetryError(error, {
					culprit: "execution-engine.execute_event",
					context: {
						accumulatedEvents: stream!.accumulatedEvents.length,
					},
				});
				stream!.isComplete = true;

				// Still do final save on error to preserve partial state
				if (options.onIncrementalSave && stream!.accumulatedEvents.length > 0) {
					try {
						await options.onIncrementalSave(stream!.accumulatedEvents, true);
					} catch (err) {
						console.error("[ExecutionEngine] Error save error:", err);
					}
				}

				// Notify subscribers of completion
				for (const subscriber of stream!.subscribers.values()) {
					if (subscriber.onComplete) {
						subscriber.onComplete(stream!.accumulatedEvents);
					}
				}

				this.notifyGlobalListeners();
			});

		return executionPromise;
	}

	isStreamActive(streamId: string): boolean {
		const stream = this.eventStreams.get(streamId);
		return stream ? !stream.isComplete : false;
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
			if (stream.subscribers.size === 0) {
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
