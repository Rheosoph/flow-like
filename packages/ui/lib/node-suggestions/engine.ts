/**
 * Main-thread side of the node-suggestion recommender. One engine per user `sub` owns the worker,
 * serves suggestions from the cached models right away and, once per session and calendar day in
 * idle time, syncs the facts corpus and retrains when it changed. Kept cheap to import: the models
 * and the extractor only exist inside the worker.
 */
import { useEffect, useMemo } from "react";
import { useBackend, useBackendReady } from "../../state/backend-state";
import type { IBoard } from "../schema/flow/board";
import type { INode } from "../schema/flow/node";
import { toCatalogEntries } from "./neural/catalog-entries";
import {
	type ModelKind,
	type StoredBoard,
	type StoredModel,
	type SuggestionStore,
	openSuggestionStore,
} from "./store";
import { type SyncBackend, localDay, syncCorpus } from "./sync";
import {
	type BoardFacts,
	type CatalogEntry,
	SUGGESTION_FORMAT_VERSION,
	type SuggestionContext,
	type SuggestionResult,
} from "./types";
import type {
	NodeSuggestionRequest,
	NodeSuggestionResponse,
} from "./worker-protocol";

/** Most recently changed boards the models learn from. */
export const MAX_CORPUS_BOARDS = 1000;
const STARTUP_GRACE_MS = 3_000;

export type NodeSuggestionPhase =
	| "idle"
	| "syncing"
	| "training"
	| "ready"
	| "disabled";

export interface NodeSuggestionStatus {
	phase: NodeSuggestionPhase;
	/** Boards in the training corpus. */
	boards: number;
	transitions: number;
	/** An n-gram model is loaded: `suggest` can answer. */
	ngram: boolean;
	neural: boolean;
	/** Day of the last sync run that finished every due app. */
	lastSyncDay: string | undefined;
}

export interface NodeSuggestionEngine {
	readonly status: NodeSuggestionStatus;
	/** For `useSyncExternalStore`; `status` is replaced, never mutated. */
	subscribe(listener: () => void): () => void;
	/**
	 * `undefined` while no model is loaded, and for a call superseded by a newer one before it was
	 * answered (latest wins).
	 */
	suggest(
		context: SuggestionContext,
		limit?: number,
	): Promise<SuggestionResult | undefined>;
}

export interface SuggestionWorkerPort {
	post(request: NodeSuggestionRequest): void;
	listen(
		onMessage: (response: NodeSuggestionResponse) => void,
		onError: (error: unknown) => void,
	): void;
	terminate(): void;
}

export interface NodeSuggestionEngineOptions {
	sub: string;
	port: SuggestionWorkerPort;
	store: Promise<SuggestionStore>;
	/** Runs the daily sync/train cycle; defaults to idle time after a start-up grace period. */
	schedule?: (task: () => void) => void;
	now?: () => Date;
}

export interface NodeSuggestionEngineHandle extends NodeSuggestionEngine {
	configure(backend: SyncBackend, catalog: CatalogEntry[]): void;
	/** Settles once the current cycle (if any) finished; for tests and diagnostics. */
	idle(): Promise<void>;
	dispose(): void;
}

type WithoutId<T> = T extends unknown ? Omit<T, "id"> : never;
type Response<K extends NodeSuggestionResponse["type"]> = Extract<
	NodeSuggestionResponse,
	{ type: K }
>;

interface PendingRequest {
	resolve: (response: NodeSuggestionResponse) => void;
}

function hash(text: string): string {
	let h1 = 0xdeadbeef;
	let h2 = 0x41c6ce57;
	for (let index = 0; index < text.length; index++) {
		const code = text.charCodeAt(index);
		h1 = Math.imul(h1 ^ code, 2654435761);
		h2 = Math.imul(h2 ^ code, 1597334677);
	}
	h1 =
		Math.imul(h1 ^ (h1 >>> 16), 2246822507) ^
		Math.imul(h2 ^ (h2 >>> 13), 3266489909);
	h2 =
		Math.imul(h2 ^ (h2 >>> 16), 2246822507) ^
		Math.imul(h1 ^ (h1 >>> 13), 3266489909);
	return (4294967296 * (2097151 & h2) + (h1 >>> 0)).toString(36);
}

export function selectCorpus(boards: readonly StoredBoard[]): StoredBoard[] {
	return [...boards]
		.sort(
			(left, right) =>
				right.updatedAt - left.updatedAt ||
				left.appId.localeCompare(right.appId) ||
				left.boardId.localeCompare(right.boardId),
		)
		.slice(0, MAX_CORPUS_BOARDS);
}

