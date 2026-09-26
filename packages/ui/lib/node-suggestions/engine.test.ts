import { describe, expect, spyOn, test } from "bun:test";
import { fileURLToPath } from "node:url";
import type { IApp, IBoard } from "../schema";
import type { IBoardSummary } from "../schema/flow/board-summary";
import {
	type SuggestionWorkerPort,
	catalogFingerprint,
	corpusFingerprint,
	createNodeSuggestionEngine,
	selectCorpus,
} from "./engine";
import { type SuggestionStore, createMemorySuggestionStore } from "./store";
import type { SyncBackend } from "./sync";
import type {
	BoardFacts,
	CatalogEntry,
	SuggestionContext,
	SuggestionResult,
} from "./types";
import type {
	NodeSuggestionRequest,
	NodeSuggestionResponse,
} from "./worker-protocol";

const SUB = "user";

const CONTEXT: SuggestionContext = {
	kind: "exec",
	src: "events_simple",
	srcPin: "exec_out",
	dataType: "Execution",
	valueType: "Normal",
	schema: "",
	pred: [],
	ups: [],
	bag: ["events_simple"],
};

const RESULT: SuggestionResult = {
	candidates: [{ type: "log_info", pin: "exec_in", probability: 0.9 }],
	pUnconnected: 0.1,
	confidence: 0.81,
};

const CATALOG: CatalogEntry[] = [
	{
		name: "log_info",
		friendlyName: "Log",
		category: "Utils",
		description: "",
		pins: [],
	},
];

/** A worker stand-in that records requests and answers only when told to. */
function manualPort() {
	const sent: NodeSuggestionRequest[] = [];
	let deliver: (response: NodeSuggestionResponse) => void = () => {};
	const port: SuggestionWorkerPort = {
		post: (request) => sent.push(request),
		listen: (onMessage) => {
			deliver = onMessage;
		},
		terminate: () => {},
	};
	const lastOf = <T extends NodeSuggestionRequest["type"]>(type: T) =>
		sent.filter((request) => request.type === type).at(-1) as Extract<
			NodeSuggestionRequest,
			{ type: T }
		>;
	return {
		port,
		sent,
		lastOf,
		reply: (response: NodeSuggestionResponse) => deliver(response),
	};
}

