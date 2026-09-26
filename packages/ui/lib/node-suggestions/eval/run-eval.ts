/**
 * App-split k-fold evaluation of the node-suggestion recommender over a folder of board JSONs.
 * Prints aggregate metrics only — never board contents.
 *
 *   bun packages/ui/lib/node-suggestions/eval/run-eval.ts <boards-dir> [--neural] [--folds 5]
 *       [--catalog <nodes.json>] [--seed 0] [--json <out.json>]
 *
 * `<boards-dir>` holds IBoard JSONs. The app of a board (the CV group) comes from an `index.json`
 * manifest (`[{ app, file }]`), else from `<app>/<board>.json` or `<app>__<board>.json` paths.
 * `--catalog` takes the catalog as INode[] or as the docs dump (camelCase pins); it feeds the
 * neural metadata features and enables the gate through `resolveGhosts`, i.e. the ghosts the
 * editor actually shows.
 */
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { basename, join, sep } from "node:path";
import { parseArgs } from "node:util";
import { GHOST_CANDIDATE_POOL } from "../../../hooks/use-node-suggestion-ghost";
import type { IBoard } from "../../schema/flow/board";
import {
	type INode,
	type IPin,
	IPinType,
	type IValueType,
	type IVariableType,
} from "../../schema/flow/node";
import {
	MODEL_POOL,
	mixRanked,
	rankSelfTypeSecond,
	suggest,
} from "../ensemble";
import {
	buildSuggestionContext,
	extractBoardFacts,
	outputPinTargets,
} from "../extract";
import { contextOf, expandTransition } from "../facts";
import { NeuralModel, toCatalogEntries } from "../neural";
import { NgramModel } from "../ngram";
import { resolveGhosts } from "../resolve";
import {
	type BoardFacts,
	type EdgeKind,
	GHOST_ALTERNATIVES,
	GHOST_MIN_CONFIDENCE,
	type SuggestionModel,
} from "../types";
import {
	type CrossFoldGate,
	type GateItem,
	type GatePoint,
	type RankMetrics,
	type RankTally,
	addRank,
	crossFoldThreshold,
	emptyTally,
	foldOf,
	gatePoint,
	rankMetrics,
	rankOf,
	thresholdForPrecision,
} from "./cv";

const KINDS: readonly EdgeKind[] = ["exec", "data"];
const ALPHAS = [0, 0.2, 0.3, 0.4, 0.5];
const PRECISION_TARGETS = [0.6, 0.7, 0.8];
const THRESHOLDS = [
	0.05, 0.1, 0.15, 0.2, 0.25, 0.3, 0.35, 0.4, 0.45, 0.5, 0.55, 0.6, 0.7, 0.8,
];
const TOP_LOST_TYPES = 12;

interface Models {
	ngram: NgramModel;
	neural?: SuggestionModel;
}

interface EvalBoard {
	app: string;
	fold: number;
	board: IBoard;
	facts: BoardFacts;
}

interface ResolveLoss {
	/** Pins whose top-1 type was right before `resolveGhosts`. */
	typeCorrect: number;
	/** …of which the shown ghost is a different type or none. */
	lost: number;
	lostTypes: Map<string, number>;
}

interface KindTallies {
	ranking: Map<string, RankTally>;
	joint: { n: number; shipped: number; ngram: number; pinGivenType: number };
	/** Gate variant → per-fold items. */
	gates: Map<string, GateItem[][]>;
	resolveLoss: ResolveLoss;
}

interface FoldTiming {
	fold: number;
	trainBoards: number;
	testBoards: number;
	ngramTrainMs: number;
	neuralTrainS?: number;
	neuralEpochs?: number;
	evalS: number;
}

interface GateReport {
	pins: number;
	correctIfAlwaysShown: number;
	grid: GatePoint[];
	atPrecision: Record<string, GatePoint | null>;
	crossFold: Record<string, CrossFoldGate | null>;
}

interface KindReport {
	ranking: Record<string, RankMetrics>;
	joint: {
		n: number;
		shippedTypeAndPin: number;
		ngramTypeAndPin: number;
		pinGivenTrueType: number;
	};
	gates: Record<string, GateReport>;
	resolveLoss?: {
		typeCorrect: number;
		lost: number;
		topLostCatalogTypes: [string, number][];
		lostNonCatalog: number;
	};
}

