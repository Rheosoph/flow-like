import { type WorkerPort, portFromSharedWorker } from "../shared-worker-port";
import { END_SILENCE_MS, TURN_CHECK_SILENCE_MS } from "./call-listening";
import { TURN_WINDOW_SAMPLES } from "./call-turn-features";
import type {
	TurnModelRole,
	TurnWorkerRequest,
	TurnWorkerResponse,
} from "./call-turn-protocol";

export interface CallTurnDetector {
	/** The voice-activity model answers; false while it loads or after it failed. */
	readonly ready: boolean;
	/** The turn-completion model answers. */
	readonly turnReady: boolean;
	/** Speech probability of 16 kHz audio; chunks of one stream are evaluated in call order. */
	speech(samples: Float32Array, stream?: string): Promise<number | null>;
	resetSpeech(stream?: string): void;
	/** Probability that the speaker finished, from the last up to 8 s of their utterance. */
	turnComplete(samples: Float32Array): Promise<number | null>;
	/** Called while audio arrives and the models are not ready, so given-up models can come back. */
	wake?(): void;
}

export type TurnWorkerPort = WorkerPort<TurnWorkerRequest, TurnWorkerResponse>;
export type TurnWorkerFactory = (role: TurnModelRole) => TurnWorkerPort;

/** How long a failed load keeps a model off, by consecutive failure. */
export const TURN_RETRY_AFTER_MS = [60_000, 5 * 60_000, 30 * 60_000] as const;
export const TURN_IDLE_DISPOSE_MS = 30_000;
/** A worker that neither loads nor fails in this time counts as failed: a slow download, or a
 * connection that reached a shared worker in the moment it closed and is never answered. */
export const TURN_LOAD_TIMEOUT_MS = 120_000;
export const TURN_REQUEST_TIMEOUT_MS = 3_000;
export const TURN_MAX_TIMEOUTS = 3;
/** A turn answer later than this arrives after the silence rule already ended the utterance. */
export const TURN_ANSWER_BUDGET_MS = END_SILENCE_MS - TURN_CHECK_SILENCE_MS;
export const TURN_MAX_LATE_ANSWERS = 3;
/** Below this `navigator.deviceMemory` (GB) a call loads no model and keeps the energy rule. */
export const MIN_CALL_MODEL_DEVICE_MEMORY_GB = 4;
/** The turn model needs a second runtime about twice the first one's size. */
export const MIN_TURN_MODEL_DEVICE_MEMORY_GB = 8;
const DEFAULT_STREAM = "default";

export interface CallModelPlan {
	speech: boolean;
	turn: boolean;
}

interface DeviceFacts {
	deviceMemory?: number;
	hardwareConcurrency?: number;
	maxTouchPoints?: number;
	userAgent?: string;
	userAgentData?: { mobile?: boolean };
}

function handheld(device: DeviceFacts): boolean {
	const agent = device.userAgent ?? "";
	return (
		device.userAgentData?.mobile === true ||
		/Android|iPhone|iPad|iPod/i.test(agent) ||
		(/Macintosh/.test(agent) && (device.maxTouchPoints ?? 0) > 1)
	);
}

/**
 * Which call models this device loads. Phones and devices that report little memory or few cores
 * skip the turn model; devices that report very little memory load none.
 */
export function callModelPlan(
	device: DeviceFacts | undefined = typeof navigator === "undefined"
		? undefined
		: (navigator as DeviceFacts),
): CallModelPlan {
	if (!device) return { speech: false, turn: false };
	const memory = device.deviceMemory;
	if (memory !== undefined && memory < MIN_CALL_MODEL_DEVICE_MEMORY_GB) {
		return { speech: false, turn: false };
	}
	const turn =
		(memory === undefined || memory >= MIN_TURN_MODEL_DEVICE_MEMORY_GB) &&
		(device.hardwareConcurrency ?? 4) >= 4 &&
		!handheld(device);
	return { speech: true, turn };
}

interface Waiting {
	resolve: (probability: number | null) => void;
	timer: ReturnType<typeof setTimeout>;
}

/**
 * The page's connection to one model's worker. The worker is shared by every tab and gives its
 * model up when no tab needs it; this side gives the connection up on failure, on page hide and
 * when the worker says it closed.
 */
class ModelLink {
	ready = false;
	private worker?: TurnWorkerPort;
	private waiting = new Map<number, Waiting>();
	private sequence = 0;
	private timeouts = 0;
	private failures = 0;
	private retryAt = 0;
	private loading?: Promise<void>;
	private settleLoading?: () => void;
	private loadTimer?: ReturnType<typeof setTimeout>;
	private unbindPageHide?: () => void;

	constructor(
		private role: TurnModelRole,
		private createWorker: TurnWorkerFactory,
		private now: () => number,
	) {}

