import {
	type EmotionInput,
	type EmotionLoadState,
	type EmotionWorkerRequest,
	type EmotionWorkerResponse,
	type ExpressionScores,
	boundEmotionInput,
} from "./protocol";

export interface EmotionWorkerPort {
	postMessage(message: EmotionWorkerRequest): void;
	terminate(): void;
	onmessage: ((event: MessageEvent<EmotionWorkerResponse>) => void) | null;
	onerror: ((event: ErrorEvent) => void) | null;
}

interface Request {
	id: number;
	owner: object;
	input: EmotionInput;
	resolve: (scores: ExpressionScores | null) => void;
}

export interface EmotionClient {
	warmup(): void;
	predict(owner: object, input: EmotionInput): Promise<ExpressionScores | null>;
	cancel(owner: object): void;
	subscribe(listener: (state: EmotionLoadState) => void): () => void;
}

const RETRY_AFTER_MS = 60_000;
const LOAD_TIMEOUT_MS = 10 * 60_000;
const INFERENCE_TIMEOUT_MS = 30_000;

export class BrowserEmotionClient implements EmotionClient {
	private worker?: EmotionWorkerPort;
	private warming = false;
	private active?: Request;
	private pending = new Map<object, Request>();
	private listeners = new Set<(state: EmotionLoadState) => void>();
	private state: EmotionLoadState = { status: "idle" };
	private sequence = 0;
	private retryAt = 0;
	private watchdog?: ReturnType<typeof setTimeout>;
	private releaseTimer?: ReturnType<typeof setTimeout>;

	constructor(private createWorker: () => EmotionWorkerPort) {}

	subscribe(listener: (state: EmotionLoadState) => void) {
		clearTimeout(this.releaseTimer);
		this.releaseTimer = undefined;
		this.listeners.add(listener);
		listener(this.state);
		return () => {
			this.listeners.delete(listener);
			this.scheduleRelease();
		};
	}

	private scheduleRelease() {
		if (
			this.listeners.size ||
			this.pending.size ||
			this.active ||
			!this.worker ||
			this.releaseTimer !== undefined
		)
			return;
		this.releaseTimer = setTimeout(() => {
			this.releaseTimer = undefined;
			if (this.listeners.size || this.pending.size || this.active) return;
			clearTimeout(this.watchdog);
			this.worker?.terminate();
			this.worker = undefined;
			this.warming = false;
			this.publish({ status: "idle" });
		}, 60_000);
	}

	private publish(state: EmotionLoadState) {
		this.state = state;
		for (const listener of this.listeners) listener(state);
	}

	warmup() {
		if (this.worker || Date.now() < this.retryAt) return;
		try {
			const worker = this.createWorker();
			this.worker = worker;
			this.warming = true;
			worker.onmessage = (event) => {
				if (this.worker !== worker) return;
				const message = event.data;
				if (message.type === "progress") {
					this.publish({ status: "loading", progress: message.progress });
					return;
				}
				if (message.type === "ready") {
					if (!this.warming) return;
					clearTimeout(this.watchdog);
					this.warming = false;
					this.publish({ status: "ready" });
					this.drain();
					return;
				}
				if (
					message.type === "error" &&
					(this.warming || message.id === this.active?.id)
				) {
					this.fail();
					return;
				}
				if (message.type !== "result" || message.id !== this.active?.id) return;
				clearTimeout(this.watchdog);
				const completed = this.active;
				this.active = undefined;
				completed.resolve(message.scores);
				this.drain();
				this.scheduleRelease();
			};
			worker.onerror = () => {
				if (this.worker === worker) this.fail();
			};
			this.publish({ status: "loading" });
			this.watchdog = setTimeout(() => this.fail(), LOAD_TIMEOUT_MS);
			worker.postMessage({ id: 0, warmup: true, draft: "", messages: [] });
		} catch {
			this.fail();
		}
	}

	predict(
		owner: object,
		input: EmotionInput,
	): Promise<ExpressionScores | null> {
		this.cancel(owner);
		const bounded = boundEmotionInput(input);
		if (!bounded.draft || Date.now() < this.retryAt)
			return Promise.resolve(null);
		return new Promise((resolve) => {
			this.pending.set(owner, {
				id: ++this.sequence,
				owner,
				input: bounded,
				resolve,
			});
			this.drain();
		});
	}

	cancel(owner: object) {
		this.pending.get(owner)?.resolve(null);
		this.pending.delete(owner);
		if (this.active?.owner === owner) {
			this.active.resolve(null);
			this.active.resolve = () => {};
		}
		this.scheduleRelease();
	}

	private drain() {
		if (this.active || !this.pending.size) return;
		if (!this.worker) {
			this.warmup();
			return;
		}
		if (this.warming) return;
		try {
			const next = this.pending.values().next().value;
			if (!next) return;
			this.pending.delete(next.owner);
			this.active = next;
			this.watchdog = setTimeout(() => this.fail(), INFERENCE_TIMEOUT_MS);
			this.worker.postMessage({ id: next.id, ...next.input });
		} catch {
			this.fail();
		}
	}

	private fail() {
		clearTimeout(this.watchdog);
		clearTimeout(this.releaseTimer);
		this.releaseTimer = undefined;
		this.worker?.terminate();
		this.worker = undefined;
		this.warming = false;
		this.active?.resolve(null);
		this.active = undefined;
		for (const request of this.pending.values()) request.resolve(null);
		this.pending.clear();
		this.retryAt = Date.now() + RETRY_AFTER_MS;
		this.publish({ status: "unavailable" });
	}
}

let client: BrowserEmotionClient | undefined;

export function getBotEmotionClient(): EmotionClient {
	client ??= new BrowserEmotionClient(
		() =>
			new Worker(new URL("./emotion.worker.ts", import.meta.url), {
				type: "module",
			}),
	);
	return client;
}