interface CliOptions {
	dir: string;
	folds: number;
	neural: boolean;
	catalog?: string;
	seed: number;
	json?: string;
}

function parseCli(argv: string[]): CliOptions {
	const { values, positionals } = parseArgs({
		args: argv,
		allowPositionals: true,
		options: {
			neural: { type: "boolean", default: false },
			folds: { type: "string", default: "5" },
			catalog: { type: "string" },
			seed: { type: "string", default: "0" },
			json: { type: "string" },
		},
	});
	const [dir] = positionals;
	const folds = Number(values.folds);
	const seed = Number(values.seed);
	if (!dir || !existsSync(dir)) {
		throw new Error(
			`run-eval: boards directory missing or not found: ${dir ?? "<none>"}`,
		);
	}
	if (!Number.isInteger(folds) || folds < 2) {
		throw new Error(
			`run-eval: --folds must be an integer >= 2, got ${values.folds}`,
		);
	}
	if (!Number.isFinite(seed)) {
		throw new Error(`run-eval: --seed must be a number, got ${values.seed}`);
	}
	return {
		dir,
		folds,
		neural: values.neural ?? false,
		catalog: values.catalog,
		seed,
		json: values.json,
	};
}

function appOf(file: string): string {
	const [first, ...rest] = file.split(sep);
	return rest.length > 0 ? first : basename(file, ".json").split("__")[0];
}

function boardFiles(dir: string): { app: string; file: string }[] {
	const manifest = join(dir, "index.json");
	if (existsSync(manifest)) {
		return JSON.parse(readFileSync(manifest, "utf8")) as {
			app: string;
			file: string;
		}[];
	}
	return readdirSync(dir, { recursive: true, encoding: "utf8" })
		.filter((file) => file.endsWith(".json"))
		.sort()
		.map((file) => ({ app: appOf(file), file }));
}

function readBoards(dir: string): { app: string; board: IBoard }[] {
	return boardFiles(dir).map(({ app, file }) => ({
		app,
		board: JSON.parse(readFileSync(join(dir, file), "utf8")) as IBoard,
	}));
}

interface DocsPin {
	name: string;
	friendlyName?: string;
	description?: string;
	pinType: string;
	dataType: string;
	valueType: string;
	schema?: string | null;
	index?: number;
	options?: {
		enforceSchema?: boolean;
		enforceGenericValueType?: boolean;
		validValues?: string[];
		range?: number[];
	} | null;
}

interface DocsNode {
	name: string;
	friendlyName?: string;
	description?: string;
	category?: string;
	pins: DocsPin[];
}

function docsPin(node: string, pin: DocsPin, position: number): IPin {
	return {
		id: `${node}_${position}`,
		name: pin.name,
		friendly_name: pin.friendlyName ?? pin.name,
		description: pin.description ?? "",
		pin_type: pin.pinType === "Output" ? IPinType.Output : IPinType.Input,
		data_type: pin.dataType as IVariableType,
		value_type: pin.valueType as IValueType,
		schema: pin.schema ?? null,
		index: pin.index ?? position,
		options: pin.options
			? {
					enforce_schema: pin.options.enforceSchema ?? null,
					enforce_generic_value_type:
						pin.options.enforceGenericValueType ?? null,
					valid_values: pin.options.validValues ?? null,
					range: pin.options.range ?? null,
				}
			: null,
		depends_on: [],
		connected_to: [],
	};
}

function docsNode(node: DocsNode): INode {
	return {
		id: node.name,
		name: node.name,
		friendly_name: node.friendlyName ?? node.name,
		description: node.description ?? "",
		category: node.category ?? "",
		pins: Object.fromEntries(
			node.pins.map((pin, position) => {
				const converted = docsPin(node.name, pin, position);
				return [converted.id, converted];
			}),
		),
	};
}

/** Catalog as `INode[]`, or the docs dump (pins as a camelCase array) mapped onto it. */
function readCatalog(path: string): INode[] {
	const entries = JSON.parse(readFileSync(path, "utf8")) as (
		| INode
		| DocsNode
	)[];
	if (!Array.isArray(entries)) {
		throw new Error(`run-eval: catalog ${path} is not a JSON array`);
	}
	return entries.map((entry) =>
		Array.isArray(entry.pins) ? docsNode(entry as DocsNode) : (entry as INode),
	);
}