export function corpusFingerprint(boards: readonly StoredBoard[]): string {
	const keys = boards
		.map((board) => `${board.appId}/${board.boardId}@${board.updatedAt}`)
		.sort();
	return hash(`${SUGGESTION_FORMAT_VERSION}\n${keys.join("\n")}`);
}

export function catalogFingerprint(catalog: readonly CatalogEntry[]): string {
	return hash(
		catalog
			.map((entry) => entry.name)
			.sort()
			.join("\n"),
	);
}

function scheduleIdle(task: () => void): void {
	setTimeout(() => {
		if (typeof requestIdleCallback === "function") {
			requestIdleCallback(() => task(), { timeout: 10_000 });
		} else {
			task();
		}
	}, STARTUP_GRACE_MS);
}

function describe(response: NodeSuggestionResponse): string {
	return response.type === "error" ? response.message : response.type;
}

class SuggestionEngine implements NodeSuggestionEngineHandle {
	status: NodeSuggestionStatus = {
		phase: "idle",
		boards: 0,
		transitions: 0,
		ngram: false,
		neural: false,
		lastSyncDay: undefined,
	};
	private readonly listeners = new Set<() => void>();
	private readonly pending = new Map<number, PendingRequest>();
	private readonly controller = new AbortController();
	private readonly sub: string;
	private readonly port: SuggestionWorkerPort;
	private readonly store: Promise<SuggestionStore>;
	private readonly schedule: (task: () => void) => void;
	private readonly now: () => Date;
	private readonly loaded: Promise<void>;
	private seq = 0;
	private latest:
		| { resolve: (result: SuggestionResult | undefined) => void }
		| undefined;
	private backend: SyncBackend | undefined;
	private catalog: CatalogEntry[] = [];
	private cycleDay: string | undefined;
	private cycle: Promise<void> | undefined;
	private busy = false;
	private stopped = false;

	constructor(options: NodeSuggestionEngineOptions) {
		this.sub = options.sub;
		this.port = options.port;
		this.store = options.store;
		this.schedule = options.schedule ?? scheduleIdle;
		this.now = options.now ?? (() => new Date());
		this.port.listen(
			(response) => this.receive(response),
			(error) => this.fail(error),
		);
		this.loaded = this.loadCachedModels();
	}

	subscribe(listener: () => void): () => void {
		this.listeners.add(listener);
		return () => {
			this.listeners.delete(listener);
		};
	}

	configure(backend: SyncBackend, catalog: CatalogEntry[]): void {
		this.backend = backend;
		this.catalog = catalog;
		this.maybeScheduleCycle();
	}

	suggest(
		context: SuggestionContext,
		limit?: number,
	): Promise<SuggestionResult | undefined> {
		this.latest?.resolve(undefined);
		this.latest = undefined;
		// A long-lived editor session crosses midnight without re-configuring.
		this.maybeScheduleCycle();
		if (this.stopped || !this.status.ngram) return Promise.resolve(undefined);
		return new Promise((resolve) => {
			const ticket = { resolve };
			this.latest = ticket;
			void this.request({ type: "suggest", context, limit }).then(
				(response) => {
					if (this.latest !== ticket) return;
					this.latest = undefined;
					resolve(response.type === "suggestion" ? response.result : undefined);
				},
			);
		});
	}

	idle(): Promise<void> {
		return (this.cycle ?? this.loaded).then(() => undefined);
	}

	dispose(): void {
		if (this.stopped) return;
		this.stop();
		this.update({ phase: "disabled" });
	}

	private stop() {
		this.stopped = true;
		this.controller.abort();
		this.latest?.resolve(undefined);
		this.latest = undefined;
		for (const [id, entry] of this.pending) {
			entry.resolve({ type: "cancelled", id });
		}
		this.pending.clear();
		try {
			this.port.terminate();
		} catch {
			// Already gone.
		}
	}

	private fail(error: unknown) {
		if (this.stopped) return;
		console.warn(
			"[node-suggestions] worker failed; suggestions disabled:",
			error,
		);
		this.stop();
		this.update({ phase: "disabled", ngram: false, neural: false });
	}

	private update(patch: Partial<NodeSuggestionStatus>) {
		const next = { ...this.status, ...patch };
		if (
			(Object.keys(next) as (keyof NodeSuggestionStatus)[]).every(
				(key) => next[key] === this.status[key],
			)
		) {
			return;
		}
		this.status = next;
		for (const listener of this.listeners) listener();
	}

	private settledPhase(): NodeSuggestionPhase {
		if (this.stopped) return "disabled";
		return this.status.ngram ? "ready" : "idle";
	}

