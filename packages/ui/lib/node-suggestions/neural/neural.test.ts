import { describe, expect, test } from "bun:test";
import type { INode } from "../../schema/flow/board";
import type {
	BoardFacts,
	CatalogEntry,
	FactTransition,
	SuggestionContext,
} from "../types";
import { toCatalogEntries } from "./catalog-entries";
import { buildMeta, encodeContext } from "./featurize";
import { fromHalfBits, toHalfBits } from "./half";
import { Network, createRandom, packExamples } from "./mlp";
import { NeuralModel, type NeuralTrainProgress } from "./model";

function entry(
	name: string,
	category: string,
	description: string,
	dataPins: string[] = [],
): CatalogEntry {
	return {
		name,
		friendlyName: name.replaceAll("_", " "),
		category,
		description,
		pins: [
			{
				name: "exec_in",
				pinType: "Input",
				dataType: "Execution",
				valueType: "Normal",
				schema: "",
			},
			{
				name: "exec_out",
				pinType: "Output",
				dataType: "Execution",
				valueType: "Normal",
				schema: "",
			},
			...dataPins.map((pin) => ({
				name: pin,
				pinType: "Input" as const,
				dataType: "String",
				valueType: "Normal",
				schema: "",
			})),
		],
	};
}

interface Step {
	type: string;
	pin: string;
}

/** A single exec chain as `BoardFacts`: every step is wired to the next one through `step.pin`. */
function chainBoard(steps: Step[]): BoardFacts {
	const types: string[] = [];
	const slot = (type: string) => {
		let index = types.indexOf(type);
		if (index < 0) index = types.push(type) - 1;
		return index;
	};
	const indices = steps.map((step) => slot(step.type));
	const firstSeen = steps.map(
		(_, position) => new Set(indices.slice(0, position + 1)).size,
	);
	const transitions: FactTransition[] = [];
	for (let position = 0; position + 1 < steps.length; position++) {
		transitions.push({
			kind: "exec",
			src: indices[position],
			srcPin: steps[position].pin,
			dataType: "Execution",
			valueType: "Normal",
			schema: "",
			pred: indices.slice(Math.max(0, position - 4), position).reverse(),
			ups: [],
			bag: firstSeen[position],
			dst: indices[position + 1],
			dstPin: "exec_in",
		});
	}
	return { types, transitions, outcomes: [] };
}

function context(
	src: string,
	srcPin = "exec_out",
	bag: string[] = [src],
): SuggestionContext {
	return {
		kind: "exec",
		src,
		srcPin,
		dataType: "Execution",
		valueType: "Normal",
		schema: "",
		pred: [],
		ups: [],
		bag,
	};
}

const TYPES = Array.from({ length: 10 }, (_, index) => `step_${index}`);
const onThen = (index: number) => (index * 3 + 1) % TYPES.length;
const onElse = (index: number) => (index * 7 + 4) % TYPES.length;

function patternCorpus(boards: number, seed = 1): BoardFacts[] {
	const random = createRandom(seed);
	return Array.from({ length: boards }, () => {
		let current = Math.floor(random() * TYPES.length);
		const steps: Step[] = [];
		for (let length = 0; length < 7; length++) {
			const pin = random() < 0.5 ? "then" : "else";
			steps.push({ type: TYPES[current], pin });
			current = pin === "then" ? onThen(current) : onElse(current);
		}
		steps.push({ type: TYPES[current], pin: "then" });
		return chainBoard(steps);
	});
}

const PATTERN_CATALOG = TYPES.map((type) =>
	entry(type, "Control/Steps", `Step ${type} of a process`),
);
const FAST = {
	d: 8,
	hidden: 16,
	batchSize: 16,
	learningRate: 1e-2,
	dropout: 0,
	patience: 100,
} as const;

describe("half precision", () => {
	test("round-trips representable values and rounds the rest", () => {
		for (const value of [0, 1, -2.5, 65504, 2 ** -24, 0.333251953125]) {
			expect(fromHalfBits(toHalfBits(value))).toBe(value);
		}
		expect(fromHalfBits(toHalfBits(1 + 2 ** -12))).toBe(1);
		expect(fromHalfBits(toHalfBits(1e6))).toBe(Number.POSITIVE_INFINITY);
		expect(Math.abs(fromHalfBits(toHalfBits(0.1)) - 0.1)).toBeLessThan(1e-4);
	});
});