	/** Connects and loads the model; resolves once it is ready or failed, at once while backing off. */
	start(): Promise<void> {
		if (this.worker) return this.loading ?? Promise.resolve();
		if (this.now() < this.retryAt) return Promise.resolve();
		this.loading = new Promise<void>((resolve) => {
			this.settleLoading = resolve;
		});
		try {
			const worker = this.createWorker(this.role);
			this.worker = worker;
			worker.onmessage = (event) => this.receive(worker, event.data);
			worker.onerror = () => this.fail(worker);
			this.bindPageHide(worker);
			worker.postMessage({ type: "init", role: this.role });
			this.loadTimer = setTimeout(
				() => this.fail(worker),
				TURN_LOAD_TIMEOUT_MS,
			);
		} catch {
			this.fail(this.worker);
		}
		return this.loading ?? Promise.resolve();
	}

	post(message: TurnWorkerRequest) {
		const worker = this.worker;
		if (!this.ready || !worker) return;
		try {
			worker.postMessage(message);
		} catch {
			this.fail(worker);
		}
	}

	request(build: (id: number) => TurnWorkerRequest): Promise<number | null> {
		if (!this.ready || !this.worker) return Promise.resolve(null);
		const id = ++this.sequence;
		return new Promise((resolve) => {
			const timer = setTimeout(
				() => this.timedOut(id),
				TURN_REQUEST_TIMEOUT_MS,
			);
			this.waiting.set(id, { resolve, timer });
			this.post(build(id));
		});
	}

	/** Gives the connection up; the shared worker frees the model once no tab holds it. */
	release() {
		for (const entry of this.waiting.values()) {
			clearTimeout(entry.timer);
			entry.resolve(null);
		}
		this.waiting.clear();
		this.ready = false;
		this.timeouts = 0;
		this.unbindPageHide?.();
		this.unbindPageHide = undefined;
		this.finishLoading();
		const worker = this.worker;
		this.worker = undefined;
		if (!worker) return;
		worker.onmessage = null;
		worker.onerror = null;
		try {
			worker.terminate();
		} catch {
			// The worker is gone either way.
		}
	}

	private bindPageHide(worker: TurnWorkerPort) {
		if (typeof window === "undefined") return;
		const page = window;
		const onPageHide = () => {
			if (this.worker === worker) this.release();
		};
		page.addEventListener("pagehide", onPageHide);
		this.unbindPageHide = () =>
			page.removeEventListener("pagehide", onPageHide);
	}

	private receive(worker: TurnWorkerPort, message: TurnWorkerResponse) {
		if (worker !== this.worker) return;
		switch (message.type) {
			case "ready":
				this.ready = true;
				this.failures = 0;
				this.timeouts = 0;
				this.finishLoading();
				return;
			case "failed":
				this.fail(worker);
				return;
			case "idle":
				this.release();
				return;
			case "result": {
				const entry = this.waiting.get(message.id);
				if (!entry) return;
				clearTimeout(entry.timer);
				this.waiting.delete(message.id);
				this.timeouts = 0;
				entry.resolve(message.probability);
			}
		}
	}

	private timedOut(id: number) {
		const entry = this.waiting.get(id);
		if (!entry) return;
		this.waiting.delete(id);
		entry.resolve(null);
		this.timeouts += 1;
		if (this.timeouts >= TURN_MAX_TIMEOUTS) this.fail(this.worker);
	}

	private fail(worker: TurnWorkerPort | undefined) {
		if (worker !== this.worker) return;
		this.release();
		const wait =
			TURN_RETRY_AFTER_MS[
				Math.min(this.failures, TURN_RETRY_AFTER_MS.length - 1)
			];
		this.failures += 1;
		this.retryAt = this.now() + wait;
	}

	private finishLoading() {
		clearTimeout(this.loadTimer);
		this.loadTimer = undefined;
		this.settleLoading?.();
		this.settleLoading = undefined;
		this.loading = undefined;
	}
}

/**
 * The page's call models. Each runs in its own worker that every tab shares, loads only while a
 * call listens and only on a device that can afford it, and is given up again when calls end, a
 * load fails or the device turns out too slow.
 */
export class BrowserCallTurnDetector implements CallTurnDetector {
	private speechLink: ModelLink;
	private turnLink: ModelLink;
	private holders = 0;
	private disposeTimer?: ReturnType<typeof setTimeout>;
	private lateTurns = 0;
	private turnTooSlow = false;
	/** How long the last turn check took, round trip; undefined before the first one. */
	lastTurnMs?: number;