	private receive(response: NodeSuggestionResponse) {
		if (response.type === "progress") return;
		const entry = this.pending.get(response.id);
		if (!entry) return;
		this.pending.delete(response.id);
		entry.resolve(response);
	}

	private request(
		message: WithoutId<NodeSuggestionRequest>,
	): Promise<NodeSuggestionResponse> {
		const id = ++this.seq;
		if (this.stopped) return Promise.resolve({ type: "cancelled", id });
		return new Promise((resolve) => {
			this.pending.set(id, { resolve });
			try {
				this.port.post({ ...message, id } as NodeSuggestionRequest);
			} catch (error) {
				this.pending.delete(id);
				resolve({
					type: "error",
					id,
					message: error instanceof Error ? error.message : String(error),
				});
			}
		});
	}

	private async expect<K extends NodeSuggestionResponse["type"]>(
		message: WithoutId<NodeSuggestionRequest>,
		type: K,
	): Promise<Response<K>> {
		const response = await this.request(message);
		if (response.type !== type) {
			throw new Error(`${message.type} failed: ${describe(response)}`);
		}
		return response as Response<K>;
	}

	private async loadCachedModels() {
		try {
			const store = await this.store;
			const [ngram, neural, lastSyncDay] = await Promise.all([
				store.getModel(this.sub, "ngram").catch(() => undefined),
				store.getModel(this.sub, "neural").catch(() => undefined),
				store.getSyncDay(this.sub).catch(() => undefined),
			]);
			this.update({
				lastSyncDay,
				boards: ngram?.boards ?? 0,
				transitions: ngram?.transitions ?? 0,
			});
			if (!ngram && !neural) return;
			const loaded = await this.expect(
				{ type: "load", ngram: ngram?.payload, neural: neural?.payload },
				"loaded",
			);
			this.update({ ngram: loaded.ngram, neural: loaded.neural });
		} catch (error) {
			console.warn("[node-suggestions] loading cached models failed:", error);
		} finally {
			// The cycle awaits this load before it moves the phase, so nothing is overwritten.
			this.update({ phase: this.settledPhase() });
		}
	}

	private maybeScheduleCycle() {
		if (
			this.stopped ||
			this.busy ||
			!this.backend ||
			this.catalog.length === 0
		) {
			return;
		}
		const today = localDay(this.now());
		if (this.cycleDay === today) return;
		this.cycleDay = today;
		this.busy = true;
		this.cycle = new Promise<void>((resolve) => {
			this.schedule(() => {
				void this.runCycle(today).finally(resolve);
			});
		});
	}

	private async runCycle(today: string) {
		try {
			await this.loaded;
			const store = await this.store;
			const { signal } = this.controller;
			const backend = this.backend;
			if (signal.aborted || !backend) return;
			if ((await store.getSyncDay(this.sub)) !== today) {
				this.update({ phase: "syncing" });
				const summary = await syncCorpus({
					backend,
					sub: this.sub,
					today,
					store,
					extract: (board) => this.extract(board),
					signal,
				});
				if (signal.aborted) return;
				if (summary.complete) {
					await store.setSyncDay(this.sub, today);
					this.update({ lastSyncDay: today });
				}
			}
			await this.train(store, signal);
		} catch (error) {
			console.warn("[node-suggestions] learning cycle failed:", error);
		} finally {
			this.busy = false;
			this.update({ phase: this.settledPhase() });
		}
	}

	private async extract(board: IBoard): Promise<BoardFacts> {
		return (await this.expect({ type: "extract", board }, "facts")).facts;
	}

	private async train(store: SuggestionStore, signal: AbortSignal) {
		const boards = selectCorpus(await store.listBoards(this.sub));
		this.update({
			boards: boards.length,
			transitions: boards.reduce((sum, board) => sum + board.transitions, 0),
		});
		if (boards.length === 0) return;

		const catalog = this.catalog;
		const ngramFingerprint = corpusFingerprint(boards);
		const neuralFingerprint = hash(
			`${ngramFingerprint}|${catalogFingerprint(catalog)}`,
		);
		const [ngramRow, neuralRow] = await Promise.all([
			store.getModel(this.sub, "ngram"),
			store.getModel(this.sub, "neural"),
		]);
		const needsNgram =
			!this.status.ngram || ngramRow?.fingerprint !== ngramFingerprint;
		const needsNeural =
			!this.status.neural || neuralRow?.fingerprint !== neuralFingerprint;
		if (!needsNgram && !needsNeural) return;

		const corpus = await store.readFacts(this.sub, boards);
		if (signal.aborted || corpus.length === 0) return;
		this.update({ phase: "training" });

		if (needsNgram) {
			const trained = await this.expect(
				{ type: "trainNgram", corpus },
				"trained",
			);
			this.update({
				ngram: true,
				boards: trained.boards,
				transitions: trained.transitions,
			});
			await this.persist(store, "ngram", ngramFingerprint, trained);
		}
		if (!needsNeural || signal.aborted) return;
		const response = await this.request({
			type: "trainNeural",
			corpus,
			catalog,
			warmStart: this.status.neural,
		});
		if (response.type === "trained") {
			this.update({ neural: true });
			await this.persist(store, "neural", neuralFingerprint, response);
		} else if (response.type === "error") {
			console.warn(
				"[node-suggestions] neural training failed:",
				response.message,
			);
		}
	}

