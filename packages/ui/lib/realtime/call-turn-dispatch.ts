import { TURN_SAMPLE_RATE, TURN_WINDOW_SAMPLES } from "./call-turn-features";
import type {
	TurnModelRole,
	TurnWorkerRequest,
	TurnWorkerResponse,
} from "./call-turn-protocol";

export interface TurnDispatchPort {
	postMessage(message: TurnWorkerResponse): void;
	start?(): void;
	close?(): void;
	onmessage: ((event: MessageEvent<TurnWorkerRequest>) => void) | null;
}

/** The models behind the dispatcher. One set serves every connected tab. */
export interface TurnModels {
	load(role: TurnModelRole): Promise<void>;
	speech(stream: string, samples: Float32Array): Promise<number>;
	turn(samples: Float32Array): Promise<number>;
	reset(stream: string): void;
	release(): Promise<void>;
}

export interface TurnDispatchTimers {
	set(callback: () => void, ms: number): unknown;
	clear(handle: unknown): void;
}

/** A tab moving between views gets this long to reconnect before the models are freed. */
export const TURN_IDLE_CLOSE_MS = 1_000;
/** A tab that vanished without disconnecting keeps the models at most twice this long. */
export const TURN_INACTIVE_CLOSE_MS = 62_000;
export const TURN_MAX_STREAMS_PER_PORT = 4;
export const TURN_MAX_SPEECH_SAMPLES = TURN_SAMPLE_RATE;
export const TURN_MAX_TURN_SAMPLES = TURN_WINDOW_SAMPLES;

const defaultTimers: TurnDispatchTimers = {
	set: (callback, ms) => setTimeout(callback, ms),
	clear: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

interface Connection {
	id: number;
	streams: string[];
}

/**
 * One inference queue for every connected tab. A model loads once, replies stay on the requesting
 * port, and the scope ends (`onIdle`) once no tab needs it, so its memory is held only while a
 * call listens. Each model gets its own worker: one queue for both would hold voice-activity
 * answers back for as long as a turn check runs.
 */
export function createTurnDispatcher(
	models: TurnModels,
	onIdle?: () => void,
	timers: TurnDispatchTimers = defaultTimers,
) {
	const ports = new Map<TurnDispatchPort, Connection>();
	const loads = new Map<TurnModelRole, Promise<void>>();
	let queue: Promise<void> = Promise.resolve();
	let sequence = 0;
	let idleTimer: unknown;
	let inactiveTimer: unknown;
	let active = false;
	let closing = false;

	const enqueue = (task: () => Promise<void>) => {
		queue = queue.then(task).catch(() => {});
	};

	function deliver(port: TurnDispatchPort, message: TurnWorkerResponse) {
		if (!ports.has(port)) return;
		try {
			port.postMessage(message);
		} catch {
			detach(port);
		}
	}

	function loaded(role: TurnModelRole): Promise<void> {
		let pending = loads.get(role);
		if (!pending) {
			pending = models.load(role);
			loads.set(role, pending);
			pending.catch(() => {
				if (loads.get(role) === pending) loads.delete(role);
			});
		}
		return pending;
	}

	async function announce(port: TurnDispatchPort, role: TurnModelRole) {
		try {
			await loaded(role);
			deliver(port, { type: "ready", role });
		} catch (cause) {
			deliver(port, {
				type: "failed",
				role,
				message: cause instanceof Error ? cause.message : String(cause),
			});
		}
	}

	async function close() {
		if (closing) return;
		closing = true;
		timers.clear(idleTimer);
		timers.clear(inactiveTimer);
		for (const port of [...ports.keys()]) {
			deliver(port, { type: "idle" });
			drop(port);
		}
		await models.release().catch(() => {});
		onIdle?.();
	}

	function armInactivity() {
		if (!onIdle || closing) return;
		timers.clear(inactiveTimer);
		inactiveTimer = timers.set(() => {
			if (active) {
				active = false;
				armInactivity();
			} else void close();
		}, TURN_INACTIVE_CLOSE_MS);
	}

	function scheduleIdle() {
		if (!onIdle || closing || ports.size || idleTimer !== undefined) return;
		idleTimer = timers.set(() => {
			idleTimer = undefined;
			if (!ports.size) void close();
		}, TURN_IDLE_CLOSE_MS);
	}

	const streamKey = (connection: Connection, stream: string) =>
		`${connection.id}:${stream}`;

	function touchStream(connection: Connection, stream: string): string {
		const at = connection.streams.indexOf(stream);
		if (at >= 0) connection.streams.splice(at, 1);
		connection.streams.push(stream);
		while (connection.streams.length > TURN_MAX_STREAMS_PER_PORT) {
			const oldest = connection.streams.shift();
			if (oldest !== undefined) models.reset(streamKey(connection, oldest));
		}
		return streamKey(connection, stream);
	}

	function forgetStream(connection: Connection, stream: string) {
		const at = connection.streams.indexOf(stream);
		if (at < 0) return;
		connection.streams.splice(at, 1);
		models.reset(streamKey(connection, stream));
	}

	function bounded(samples: Float32Array, limit: number, what: string) {
		if (samples.length > limit) throw new Error(`${what} is too long`);
		return samples;
	}

	function drop(port: TurnDispatchPort) {
		const connection = ports.get(port);
		if (!connection) return;
		for (const stream of connection.streams) {
			models.reset(streamKey(connection, stream));
		}
		ports.delete(port);
		port.onmessage = null;
		port.close?.();
	}

	function detach(port: TurnDispatchPort) {
		drop(port);
		scheduleIdle();
	}

	function answer(
		port: TurnDispatchPort,
		id: number,
		role: TurnModelRole,
		run: () => Promise<number>,
	) {
		enqueue(async () => {
			let probability: number | null = null;
			try {
				// Requests never start a load: a model that failed must not be fetched again per block.
				const ready = loads.get(role);
				if (ready && ports.has(port)) {
					await ready;
					probability = await run();
				}
			} catch {
				probability = null;
			}
			deliver(port, { type: "result", id, probability });
		});
	}

	function handle(port: TurnDispatchPort, message: TurnWorkerRequest) {
		const connection = ports.get(port);
		if (!connection) return;
		switch (message.type) {
			case "disconnect":
				detach(port);
				return;
			case "init":
				enqueue(() => announce(port, message.role));
				return;
			case "reset":
				enqueue(async () => forgetStream(connection, message.stream));
				return;
			case "speech":
				answer(port, message.id, "speech", () => {
					const samples = bounded(
						message.samples,
						TURN_MAX_SPEECH_SAMPLES,
						"Voice-activity block",
					);
					return models.speech(
						touchStream(connection, message.stream),
						samples,
					);
				});
				return;
			case "turn":
				answer(port, message.id, "turn", () =>
					models.turn(
						bounded(message.samples, TURN_MAX_TURN_SAMPLES, "Turn window"),
					),
				);
				return;
		}
	}

	function attach(port: TurnDispatchPort) {
		if (closing) {
			try {
				port.postMessage({ type: "idle" });
			} catch {
				// The tab is gone already.
			}
			port.close?.();
			return;
		}
		timers.clear(idleTimer);
		idleTimer = undefined;
		ports.set(port, { id: ++sequence, streams: [] });
		port.onmessage = (event) => {
			active = true;
			handle(port, event.data);
		};
		port.start?.();
		armInactivity();
	}

	return { attach };
}