	constructor(
		createWorker: TurnWorkerFactory,
		private now: () => number = Date.now,
		private disposeAfterMs: number = TURN_IDLE_DISPOSE_MS,
		private plan: () => CallModelPlan = callModelPlan,
	) {
		this.speechLink = new ModelLink("speech", createWorker, now);
		this.turnLink = new ModelLink("turn", createWorker, now);
	}

	get ready() {
		return this.speechLink.ready;
	}

	/** False on a device too slow for the answer to arrive before the silence rule decides. */
	get turnReady() {
		return this.turnLink.ready && !this.turnTooSlow;
	}

	speech(samples: Float32Array, stream: string = DEFAULT_STREAM) {
		return this.speechLink.request((id) => ({
			type: "speech",
			id,
			stream,
			samples,
		}));
	}

	resetSpeech(stream: string = DEFAULT_STREAM) {
		this.speechLink.post({ type: "reset", stream });
	}

	async turnComplete(samples: Float32Array) {
		if (!this.turnReady) return null;
		const window =
			samples.length > TURN_WINDOW_SAMPLES
				? samples.slice(samples.length - TURN_WINDOW_SAMPLES)
				: samples;
		const asked = this.now();
		const probability = await this.turnLink.request((id) => ({
			type: "turn",
			id,
			samples: window,
		}));
		this.lastTurnMs = this.now() - asked;
		this.lateTurns =
			this.lastTurnMs > TURN_ANSWER_BUDGET_MS ? this.lateTurns + 1 : 0;
		if (this.lateTurns >= TURN_MAX_LATE_ANSWERS && !this.turnTooSlow) {
			this.turnTooSlow = true;
			this.turnLink.release();
			console.info(
				"[call-latency] the turn model answers too slowly on this device; calls use the silence rule",
				{ lastTurnMs: this.lastTurnMs, budgetMs: TURN_ANSWER_BUDGET_MS },
			);
		}
		return probability;
	}

	/** Starts loading the models and keeps them alive until released. */
	acquire(): () => void {
		this.holders += 1;
		clearTimeout(this.disposeTimer);
		this.disposeTimer = undefined;
		void this.start();
		let released = false;
		return () => {
			if (released) return;
			released = true;
			this.holders -= 1;
			if (this.holders > 0 || this.disposeTimer !== undefined) return;
			this.disposeTimer = setTimeout(() => {
				this.disposeTimer = undefined;
				if (this.holders === 0) this.dispose();
			}, this.disposeAfterMs);
		};
	}

	async warm(): Promise<void> {
		const release = this.acquire();
		try {
			await this.start();
		} finally {
			release();
		}
	}

	/** Asks for the models again after they were given up while a call still listens. */
	wake() {
		if (this.holders > 0) void this.start();
	}

	dispose() {
		clearTimeout(this.disposeTimer);
		this.disposeTimer = undefined;
		this.lateTurns = 0;
		this.speechLink.release();
		this.turnLink.release();
	}

	private async start(): Promise<void> {
		const plan = this.plan();
		if (!plan.speech) return;
		await Promise.all([
			this.speechLink.start(),
			plan.turn && !this.turnTooSlow ? this.turnLink.start() : undefined,
		]);
	}
}

function createDedicatedTurnWorker(): TurnWorkerPort {
	return new Worker(new URL("./call-turn.worker.ts", import.meta.url), {
		type: "module",
	});
}

// Spelled out per model: bundlers only follow a worker URL written inside its constructor call.
const SHARED_WORKERS: Record<TurnModelRole, () => SharedWorker> = {
	speech: () =>
		new SharedWorker(new URL("./call-turn.worker.ts", import.meta.url), {
			type: "module",
			name: "flow-like-call-speech",
		}),
	turn: () =>
		new SharedWorker(new URL("./call-turn.worker.ts", import.meta.url), {
			type: "module",
			name: "flow-like-call-turn",
		}),
};

/** One worker per model for every tab. A dedicated worker is the fallback. */
export function createTurnWorkerPort(role: TurnModelRole): TurnWorkerPort {
	if (typeof SharedWorker !== "undefined" && typeof window !== "undefined") {
		try {
			return portFromSharedWorker<TurnWorkerRequest, TurnWorkerResponse>(
				SHARED_WORKERS[role](),
				createDedicatedTurnWorker,
			);
		} catch {
			// Some WebViews expose the constructor and still reject module workers.
		}
	}
	return createDedicatedTurnWorker();
}

let shared: BrowserCallTurnDetector | undefined;

export function getCallTurnDetector(): BrowserCallTurnDetector {
	shared ??= new BrowserCallTurnDetector(createTurnWorkerPort);
	return shared;
}

/** Loads the voice-activity and turn models ahead of a call; never rejects. */
export async function warmCallListening(): Promise<void> {
	try {
		await getCallTurnDetector().warm();
	} catch {
		// The energy rule keeps the call working without the models.
	}
}