function emptyKindTallies(): KindTallies {
	return {
		ranking: new Map(),
		joint: { n: 0, shipped: 0, ngram: 0, pinGivenType: 0 },
		gates: new Map(),
		resolveLoss: { typeCorrect: 0, lost: 0, lostTypes: new Map() },
	};
}

function tallyOf(tallies: KindTallies, name: string): RankTally {
	let tally = tallies.ranking.get(name);
	if (!tally) {
		tally = emptyTally();
		tallies.ranking.set(name, tally);
	}
	return tally;
}

function pushGate(
	tallies: KindTallies,
	variant: string,
	fold: number,
	folds: number,
	item: GateItem,
): void {
	let byFold = tallies.gates.get(variant);
	if (!byFold) {
		byFold = Array.from({ length: folds }, () => []);
		tallies.gates.set(variant, byFold);
	}
	byFold[fold].push(item);
}

const typesOf = (ranked: readonly { type: string }[]) =>
	ranked.map(({ type }) => type);
const pairTypes = (ranked: readonly [string, number][]) =>
	ranked.map(([type]) => type);

/** Every transition of a held-out board is one query. */
function evaluateTransitions(
	facts: BoardFacts,
	models: Models,
	tallies: Record<EdgeKind, KindTallies>,
): void {
	for (const transition of facts.transitions) {
		const fact = expandTransition(facts, transition);
		const context = contextOf(fact);
		const kind = tallies[fact.kind];
		const ngramRanked = models.ngram.rank(context, MODEL_POOL);
		addRank(tallyOf(kind, "ngram"), rankOf(typesOf(ngramRanked), fact.dst));

		const shipped = suggest(models, context, GHOST_ALTERNATIVES * 2);
		addRank(
			tallyOf(kind, "shipped"),
			rankOf(typesOf(shipped.candidates), fact.dst),
		);

		const top = shipped.candidates[0];
		const ngramTop = ngramRanked[0]?.type;
		kind.joint.n++;
		if (top?.type === fact.dst && top.pin === fact.dstPin) kind.joint.shipped++;
		if (
			ngramTop === fact.dst &&
			models.ngram.predictPin(context, ngramTop) === fact.dstPin
		)
			kind.joint.ngram++;
		if (models.ngram.predictPin(context, fact.dst) === fact.dstPin)
			kind.joint.pinGivenType++;

		if (!models.neural) continue;
		const neuralRanked = models.neural.rank(context, MODEL_POOL);
		addRank(tallyOf(kind, "neural"), rankOf(typesOf(neuralRanked), fact.dst));
		for (const alpha of ALPHAS) {
			const mixed = mixRanked(ngramRanked, neuralRanked, alpha);
			addRank(
				tallyOf(kind, `mix α=${alpha}`),
				rankOf(pairTypes(mixed), fact.dst),
			);
			addRank(
				tallyOf(kind, `mix α=${alpha} self-second`),
				rankOf(pairTypes(rankSelfTypeSecond(mixed, context.src)), fact.dst),
			);
		}
	}
}

/**
 * Every output pin of a held-out board is a would-be anchor, its live context built from the
 * finished board. A ghost is right when its type is among the pin's actual targets; an unconnected
 * pin makes every ghost wrong.
 */
function evaluateGate(
	entry: EvalBoard,
	variants: [string, Models][],
	catalogByName: ReadonlyMap<string, INode> | undefined,
	tallies: Record<EdgeKind, KindTallies>,
	folds: number,
): void {
	const { board, fold } = entry;
	const refs = board.refs ?? {};
	for (const { node, pin, kind, targets } of outputPinTargets(board)) {
		const context = buildSuggestionContext(board, node.id, pin.id);
		if (!context) continue;
		const kindTallies = tallies[kind];
		variants.forEach(([variant, models], position) => {
			const result = suggest(models, context, GHOST_CANDIDATE_POOL);
			const top = result.candidates[0]?.type;
			const typeCorrect = top !== undefined && targets.includes(top);
			pushGate(kindTallies, `${variant}/type`, fold, folds, {
				confidence: result.confidence,
				correct: typeCorrect,
			});
			if (!catalogByName) return;
			const ghost = resolveGhosts(
				result,
				{ node, pin },
				catalogByName,
				refs,
			)[0];
			const shown = ghost?.node.name;
			pushGate(kindTallies, `${variant}/resolved`, fold, folds, {
				confidence: ghost?.confidence ?? 0,
				correct: shown !== undefined && targets.includes(shown),
			});
			if (position !== variants.length - 1 || !typeCorrect || !top) return;
			const loss = kindTallies.resolveLoss;
			loss.typeCorrect++;
			if (shown === top) return;
			loss.lost++;
			loss.lostTypes.set(top, (loss.lostTypes.get(top) ?? 0) + 1);
		});
	}
}

