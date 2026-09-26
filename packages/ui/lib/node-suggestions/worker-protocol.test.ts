import { describe, expect, spyOn, test } from "bun:test";
import { fileURLToPath } from "node:url";
import type { IBoard } from "../schema/flow/board";
import type { BoardFacts, SuggestionContext } from "./types";
import {
	type NodeSuggestionRequest,
	type NodeSuggestionResponse,
	NodeSuggestionWorkerCore,
} from "./worker-protocol";

function execFacts(chain: string[]): BoardFacts {
	return {
		types: chain,
		transitions: chain.slice(0, -1).map((_, index) => ({
			kind: "exec" as const,
			src: index,
			srcPin: "exec_out",
			dataType: "Execution",
			valueType: "Normal",
			schema: "",
			pred: index > 0 ? [index - 1] : [],
			ups: [],
			bag: index + 1,
			dst: index + 1,
			dstPin: "exec_in",
		})),
		outcomes: chain.map((_, index) => ({
			kind: "exec" as const,
			src: index,
			srcPin: "exec_out",
			connected: index < chain.length - 1,
		})),
	};
}

const CORPUS = Array.from({ length: 6 }, () =>
	JSON.stringify(execFacts(["events_simple", "log_info", "branch"])),
);

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

function harness() {
	const responses: NodeSuggestionResponse[] = [];
	const core = new NodeSuggestionWorkerCore((response) =>
		responses.push(response),
	);
	const send = async (request: NodeSuggestionRequest) => {
		await core.handle(request);
		return responses.filter((response) => response.id === request.id);
	};
	return { send };
}

describe("node-suggestion worker core", () => {
	test("suggest answers undefined until a model is loaded", async () => {
		const { send } = harness();
		expect(await send({ type: "suggest", id: 1, context: CONTEXT })).toEqual([
			{ type: "suggestion", id: 1, result: undefined },
		]);
	});

	test("trainNgram adopts the model and returns it serialized", async () => {
		const { send } = harness();
		const [trained] = await send({ type: "trainNgram", id: 1, corpus: CORPUS });
		expect(trained).toMatchObject({
			type: "trained",
			kind: "ngram",
			boards: 6,
			transitions: 12,
		});

		const [answer] = await send({ type: "suggest", id: 2, context: CONTEXT });
		expect(answer.type).toBe("suggestion");
		if (answer.type !== "suggestion") return;
		expect(answer.result?.candidates[0]).toMatchObject({
			type: "log_info",
			pin: "exec_in",
		});
	});

	test("load restores a serialized model and reports unreadable payloads", async () => {
		const trainer = harness();
		const [trained] = await trainer.send({
			type: "trainNgram",
			id: 1,
			corpus: CORPUS,
		});
		if (trained.type !== "trained") throw new Error("training failed");

		const { send } = harness();
		const warn = spyOn(console, "warn").mockImplementation(() => {});
		expect(
			await send({ type: "load", id: 1, ngram: "not json", neural: "nope" }),
		).toEqual([{ type: "loaded", id: 1, ngram: false, neural: false }]);
		warn.mockRestore();
		expect(await send({ type: "load", id: 2, ngram: trained.payload })).toEqual(
			[{ type: "loaded", id: 2, ngram: true, neural: false }],
		);
		const [answer] = await send({ type: "suggest", id: 3, context: CONTEXT });
		if (answer.type !== "suggestion") throw new Error(answer.type);
		expect(answer.result?.candidates[0]?.type).toBe("log_info");
	});

	test("trainNeural reports progress, adopts the model and can be cancelled", async () => {
		const catalog = ["events_simple", "log_info", "branch"].map((name) => ({
			name,
			friendlyName: name,
			category: "Test",
			description: "",
			pins: [],
		}));
		const { send } = harness();
		await send({ type: "trainNgram", id: 1, corpus: CORPUS });

		const responses = await send({
			type: "trainNeural",
			id: 2,
			corpus: CORPUS,
			catalog,
			warmStart: false,
		});
		expect(responses.some((response) => response.type === "progress")).toBe(
			true,
		);
		expect(responses.at(-1)).toMatchObject({ type: "trained", kind: "neural" });
		expect(
			await send({
				type: "load",
				id: 3,
			}),
		).toEqual([{ type: "loaded", id: 3, ngram: true, neural: true }]);

		const training = send({
			type: "trainNeural",
			id: 4,
			corpus: CORPUS,
			catalog,
			warmStart: true,
		});
		await send({ type: "cancelTraining", id: 5 });
		expect((await training).at(-1)).toEqual({ type: "cancelled", id: 4 });
	});

	test("corrupted corpus rows cost only their board", async () => {
		const { send } = harness();
		const [trained] = await send({
			type: "trainNgram",
			id: 1,
			corpus: ["{broken", ...CORPUS.slice(0, 2)],
		});
		expect(trained).toMatchObject({ type: "trained", boards: 2 });
	});

	test("extract reduces a board to facts", async () => {
		const pin = (
			id: string,
			name: string,
			pinType: "Input" | "Output",
			link: { connected_to?: string[]; depends_on?: string[] },
		) => ({
			id,
			name,
			friendly_name: name,
			description: "",
			pin_type: pinType,
			data_type: "Execution",
			value_type: "Normal",
			index: 0,
			connected_to: link.connected_to ?? [],
			depends_on: link.depends_on ?? [],
		});
		const board = {
			id: "board",
			nodes: {
				a: {
					id: "a",
					name: "events_simple",
					coordinates: [0, 0, 0],
					pins: {
						pa: pin("pa", "exec_out", "Output", { connected_to: ["pb"] }),
					},
				},
				b: {
					id: "b",
					name: "log_info",
					coordinates: [200, 0, 0],
					pins: { pb: pin("pb", "exec_in", "Input", { depends_on: ["pa"] }) },
				},
			},
			layers: {},
		} as unknown as IBoard;

		const [response] = await harness().send({ type: "extract", id: 1, board });

		expect(response.type).toBe("facts");
		if (response.type !== "facts") return;
		expect(response.facts.types).toEqual(["events_simple", "log_info"]);
		expect(response.facts.transitions).toHaveLength(1);
		expect(response.facts.transitions[0]).toMatchObject({
			kind: "exec",
			srcPin: "exec_out",
			dstPin: "exec_in",
		});
	});
});

test("the worker bundle does not reach the main-thread engine", async () => {
	const result = await Bun.build({
		entrypoints: [
			fileURLToPath(new URL("./node-suggestions.worker.ts", import.meta.url)),
		],
		target: "browser",
		format: "esm",
		metafile: true,
		throw: false,
	});

	expect(result.success).toBe(true);
	const inputs = Object.keys(result.metafile?.inputs ?? {}).map((path) =>
		path.replaceAll("\\", "/"),
	);
	expect(inputs.some((path) => path.endsWith("/worker-protocol.ts"))).toBe(
		true,
	);
	expect(inputs.filter((path) => path.endsWith("/engine.ts"))).toEqual([]);
	expect(inputs.filter((path) => path.includes("/state/"))).toEqual([]);
});
