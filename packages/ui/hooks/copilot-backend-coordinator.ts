import type { AgentBackendProvider } from "../components/flowpilot/types";

export interface CopilotBackendConnectionSnapshot {
	isRunning: boolean;
	isConnecting: boolean;
	error: string | null;
	retryAtMs: number;
}

interface CopilotBackendConnectionRecord
	extends CopilotBackendConnectionSnapshot {
	startPromise?: Promise<void>;
	stopPromise?: Promise<void>;
	failureCount: number;
	generation: number;
	listeners: Set<(snapshot: CopilotBackendConnectionSnapshot) => void>;
}

const INITIAL_SNAPSHOT: CopilotBackendConnectionSnapshot = {
	isRunning: false,
	isConnecting: false,
	error: null,
	retryAtMs: 0,
};

function errorMessage(error: unknown) {
	return error instanceof Error ? error.message : String(error);
}

/**
 * Process-wide frontend coordinator for native agent backends.
 *
 * FlowPilot is mounted in several surfaces at once. Keeping startup state at module scope makes
 * those hook instances share one native invocation. Crucially, the raw invocation remains the
 * singleflight promise after an individual UI caller times out, preventing timeout/retry storms
 * from spawning more CLI processes while the first native start is still alive.
 */
export class CopilotBackendConnectionCoordinator {
	private readonly records = new Map<
		AgentBackendProvider,
		CopilotBackendConnectionRecord
	>();

	constructor(
		private readonly baseBackoffMs = 1_000,
		private readonly maxBackoffMs = 30_000,
	) {}

	private record(backend: AgentBackendProvider) {
		let record = this.records.get(backend);
		if (!record) {
			record = {
				...INITIAL_SNAPSHOT,
				failureCount: 0,
				generation: 0,
				listeners: new Set(),
			};
			this.records.set(backend, record);
		}
		return record;
	}

	private snapshotRecord(
		record: CopilotBackendConnectionRecord,
	): CopilotBackendConnectionSnapshot {
		return {
			isRunning: record.isRunning,
			isConnecting: record.isConnecting,
			error: record.error,
			retryAtMs: record.retryAtMs,
		};
	}

	private publish(record: CopilotBackendConnectionRecord) {
		const snapshot = this.snapshotRecord(record);
		for (const listener of record.listeners) listener(snapshot);
	}

	snapshot(backend: AgentBackendProvider) {
		return this.snapshotRecord(this.record(backend));
	}

	subscribe(
		backend: AgentBackendProvider,
		listener: (snapshot: CopilotBackendConnectionSnapshot) => void,
	) {
		const record = this.record(backend);
		record.listeners.add(listener);
		listener(this.snapshotRecord(record));
		return () => {
			record.listeners.delete(listener);
		};
	}

	reconcile(backend: AgentBackendProvider, isRunning: boolean) {
		const record = this.record(backend);
		record.isRunning = isRunning;
		if (isRunning) {
			record.failureCount = 0;
			record.retryAtMs = 0;
			record.error = null;
		}
		this.publish(record);
	}

	start(
		backend: AgentBackendProvider,
		operation: () => Promise<unknown>,
		nowMs = Date.now(),
	): Promise<void> {
		const record = this.record(backend);
		if (record.isRunning) return Promise.resolve();
		if (record.startPromise) return record.startPromise;
		if (record.stopPromise) {
			return Promise.reject(
				new Error(
					`${backend} is still stopping; wait before starting it again.`,
				),
			);
		}
		if (record.retryAtMs > nowMs) {
			return Promise.reject(
				new Error(
					`${backend} startup is cooling down for ${record.retryAtMs - nowMs}ms after a failure.`,
				),
			);
		}

		const generation = ++record.generation;
		record.isConnecting = true;
		record.error = null;
		this.publish(record);
		const promise = Promise.resolve()
			.then(operation)
			.then(() => {
				if (record.generation !== generation) return;
				record.isRunning = true;
				record.failureCount = 0;
				record.retryAtMs = 0;
				record.error = null;
			})
			.catch((error) => {
				if (record.generation === generation) {
					record.failureCount += 1;
					const delay = Math.min(
						this.maxBackoffMs,
						this.baseBackoffMs * 2 ** (record.failureCount - 1),
					);
					record.retryAtMs = Date.now() + delay;
					record.error = errorMessage(error);
				}
				throw error;
			})
			.finally(() => {
				if (record.startPromise === promise) record.startPromise = undefined;
				if (record.generation === generation) record.isConnecting = false;
				this.publish(record);
			});
		record.startPromise = promise;
		return promise;
	}

	stop(
		backend: AgentBackendProvider,
		operation: () => Promise<unknown>,
	): Promise<void> {
		const record = this.record(backend);
		if (record.stopPromise) return record.stopPromise;
		const pendingStart = record.startPromise;
		const generation = ++record.generation;
		record.isConnecting = true;
		record.error = null;
		this.publish(record);
		const promise = (
			pendingStart ? pendingStart.catch(() => undefined) : Promise.resolve()
		)
			.then(operation)
			.then(() => {
				if (record.generation !== generation) return;
				record.isRunning = false;
				record.failureCount = 0;
				record.retryAtMs = 0;
			})
			.catch((error) => {
				if (record.generation === generation)
					record.error = errorMessage(error);
				throw error;
			})
			.finally(() => {
				if (record.stopPromise === promise) record.stopPromise = undefined;
				if (record.generation === generation) record.isConnecting = false;
				this.publish(record);
			});
		record.stopPromise = promise;
		return promise;
	}
}

export const copilotBackendConnectionCoordinator =
	new CopilotBackendConnectionCoordinator();