describe("Network", () => {
	test("analytic gradients match finite differences", () => {
		const catalog = [
			entry("alpha_read", "IO/Files", "Reads a file from disk", ["path"]),
			entry("beta_write", "IO/Files", "Writes text into a file", [
				"path",
				"text",
			]),
			entry("gamma_parse", "Data/JSON", "Parses JSON text", ["text"]),
			entry("delta_fetch", "Web/HTTP", "Fetches a URL over HTTP", ["url"]),
			entry("epsilon_log", "Utils/Debug", "Logs a message", ["message"]),
		];
		const vocab = [
			"<pad>",
			...catalog.map((item) => item.name),
			"foreign_node",
		];
		const index = new Map(vocab.map((type, row) => [type, row]));
		const meta = buildMeta(vocab, catalog);
		const config = { d: 4, hidden: 8, pinBuckets: 16, schemaBuckets: 8 };
		const contexts: SuggestionContext[] = [
			{
				...context("alpha_read", "then", ["alpha_read", "gamma_parse"]),
				pred: ["delta_fetch", "epsilon_log", "gamma_parse"],
				ups: ["beta_write"],
			},
			{
				...context("gamma_parse", "result", ["gamma_parse"]),
				kind: "data",
				dataType: "Struct",
				schema: "Config",
				pred: ["alpha_read"],
			},
			{
				...context("foreign_node", "else", [
					"foreign_node",
					"delta_fetch",
					"epsilon_log",
				]),
				ups: ["alpha_read", "gamma_parse"],
			},
			{
				...context("delta_fetch"),
				valueType: "Array",
				pred: ["beta_write", "alpha_read"],
			},
		];
		const set = packExamples(
			contexts.map((item) => encodeContext(item, index, config)),
			[
				index.get("gamma_parse") ?? 0,
				index.get("epsilon_log") ?? 0,
				index.get("alpha_read") ?? 0,
				index.get("delta_fetch") ?? 0,
			],
			[1, 0.5, 1, 0.25],
		);
		const maskIn = Uint8Array.from(vocab, (_, row) =>
			row > 0 && row !== 2 ? 1 : 0,
		);
		const maskOut = Uint8Array.from(vocab, (_, row) =>
			row > 0 && row !== 5 ? 1 : 0,
		);
		const network = new Network(config, vocab.length, meta, maskIn, maskOut);
		const random = createRandom(7);
		network.initialize(random);
		for (const name of ["freeIn", "freeOut", "bFree"] as const) {
			const tensor = network.tensors[name];
			for (let item = 0; item < tensor.length; item++)
				tensor[item] = (random() - 0.5) * 0.6;
		}
		const order = [0, 1, 2, 3];
		network.beginBatch(set, order, 0, order.length);
		const totalWeight = 2.75;
		for (const i of order)
			network.trainExample(set, i, set.weight[i] / totalWeight, 0, null);
		network.endBatch();
		const analytic = network.gradient().slice();

		const epsilon = 1e-2;
		let checked = 0;
		let worst = 0;
		for (const [name, tensor] of Object.entries(network.tensors)) {
			const base = tensor.byteOffset / 4;
			const picks = new Set<number>();
			for (
				let attempt = 0;
				attempt < 24 && picks.size < Math.min(8, tensor.length);
				attempt++
			) {
				const item = Math.floor(random() * tensor.length);
				if (analytic[base + item] !== 0 || attempt > 16) picks.add(item);
			}
			for (const item of picks) {
				const original = tensor[item];
				tensor[item] = original + epsilon;
				const plus = network.batchLoss(set, order, 0, order.length);
				tensor[item] = original - epsilon;
				const minus = network.batchLoss(set, order, 0, order.length);
				tensor[item] = original;
				const numeric = (plus - minus) / (2 * epsilon);
				const expected = analytic[base + item];
				const error =
					Math.abs(numeric - expected) /
					(1e-3 + Math.abs(numeric) + Math.abs(expected));
				worst = Math.max(worst, error);
				if (error > 2e-2)
					throw new Error(
						`${name}[${item}]: numeric ${numeric} vs analytic ${expected}`,
					);
				checked++;
			}
		}
		expect(checked).toBeGreaterThan(100);
		expect(worst).toBeLessThan(2e-2);
	});
});