function gateReport(byFold: GateItem[][]): GateReport {
	const all = byFold.flat();
	const atPrecision: Record<string, GatePoint | null> = {};
	const crossFold: Record<string, CrossFoldGate | null> = {};
	for (const target of PRECISION_TARGETS) {
		atPrecision[target] = thresholdForPrecision(all, target) ?? null;
		crossFold[target] = crossFoldThreshold(byFold, target) ?? null;
	}
	return {
		pins: all.length,
		correctIfAlwaysShown: all.filter((item) => item.correct).length,
		grid: [...new Set([...THRESHOLDS, GHOST_MIN_CONFIDENCE])]
			.sort((a, b) => a - b)
			.map((threshold) => gatePoint(all, threshold)),
		atPrecision,
		crossFold,
	};
}

function kindReport(
	tallies: KindTallies,
	catalogByName: ReadonlyMap<string, INode> | undefined,
): KindReport {
	const { joint, resolveLoss } = tallies;
	const share = (count: number) => (joint.n > 0 ? count / joint.n : 0);
	const report: KindReport = {
		ranking: Object.fromEntries(
			[...tallies.ranking].map(([name, tally]) => [name, rankMetrics(tally)]),
		),
		joint: {
			n: joint.n,
			shippedTypeAndPin: share(joint.shipped),
			ngramTypeAndPin: share(joint.ngram),
			pinGivenTrueType: share(joint.pinGivenType),
		},
		gates: Object.fromEntries(
			[...tallies.gates].map(([variant, byFold]) => [
				variant,
				gateReport(byFold),
			]),
		),
	};
	if (catalogByName) {
		const lost = [...resolveLoss.lostTypes].sort((a, b) => b[1] - a[1]);
		report.resolveLoss = {
			typeCorrect: resolveLoss.typeCorrect,
			lost: resolveLoss.lost,
			topLostCatalogTypes: lost
				.filter(([type]) => catalogByName.has(type))
				.slice(0, TOP_LOST_TYPES),
			lostNonCatalog: lost
				.filter(([type]) => !catalogByName.has(type))
				.reduce((sum, [, count]) => sum + count, 0),
		};
	}
	return report;
}

async function trainNeural(
	train: BoardFacts[],
	catalog: INode[],
	seed: number,
): Promise<{ model: NeuralModel; seconds: number; epochs: number }> {
	const started = performance.now();
	let epochs = 0;
	const model = await NeuralModel.train(train, toCatalogEntries(catalog), {
		seed,
		onProgress: (progress) => {
			epochs = progress.epoch;
		},
	});
	return { model, seconds: (performance.now() - started) / 1000, epochs };
}

const pct = (value: number) => `${(value * 100).toFixed(1)}%`;
const num = (value: number, digits = 3) => value.toFixed(digits);

function printTable(title: string, rows: string[][]): void {
	const widths = rows[0].map((_, column) =>
		Math.max(...rows.map((row) => row[column]?.length ?? 0)),
	);
	console.log(`\n${title}`);
	for (const row of rows) {
		console.log(
			`  ${row.map((cell, column) => cell.padEnd(widths[column])).join("  ")}`,
		);
	}
}

function printRanking(reports: Record<EdgeKind, KindReport>): void {
	const names = Object.keys(reports.exec.ranking).filter(
		(name) => !name.startsWith("mix"),
	);
	const header = ["model", "kind", "n", "hit@1", "hit@3", "hit@5", "MRR@10"];
	const rows = names.flatMap((name) =>
		KINDS.map((kind) => {
			const metrics = reports[kind].ranking[name];
			return [
				name,
				kind.toUpperCase(),
				String(metrics.n),
				num(metrics.hit1),
				num(metrics.hit3),
				num(metrics.hit5),
				num(metrics.mrr10),
			];
		}),
	);
	printTable("Ranking (every transition of held-out boards)", [
		header,
		...rows,
	]);
}