async function storeWithModel(): Promise<SuggestionStore> {
	const store = createMemorySuggestionStore();
	await store.putModel(SUB, "ngram", {
		fingerprint: "fp",
		trainedAt: 1,
		payload: "{}",
		boards: 4,
		transitions: 9,
	});
	return store;
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

async function loadedEngine() {
	const worker = manualPort();
	const engine = createNodeSuggestionEngine({
		sub: SUB,
		port: worker.port,
		store: storeWithModel(),
		schedule: () => {},
	});
	await flush();
	const load = worker.lastOf("load");
	worker.reply({ type: "loaded", id: load.id, ngram: true, neural: false });
	await engine.idle();
	return { engine, worker };
}

describe("node-suggestion engine", () => {
	test("suggest is undefined while no model is loaded", async () => {
		const worker = manualPort();
		const engine = createNodeSuggestionEngine({
			sub: SUB,
			port: worker.port,
			store: Promise.resolve(createMemorySuggestionStore()),
			schedule: () => {},
		});
		await engine.idle();

		expect(await engine.suggest(CONTEXT)).toBeUndefined();
		expect(worker.sent).toEqual([]);
		expect(engine.status.phase).toBe("idle");
	});

	test("cached models load at start-up", async () => {
		const { engine, worker } = await loadedEngine();

		expect(worker.lastOf("load")).toMatchObject({ ngram: "{}" });
		expect(engine.status).toMatchObject({
			phase: "ready",
			ngram: true,
			neural: false,
			boards: 4,
			transitions: 9,
		});
	});

	test("the latest suggest wins", async () => {
		const { engine, worker } = await loadedEngine();

		const first = engine.suggest(CONTEXT);
		const firstId = worker.lastOf("suggest").id;
		const second = engine.suggest({ ...CONTEXT, srcPin: "then" });
		const secondId = worker.lastOf("suggest").id;

		expect(await first).toBeUndefined();
		worker.reply({ type: "suggestion", id: secondId, result: RESULT });
		worker.reply({ type: "suggestion", id: firstId, result: RESULT });
		expect(await second).toEqual(RESULT);
	});

	test("status changes notify subscribers with a fresh snapshot", async () => {
		const worker = manualPort();
		const engine = createNodeSuggestionEngine({
			sub: SUB,
			port: worker.port,
			store: storeWithModel(),
			schedule: () => {},
		});
		const snapshots: unknown[] = [];
		const unsubscribe = engine.subscribe(() => snapshots.push(engine.status));
		await flush();
		worker.reply({
			type: "loaded",
			id: worker.lastOf("load").id,
			ngram: true,
			neural: false,
		});
		await engine.idle();
		unsubscribe();

		expect(snapshots.length).toBeGreaterThan(0);
		expect(new Set(snapshots).size).toBe(snapshots.length);
		engine.dispose();
		expect(engine.status.phase).toBe("disabled");
		expect(await engine.suggest(CONTEXT)).toBeUndefined();
	});

	test("a worker failure disables the engine", async () => {
		let fail: (error: unknown) => void = () => {};
		let terminated = false;
		const engine = createNodeSuggestionEngine({
			sub: SUB,
			port: {
				post: () => {},
				listen: (_onMessage, onError) => {
					fail = onError;
				},
				terminate: () => {
					terminated = true;
				},
			},
			store: storeWithModel(),
			schedule: () => {},
		});
		const warn = spyOn(console, "warn").mockImplementation(() => {});
		fail(new Error("crashed"));
		await engine.idle();
		warn.mockRestore();

		expect(terminated).toBe(true);
		expect(engine.status.phase).toBe("disabled");
		expect(await engine.suggest(CONTEXT)).toBeUndefined();
	});
});

describe("daily cycle", () => {
	const facts = (type: string): BoardFacts => ({
		types: ["events_simple", type],
		transitions: [],
		outcomes: [],
	});

	function backendFor(boards: Record<string, number>) {
		const fetched: string[] = [];
		const backend: SyncBackend = {
			appState: {
				getApps: async () => [
					[
						{ id: "app", updated_at: { secs_since_epoch: 1 } } as IApp,
						undefined,
					],
				],
			},
			boardState: {
				getBoardSummaries: async () =>
					Object.entries(boards).map(
						([id, secs]) =>
							({
								id,
								updatedAt: { secs_since_epoch: secs, nanos_since_epoch: 0 },
							}) as IBoardSummary,
					),
				getBoard: async (_appId, boardId) => {
					fetched.push(boardId);
					return { id: boardId } as IBoard;
				},
			},
		};
		return { backend, fetched };
	}

	/** Answers every request like the real worker would, with canned payloads. */
	function autoPort() {
		const sent: NodeSuggestionRequest[] = [];
		let deliver: (response: NodeSuggestionResponse) => void = () => {};
		const answer = (request: NodeSuggestionRequest): NodeSuggestionResponse => {
			switch (request.type) {
				case "extract":
					return {
						type: "facts",
						id: request.id,
						facts: facts((request.board as IBoard).id),
					};
				case "trainNgram":
				case "trainNeural":
					return {
						type: "trained",
						id: request.id,
						kind: request.type === "trainNgram" ? "ngram" : "neural",
						payload: `${request.type}:${request.corpus.length}`,
						boards: request.corpus.length,
						transitions: 0,
					};
				case "load":
					return {
						type: "loaded",
						id: request.id,
						ngram: request.ngram !== undefined,
						neural: request.neural !== undefined,
					};
				case "suggest":
					return { type: "suggestion", id: request.id, result: RESULT };
				case "cancelTraining":
					return { type: "cancelled", id: request.id };
			}
		};
		const port: SuggestionWorkerPort = {
			post: (request) => {
				sent.push(request);
				queueMicrotask(() => deliver(answer(request)));
			},
			listen: (onMessage) => {
				deliver = onMessage;
			},
			terminate: () => {},
		};
		return { port, sent };
	}

	test("syncs, trains n-gram then neural, persists both, and skips a second session the same day", async () => {
		const store = createMemorySuggestionStore();
		const { backend, fetched } = backendFor({ b1: 10, b2: 20 });
		const worker = autoPort();
		const engine = createNodeSuggestionEngine({
			sub: SUB,
			port: worker.port,
			store: Promise.resolve(store),
			schedule: (task) => task(),
			now: () => new Date(2026, 8, 23, 12),
		});
		engine.configure(backend, CATALOG);
		await engine.idle();

		expect(fetched.sort()).toEqual(["b1", "b2"]);
		expect(worker.sent.map((request) => request.type)).toEqual([
			"extract",
			"extract",
			"trainNgram",
			"trainNeural",
		]);
		expect(engine.status).toMatchObject({
			phase: "ready",
			ngram: true,
			neural: true,
			boards: 2,
			lastSyncDay: "2026-09-23",
		});
		expect((await store.getModel(SUB, "ngram"))?.payload).toBe("trainNgram:2");
		expect((await store.getModel(SUB, "neural"))?.payload).toBe(
			"trainNeural:2",
		);
		expect(await engine.suggest(CONTEXT)).toEqual(RESULT);
		engine.dispose();

		const next = autoPort();
		const again = createNodeSuggestionEngine({
			sub: SUB,
			port: next.port,
			store: Promise.resolve(store),
			schedule: (task) => task(),
			now: () => new Date(2026, 8, 23, 18),
		});
		again.configure(backend, CATALOG);
		await again.idle();

		expect(fetched).toHaveLength(2);
		expect(next.sent.map((request) => request.type)).toEqual(["load"]);
		expect(again.status).toMatchObject({ ngram: true, neural: true });
	});

	test("a catalog change retrains only the neural model", async () => {
		const store = createMemorySuggestionStore();
		const { backend } = backendFor({ b1: 10 });
		const first = autoPort();
		const engine = createNodeSuggestionEngine({
			sub: SUB,
			port: first.port,
			store: Promise.resolve(store),
			schedule: (task) => task(),
			now: () => new Date(2026, 8, 23, 12),
		});
		engine.configure(backend, CATALOG);
		await engine.idle();
		engine.dispose();

		const next = autoPort();
		const later = createNodeSuggestionEngine({
			sub: SUB,
			port: next.port,
			store: Promise.resolve(store),
			schedule: (task) => task(),
			now: () => new Date(2026, 8, 23, 13),
		});
		later.configure(backend, [...CATALOG, { ...CATALOG[0], name: "branch" }]);
		await later.idle();

		expect(next.sent.map((request) => request.type)).toEqual([
			"load",
			"trainNeural",
		]);
		expect(next.sent.at(-1)).toMatchObject({ warmStart: true });
	});
});

describe("fingerprints", () => {
	const board = (boardId: string, updatedAt: number) => ({
		appId: "app",
		boardId,
		updatedAt,
		transitions: 1,
	});

	test("the corpus fingerprint follows ids and updatedAt, not order", () => {
		const base = corpusFingerprint([board("a", 1), board("b", 2)]);
		expect(corpusFingerprint([board("b", 2), board("a", 1)])).toBe(base);
		expect(corpusFingerprint([board("a", 1), board("b", 3)])).not.toBe(base);
		expect(corpusFingerprint([board("a", 1)])).not.toBe(base);
	});

	test("the catalog fingerprint follows node names only", () => {
		const base = catalogFingerprint(CATALOG);
		expect(catalogFingerprint([{ ...CATALOG[0], description: "x" }])).toBe(
			base,
		);
		expect(catalogFingerprint([{ ...CATALOG[0], name: "other" }])).not.toBe(
			base,
		);
	});

	test("the corpus keeps the most recently changed boards", () => {
		const boards = [board("old", 1), board("new", 3), board("mid", 2)];
		expect(selectCorpus(boards).map((entry) => entry.boardId)).toEqual([
			"new",
			"mid",
			"old",
		]);
	});
});

test("the engine module does not bundle training code", async () => {
	const result = await Bun.build({
		entrypoints: [fileURLToPath(new URL("./engine.ts", import.meta.url))],
		target: "browser",
		format: "esm",
		metafile: true,
		packages: "external",
		throw: false,
	});

	expect(result.success).toBe(true);
	const inputs = Object.keys(result.metafile?.inputs ?? {}).map((path) =>
		path.replaceAll("\\", "/"),
	);
	expect(inputs.some((path) => path.endsWith("/sync.ts"))).toBe(true);
	for (const heavy of [
		"/ngram.ts",
		"/ensemble.ts",
		"/extract.ts",
		"/neural/model.ts",
		"/neural/mlp.ts",
		"/worker-protocol.ts",
	]) {
		expect(inputs.filter((path) => path.endsWith(heavy))).toEqual([]);
	}
});