describe("NeuralModel", () => {
	test("learns a deterministic pin-dependent successor pattern", async () => {
		const model = await NeuralModel.train(patternCorpus(120), PATTERN_CATALOG, {
			...FAST,
			epochs: 25,
		});
		let correct = 0;
		for (let index = 0; index < TYPES.length; index++) {
			const then = model.rank(context(TYPES[index], "then"), 3);
			const otherwise = model.rank(context(TYPES[index], "else"), 3);
			if (then[0]?.type === TYPES[onThen(index)]) correct++;
			if (otherwise[0]?.type === TYPES[onElse(index)]) correct++;
		}
		expect(correct).toBeGreaterThanOrEqual(19);
	});

	test("scores a never-placed catalog node through its metadata", async () => {
		const catalog = [
			entry("http_get", "Web/HTTP", "Sends an HTTP GET request to a URL", [
				"url",
				"headers",
			]),
			entry(
				"http_post",
				"Web/HTTP",
				"Sends an HTTP POST request with a body to a URL",
				["url", "headers", "body"],
			),
			entry("json_parse", "Data/JSON", "Parses JSON text into a struct", [
				"text",
			]),
			entry("file_read", "IO/Files", "Reads a file from local storage", [
				"path",
			]),
			entry(
				"file_write",
				"IO/Files",
				"Writes content to a file in local storage",
				["path", "content"],
			),
			entry("text_split", "Data/Text", "Splits text by a separator", [
				"text",
				"separator",
			]),
			entry("log_info", "Utils/Debug", "Prints a debug message", ["message"]),
			entry("math_add", "Math/Arithmetic", "Adds two numbers", ["a", "b"]),
		];
		const random = createRandom(3);
		const corpus = Array.from({ length: 80 }, () => {
			const web = random() < 0.5;
			return chainBoard([
				{ type: "log_info", pin: "exec_out" },
				{ type: web ? "http_get" : "file_read", pin: "exec_out" },
				{ type: web ? "json_parse" : "text_split", pin: "exec_out" },
				{ type: random() < 0.5 ? "log_info" : "math_add", pin: "exec_out" },
			]);
		});
		const model = await NeuralModel.train(corpus, catalog, {
			...FAST,
			epochs: 40,
		});
		expect(model.rank(context("http_get"), 1)[0]?.type).toBe("json_parse");
		expect(model.rank(context("http_post"), 1)[0]?.type).toBe("json_parse");
		expect(model.rank(context("file_write"), 1)[0]?.type).toBe("text_split");
		const all = model.rank(context("http_post"), catalog.length);
		expect(all.map((item) => item.type)).toContain("http_post");
		expect(all.every((item) => item.probability > 0)).toBe(true);
	});

	test("serialize/deserialize round trip ranks identically", async () => {
		const model = await NeuralModel.train(patternCorpus(40), PATTERN_CATALOG, {
			...FAST,
			epochs: 5,
		});
		const json = model.serialize();
		const restored = NeuralModel.deserialize(json);
		for (const type of [...TYPES, "unknown_type"]) {
			for (const pin of ["then", "else"]) {
				const probe = {
					...context(type, pin, [type, TYPES[2]]),
					pred: [TYPES[1], "missing"],
				};
				expect(restored.rank(probe, 5)).toEqual(model.rank(probe, 5));
			}
		}
		expect(restored.serialize()).toBe(json);
		expect(() =>
			NeuralModel.deserialize(json.replace('"version":1', '"version":99')),
		).toThrow();
	});

	test("warm start maps weights by type name across a changed vocabulary", async () => {
		const model = await NeuralModel.train(patternCorpus(120), PATTERN_CATALOG, {
			...FAST,
			epochs: 25,
		});
		const changedCatalog = [
			entry("brand_new", "Control/Steps", "A step added to the catalog later"),
			...PATTERN_CATALOG.slice(1),
		];
		const restored = NeuralModel.deserialize(model.serialize());
		const warm = await NeuralModel.train(patternCorpus(40, 9), changedCatalog, {
			...FAST,
			epochs: 1,
			learningRate: 1e-9,
			warmStart: restored,
		});
		const cold = await NeuralModel.train(patternCorpus(40, 9), changedCatalog, {
			...FAST,
			epochs: 1,
			learningRate: 1e-9,
		});
		let agreeWarm = 0;
		let agreeCold = 0;
		for (let index = 0; index < TYPES.length; index++) {
			for (const pin of ["then", "else"]) {
				const probe = context(TYPES[index], pin);
				const expected = model.rank(probe, 1)[0]?.type;
				if (warm.rank(probe, 1)[0]?.type === expected) agreeWarm++;
				if (cold.rank(probe, 1)[0]?.type === expected) agreeCold++;
			}
		}
		expect(agreeWarm).toBeGreaterThanOrEqual(19);
		expect(agreeCold).toBeLessThan(agreeWarm);
		expect(warm.rank(context(TYPES[3]), 20).map((item) => item.type)).toContain(
			"brand_new",
		);
	});

	test("abort rejects training while the event loop keeps running", async () => {
		const controller = new AbortController();
		const progress: NeuralTrainProgress[] = [];
		let ticks = 0;
		const timer = setInterval(() => ticks++, 1);
		setTimeout(() => controller.abort(), 60);
		const started = performance.now();
		const training = NeuralModel.train(patternCorpus(600), PATTERN_CATALOG, {
			...FAST,
			d: 32,
			hidden: 64,
			epochs: 200,
			yieldEveryMs: 5,
			signal: controller.signal,
			onProgress: (item) => progress.push(item),
		});
		await expect(training).rejects.toMatchObject({ name: "AbortError" });
		clearInterval(timer);
		expect(performance.now() - started).toBeLessThan(2000);
		expect(ticks).toBeGreaterThan(3);
		expect(progress.length).toBeLessThan(200);
	});

	test("rank never throws on malformed input", async () => {
		const model = await NeuralModel.train(patternCorpus(20), PATTERN_CATALOG, {
			...FAST,
			epochs: 1,
		});
		expect(model.rank({} as SuggestionContext, 5)).toBeInstanceOf(Array);
		expect(model.rank(context(TYPES[0]), 0)).toEqual([]);
		expect(model.rank(context(TYPES[0]), Number.NaN)).toEqual([]);
		const ranked = model.rank(context("not_in_vocab"), 500);
		expect(ranked.length).toBe(TYPES.length);
		expect(ranked.reduce((sum, item) => sum + item.probability, 0)).toBeCloseTo(
			1,
			4,
		);
	});
});