function printAlphaGrid(reports: Record<EdgeKind, KindReport>): void {
	const rows = [
		[
			"α (neural weight)",
			"EXEC h@1/h@3/h@5/MRR",
			"DATA h@1/h@3/h@5/MRR",
			"EXEC h@1 self-second",
			"DATA h@1 self-second",
		],
	];
	const cell = (metrics: RankMetrics | undefined) =>
		metrics
			? [metrics.hit1, metrics.hit3, metrics.hit5, metrics.mrr10]
					.map((value) => num(value))
					.join(" / ")
			: "-";
	for (const alpha of ALPHAS) {
		const plain = (kind: EdgeKind) => reports[kind].ranking[`mix α=${alpha}`];
		const swapped = (kind: EdgeKind) =>
			reports[kind].ranking[`mix α=${alpha} self-second`];
		rows.push([
			String(alpha),
			cell(plain("exec")),
			cell(plain("data")),
			num(swapped("exec")?.hit1 ?? 0),
			num(swapped("data")?.hit1 ?? 0),
		]);
	}
	printTable(
		"Geometric mixture α grid (union of both top-60 lists, no self-type swap unless noted)",
		rows,
	);
}

function printJoint(reports: Record<EdgeKind, KindReport>): void {
	printTable("Joint type + target pin top-1", [
		["kind", "shipped type+pin", "n-gram type+pin", "pin | true type"],
		...KINDS.map((kind) => {
			const { joint } = reports[kind];
			return [
				kind.toUpperCase(),
				num(joint.shippedTypeAndPin),
				num(joint.ngramTypeAndPin),
				num(joint.pinGivenTrueType),
			];
		}),
	]);
}

function printGates(reports: Record<EdgeKind, KindReport>): void {
	for (const variant of Object.keys(reports.exec.gates)) {
		const header = ["threshold"];
		for (const kind of KINDS) {
			header.push(`${kind.toUpperCase()} cov`, `${kind.toUpperCase()} prec`);
		}
		const grids = KINDS.map((kind) => reports[kind].gates[variant].grid);
		const rows = grids[0].map((point, index) => [
			num(point.threshold, 2) +
				(point.threshold === GHOST_MIN_CONFIDENCE ? " (current)" : ""),
			...grids.flatMap((grid) => [
				pct(grid[index].coverage),
				pct(grid[index].precision),
			]),
		]);
		const summary = KINDS.map((kind) => {
			const gate = reports[kind].gates[variant];
			return `${kind.toUpperCase()}: ${gate.pins} pins, ${pct(gate.correctIfAlwaysShown / Math.max(gate.pins, 1))} ghost-right when always shown`;
		}).join("; ");
		printTable(`Pin gate ${variant} — ${summary}`, [header, ...rows]);

		const targetRows = [
			[
				"precision",
				"kind",
				"pooled threshold",
				"pooled coverage",
				"cross-fold coverage",
				"cross-fold precision (min-max)",
				"per-fold thresholds",
			],
		];
		for (const target of PRECISION_TARGETS) {
			for (const kind of KINDS) {
				const gate = reports[kind].gates[variant];
				const pooled = gate.atPrecision[target];
				const honest = gate.crossFold[target];
				targetRows.push([
					pct(target),
					kind.toUpperCase(),
					pooled ? num(pooled.threshold) : "-",
					pooled ? pct(pooled.coverage) : "-",
					honest ? pct(honest.coverage) : "-",
					honest
						? `${pct(honest.precision)} (${pct(honest.precisionRange[0])}-${pct(honest.precisionRange[1])})`
						: "-",
					honest
						? honest.thresholds.map((value) => num(value, 2)).join(" ")
						: "-",
				]);
			}
		}
		printTable(
			`Thresholds reaching a precision target — ${variant}`,
			targetRows,
		);
	}
}