	private async persist(
		store: SuggestionStore,
		kind: ModelKind,
		fingerprint: string,
		trained: Response<"trained">,
	) {
		const model: StoredModel = {
			fingerprint,
			trainedAt: Date.now(),
			payload: trained.payload,
			boards: trained.boards,
			transitions: trained.transitions,
		};
		try {
			await store.putModel(this.sub, kind, model);
		} catch (error) {
			console.warn(
				`[node-suggestions] caching the ${kind} model failed:`,
				error,
			);
		}
	}
}

export function createNodeSuggestionEngine(
	options: NodeSuggestionEngineOptions,
): NodeSuggestionEngineHandle {
	return new SuggestionEngine(options);
}

function createWorkerPort(): SuggestionWorkerPort | undefined {
	if (typeof window === "undefined" || typeof Worker === "undefined") {
		return undefined;
	}
	let worker: Worker;
	try {
		worker = new Worker(
			new URL("./node-suggestions.worker.ts", import.meta.url),
			{ type: "module" },
		);
	} catch (error) {
		console.warn("[node-suggestions] worker unavailable:", error);
		return undefined;
	}
	return {
		post: (request) => worker.postMessage(request),
		listen: (onMessage, onError) => {
			worker.onmessage = (event: MessageEvent<NodeSuggestionResponse>) =>
				onMessage(event.data);
			worker.onerror = (event) => onError(event.message || event);
		},
		terminate: () => worker.terminate(),
	};
}

let active: { sub: string; engine: NodeSuggestionEngineHandle } | undefined;
let workerUnavailable = false;

function engineFor(sub: string): NodeSuggestionEngineHandle | undefined {
	if (active?.sub === sub) return active.engine;
	if (workerUnavailable) return undefined;
	active?.engine.dispose();
	active = undefined;
	const port = createWorkerPort();
	if (!port) {
		workerUnavailable = typeof window !== "undefined";
		return undefined;
	}
	const engine = createNodeSuggestionEngine({
		sub,
		port,
		store: openSuggestionStore(),
	});
	active = { sub, engine };
	return engine;
}

/** Stops the active engine (worker, sync, training), e.g. on sign-out. */
export function disposeNodeSuggestionEngine(): void {
	active?.engine.dispose();
	active = undefined;
}

const NO_CATALOG: CatalogEntry[] = [];
const catalogEntries = new WeakMap<readonly INode[], CatalogEntry[]>();

function catalogEntriesFor(catalog: INode[] | undefined): CatalogEntry[] {
	if (!catalog || catalog.length === 0) return NO_CATALOG;
	let entries = catalogEntries.get(catalog);
	if (!entries) {
		entries = toCatalogEntries(catalog);
		catalogEntries.set(catalog, entries);
	}
	return entries;
}

export interface UseNodeSuggestionEngineOptions {
	/** User the corpus and models belong to; `undefined` → `"local"`. */
	sub: string | undefined;
	catalog: INode[] | undefined;
	enabled: boolean;
}

/**
 * The recommender for the signed-in user, or `undefined` while disabled or where no Worker exists
 * (SSR, tests). The worker starts on the first enabled use; learning starts once the backend and a
 * catalog are available.
 */
export function useNodeSuggestionEngine({
	sub,
	catalog,
	enabled,
}: UseNodeSuggestionEngineOptions): NodeSuggestionEngine | undefined {
	const backend = useBackend();
	const backendReady = useBackendReady();
	const owner = sub ?? "local";
	const engine = useMemo(
		() => (enabled ? engineFor(owner) : undefined),
		[enabled, owner],
	);
	const entries = catalogEntriesFor(catalog);
	useEffect(() => {
		if (engine && backendReady && entries.length > 0) {
			engine.configure(backend, entries);
		}
	}, [engine, backend, backendReady, entries]);
	return engine;
}