describe("toCatalogEntries", () => {
	test("keeps the featurized slice and reads inline schema titles", () => {
		const node = {
			name: "parse_config",
			friendly_name: "Parse Config",
			category: "Data/Config",
			description: "Parses a config",
			id: "n1",
			pins: {
				b: {
					name: "config",
					pin_type: "Output",
					data_type: "Struct",
					value_type: "Normal",
					index: 2,
					schema: '{"title":"AppConfig","type":"object"}',
				},
				a: {
					name: "exec_in",
					pin_type: "Input",
					data_type: "Execution",
					value_type: "Normal",
					index: 1,
					schema: null,
				},
				c: {
					name: "raw",
					pin_type: "Input",
					data_type: "String",
					value_type: "Normal",
					index: 3,
					schema: "schema_ref_key",
				},
			},
		} as unknown as INode;
		expect(toCatalogEntries([node])).toEqual([
			{
				name: "parse_config",
				friendlyName: "Parse Config",
				category: "Data/Config",
				description: "Parses a config",
				pins: [
					{
						name: "exec_in",
						pinType: "Input",
						dataType: "Execution",
						valueType: "Normal",
						schema: "",
					},
					{
						name: "config",
						pinType: "Output",
						dataType: "Struct",
						valueType: "Normal",
						schema: "AppConfig",
					},
					{
						name: "raw",
						pinType: "Input",
						dataType: "String",
						valueType: "Normal",
						schema: "",
					},
				],
			},
		]);
	});
});