function printResolveLoss(reports: Record<EdgeKind, KindReport>): void {
	for (const kind of KINDS) {
		const loss = reports[kind].resolveLoss;
		if (!loss) continue;
		console.log(
			`\nresolveGhosts on right top-1 types (${kind.toUpperCase()}): ${loss.lost} of ${loss.typeCorrect} (${pct(loss.lost / Math.max(loss.typeCorrect, 1))}) shown as another type or not at all; ${loss.lostNonCatalog} of those are not in the catalog`,
		);
		for (const [type, count] of loss.topLostCatalogTypes) {
			console.log(`  ${type}: ${count}`);
		}
	}
}

async function main(): Promise<void> {
	const options = parseCli(process.argv.slice(2));
	const loaded = readBoards(options.dir);
	const extractStart = performance.now();
	const boards: EvalBoard[] = loaded.map(({ app, board }) => ({
		app,
		fold: foldOf(app, options.folds),
		board,
		facts: extractBoardFacts(board),
	}));
	const extractMs = performance.now() - extractStart;
	const catalog = options.catalog ? readCatalog(options.catalog) : undefined;
	const catalogByName = catalog
		? new Map(catalog.map((node) => [node.name, node]))
		: undefined;
	if (options.neural && !catalog) {
		console.warn(
			"run-eval: --neural without --catalog trains without catalog metadata features",
		);
	}

	const transitions = { exec: 0, data: 0 };
	for (const { facts } of boards) {
		for (const transition of facts.transitions) transitions[transition.kind]++;
	}
	console.log(
		`boards=${boards.length} apps=${new Set(boards.map(({ app }) => app)).size} non-empty=${boards.filter(({ facts }) => facts.types.length > 0).length} transitions exec=${transitions.exec} data=${transitions.data} catalog=${catalog?.length ?? 0} folds=${options.folds}`,
	);
	console.log(
		`extract: ${extractMs.toFixed(0)} ms for ${boards.length} boards (${(extractMs / Math.max(boards.length, 1)).toFixed(2)} ms/board)`,
	);

	const tallies: Record<EdgeKind, KindTallies> = {
		exec: emptyKindTallies(),
		data: emptyKindTallies(),
	};
	const timings: FoldTiming[] = [];
	for (let fold = 0; fold < options.folds; fold++) {
		const train = boards.filter((entry) => entry.fold !== fold);
		const test = boards.filter((entry) => entry.fold === fold);
		const trainFacts = train.map(({ facts }) => facts);
		const ngramStart = performance.now();
		const ngram = NgramModel.train(trainFacts);
		const timing: FoldTiming = {
			fold,
			trainBoards: train.length,
			testBoards: test.length,
			ngramTrainMs: performance.now() - ngramStart,
			evalS: 0,
		};
		let neural: NeuralModel | undefined;
		if (options.neural) {
			const trained = await trainNeural(
				trainFacts,
				catalog ?? [],
				options.seed,
			);
			neural = trained.model;
			timing.neuralTrainS = trained.seconds;
			timing.neuralEpochs = trained.epochs;
		}
		const models: Models = { ngram, neural };
		const variants: [string, Models][] = neural
			? [
					["ngram", { ngram }],
					["ensemble", models],
				]
			: [["ngram", { ngram }]];
		const evalStart = performance.now();
		for (const entry of test) {
			evaluateTransitions(entry.facts, models, tallies);
			evaluateGate(entry, variants, catalogByName, tallies, options.folds);
		}
		timing.evalS = (performance.now() - evalStart) / 1000;
		timings.push(timing);
		console.log(
			`fold ${fold}: train=${timing.trainBoards} test=${timing.testBoards} ngram_train=${timing.ngramTrainMs.toFixed(0)}ms${timing.neuralTrainS === undefined ? "" : ` neural_train=${timing.neuralTrainS.toFixed(1)}s epochs=${timing.neuralEpochs}`} eval=${timing.evalS.toFixed(1)}s`,
		);
	}

	const reports: Record<EdgeKind, KindReport> = {
		exec: kindReport(tallies.exec, catalogByName),
		data: kindReport(tallies.data, catalogByName),
	};
	printRanking(reports);
	if (options.neural) printAlphaGrid(reports);
	printJoint(reports);
	printGates(reports);
	printResolveLoss(reports);

	if (options.json) {
		writeFileSync(
			options.json,
			JSON.stringify(
				{
					boards: boards.length,
					folds: options.folds,
					neural: options.neural,
					extractMs,
					timings,
					exec: reports.exec,
					data: reports.data,
				},
				null,
				1,
			),
		);
	}
}

await main();
