import {
	type BoardFacts,
	type EdgeKind,
	type RankedCandidate,
	SUGGESTION_FORMAT_VERSION,
	type SuggestionContext,
	type SuggestionModel,
} from "./types";

const MODEL_FORMAT_VERSION = 1;

/** Dirichlet prior mass of each interpolation level. */
const BETA = 1;
/** Prior mass of each `pUnconnected` backoff level. */
const GAMMA = 2;
/** Smoothing of the bag lift `log(1 + co / (ALPHA · p(c)))`. */
const ALPHA = 5;
/** Candidates re-ranked by the log-linear features. */
const RERANK = 30;
/** Popular types appended to the candidate pool when the context has not seen them. */
const POPULAR_TAIL = 60;
/** Co-occurrence counts below this are dropped: they carry little signal and dominate the model size. */
const MIN_COOCCURRENCE = 2;
const NONE = -1;
const UNKNOWN = -2;

/**
 * Median of the per-fold weights of the best offline configuration (`all_nofilter`, 351 boards).
 * `pred1` weighs the fourth n-gram level as a log-lift over the interpolated trigram; `repeat`
 * rewards a follow-up of the anchor's own type (the ensemble ranks such a top-1 second).
 */
const WEIGHTS: Record<
	EdgeKind,
	{ pred1: number; ups: number; bag: number; repeat: number }
> = {
	exec: { pred1: 0.75, ups: 0.5, bag: 1, repeat: 0.5 },
	data: { pred1: 0.75, ups: 0.5, bag: 2, repeat: 0.5 },
};

interface Counts {
	counts: Map<number, number>;
	total: number;
}

type CountTable = Map<string, Counts>;
/** Symmetric `type → other → boards holding both`; the diagonal counts boards repeating the type. */
type Cooccurrence = Map<number, Map<number, number>>;

interface KindTables {
	popularity: Map<number, number>;
	popularityTotal: number;
	popular: number[];
	/** `(src)`, `(src, srcPin)`, `(pred0, src, srcPin)`, `(pred1, pred0, src, srcPin)` → dst. */
	levels: CountTable[];
	/** `(src, srcPin, upstream type)` → dst. */
	upstream: CountTable;
	/** `(src, srcPin, dst)` → dstPin. */
	pins: CountTable;
	/** `(dst)` → dstPin. */
	dstPins: CountTable;
	/** `()`, `(srcPin)`, `(src, srcPin)` → [unconnected, total]. */
	outcomes: Map<string, [number, number]>[];
}

interface SerializedKind {
	popularity: number[];
	levels: number[][][];
	upstream: number[][];
	pins: number[][];
	dstPins: number[][];
	outcomes: number[][][];
}

interface SerializedModel {
	version: number;
	factsVersion: number;
	types: string[];
	pins: string[];
	boards: number;
	boardCounts: number[];
	cooccurrence: number[][];
	kinds: Record<EdgeKind, SerializedKind>;
}

interface Candidate {
	type: number;
	p: number;
}

function emptyKind(): KindTables {
	return {
		popularity: new Map(),
		popularityTotal: 0,
		popular: [],
		levels: [new Map(), new Map(), new Map(), new Map()],
		upstream: new Map(),
		pins: new Map(),
		dstPins: new Map(),
		outcomes: [new Map(), new Map(), new Map()],
	};
}

function increment(table: CountTable, key: string, value: number): void {
	let entry = table.get(key);
	if (!entry) {
		entry = { counts: new Map(), total: 0 };
		table.set(key, entry);
	}
	entry.counts.set(value, (entry.counts.get(value) ?? 0) + 1);
	entry.total++;
}

function mostCommon(entry: Counts | undefined): number | undefined {
	if (!entry) return undefined;
	let best: number | undefined;
	let bestCount = 0;
	for (const [value, count] of entry.counts) {
		if (count > bestCount) {
			best = value;
			bestCount = count;
		}
	}
	return best;
}

function popularOrder(popularity: Map<number, number>): number[] {
	return [...popularity.entries()]
		.sort((a, b) => b[1] - a[1])
		.map(([type]) => type);
}

function writeCounts(table: CountTable): number[][] {
	const rows: number[][] = [];
	for (const [key, entry] of table) {
		const row = key.split(",").map(Number);
		for (const [value, count] of entry.counts) row.push(value, count);
		rows.push(row);
	}
	return rows;
}

function readCounts(rows: number[][], keyParts: number): CountTable {
	const table: CountTable = new Map();
	for (const row of rows) {
		const counts = new Map<number, number>();
		let total = 0;
		for (let index = keyParts; index + 1 < row.length; index += 2) {
			counts.set(row[index], row[index + 1]);
			total += row[index + 1];
		}
		table.set(row.slice(0, keyParts).join(","), { counts, total });
	}
	return table;
}

function writeOutcomes(table: Map<string, [number, number]>): number[][] {
	return [...table].map(([key, tally]) => [
		...(key === "" ? [] : key.split(",").map(Number)),
		...tally,
	]);
}

function readOutcomes(
	rows: number[][],
	keyParts: number,
): Map<string, [number, number]> {
	const table = new Map<string, [number, number]>();
	for (const row of rows) {
		table.set(row.slice(0, keyParts).join(","), [
			row[keyParts],
			row[keyParts + 1],
		]);
	}
	return table;
}

/** Rows `[type, other, count, …]` holding each symmetric pair once (`other >= type`), sorted. */
function writeCooccurrence(table: Cooccurrence): number[][] {
	const rows: number[][] = [];
	for (const type of [...table.keys()].sort((a, b) => a - b)) {
		const row = [type];
		const others = [...(table.get(type) ?? [])]
			.filter(([other]) => other >= type)
			.sort((a, b) => a[0] - b[0]);
		for (const [other, count] of others) row.push(other, count);
		if (row.length > 1) rows.push(row);
	}
	return rows;
}

function readCooccurrence(rows: number[][]): Cooccurrence {
	const table: Cooccurrence = new Map();
	for (const row of rows) {
		for (let index = 1; index + 1 < row.length; index += 2) {
			setPair(table, row[0], row[index], row[index + 1]);
		}
	}
	return table;
}

function setPair(
	table: Cooccurrence,
	type: number,
	other: number,
	count: number,
): void {
	for (const [from, to] of [
		[type, other],
		[other, type],
	]) {
		let row = table.get(from);
		if (!row) {
			row = new Map();
			table.set(from, row);
		}
		row.set(to, count);
	}
}

function firstAbove(sorted: number[], value: number): number {
	let low = 0;
	let high = sorted.length;
	while (low < high) {
		const middle = (low + high) >> 1;
		if (sorted[middle] <= value) low = middle + 1;
		else high = middle;
	}
	return low;
}

/**
 * Boards holding both types, accumulated one type at a time into a dense scratch row: the pair
 * count grows with the square of the distinct types per board, which nested maps cannot keep up with.
 */
function countCooccurrence(
	boardTypes: number[][],
	repeatedCounts: number[],
	size: number,
): Cooccurrence {
	const boardsOf: number[][] = Array.from({ length: size }, () => []);
	boardTypes.forEach((present, board) => {
		for (const type of present) boardsOf[type].push(board);
	});
	const table: Cooccurrence = new Map();
	const counts = new Int32Array(size);
	for (let type = 0; type < size; type++) {
		const touched: number[] = [];
		for (const board of boardsOf[type]) {
			const present = boardTypes[board];
			for (
				let index = firstAbove(present, type);
				index < present.length;
				index++
			) {
				if (counts[present[index]]++ === 0) touched.push(present[index]);
			}
		}
		for (const other of touched) {
			if (counts[other] >= MIN_COOCCURRENCE) {
				setPair(table, type, other, counts[other]);
			}
			counts[other] = 0;
		}
		if ((repeatedCounts[type] ?? 0) >= MIN_COOCCURRENCE) {
			setPair(table, type, type, repeatedCounts[type]);
		}
	}
	return table;
}

function backoff(tally: [number, number] | undefined, prior: number): number {
	return tally ? (tally[0] + GAMMA * prior) / (tally[1] + GAMMA) : prior;
}

class Interner {
	readonly values: string[] = [];
	readonly index = new Map<string, number>();

	intern(value: string): number {
		let id = this.index.get(value);
		if (id === undefined) {
			id = this.values.length;
			this.index.set(value, id);
			this.values.push(value);
		}
		return id;
	}
}

/**
 * Interpolated n-gram over `(pred1, pred0, src, srcPin) → dst` with a log-linear re-rank of its
 * top candidates by upstream types, board co-occurrence and the fourth n-gram level. Port of the
 * Python reference validated on 351 local boards (EXEC hit@1 .50, DATA .43).
 */
export class NgramModel implements SuggestionModel {
	private readonly typeIndex: Map<string, number>;
	private readonly pinIndex: Map<string, number>;

	private constructor(
		private readonly types: string[],
		private readonly pinNames: string[],
		private readonly boards: number,
		private readonly boardCounts: number[],
		private readonly cooccurrence: Cooccurrence,
		private readonly tables: Record<EdgeKind, KindTables>,
	) {
		this.typeIndex = new Map(types.map((type, index) => [type, index]));
		this.pinIndex = new Map(pinNames.map((pin, index) => [pin, index]));
	}

	static train(corpus: BoardFacts[]): NgramModel {
		const types = new Interner();
		const pins = new Interner();
		const tables: Record<EdgeKind, KindTables> = {
			exec: emptyKind(),
			data: emptyKind(),
		};
		const boardCounts: number[] = [];
		const repeatedCounts: number[] = [];
		const boardTypes: number[][] = [];

		for (const facts of corpus) {
			const local = facts.types.map((type) => types.intern(type));
			const typeAt = (index: number | undefined) =>
				index === undefined ? NONE : (local[index] ?? UNKNOWN);
			const seen = new Set<string>();
			const once = (key: string) => {
				if (seen.has(key)) return false;
				seen.add(key);
				return true;
			};

			for (const transition of facts.transitions) {
				const table = tables[transition.kind];
				if (!table) continue;
				const src = typeAt(transition.src);
				const dst = typeAt(transition.dst);
				const pin = pins.intern(transition.srcPin);
				const pred0 = typeAt(transition.pred[0]);
				const pred1 = typeAt(transition.pred[1]);
				const keys = [
					`${src}`,
					`${src},${pin}`,
					`${pred0},${src},${pin}`,
					`${pred1},${pred0},${src},${pin}`,
				];
				const scope = `${transition.kind}|`;
				if (once(`${scope}p|${dst}`)) {
					table.popularity.set(dst, (table.popularity.get(dst) ?? 0) + 1);
					table.popularityTotal++;
				}
				keys.forEach((key, level) => {
					if (once(`${scope}${level}|${key}|${dst}`)) {
						increment(table.levels[level], key, dst);
					}
				});
				for (const upstream of transition.ups) {
					const key = `${src},${pin},${typeAt(upstream)}`;
					if (once(`${scope}u|${key}|${dst}`)) {
						increment(table.upstream, key, dst);
					}
				}
				const dstPin = pins.intern(transition.dstPin);
				increment(table.pins, `${src},${pin},${dst}`, dstPin);
				increment(table.dstPins, `${dst}`, dstPin);
			}

			for (const outcome of facts.outcomes) {
				const table = tables[outcome.kind];
				if (!table) continue;
				const src = typeAt(outcome.src);
				const pin = pins.intern(outcome.srcPin);
				const unconnected = outcome.connected ? 0 : 1;
				["", `${pin}`, `${src},${pin}`].forEach((key, level) => {
					const tally = table.outcomes[level].get(key);
					if (tally) {
						tally[0] += unconnected;
						tally[1]++;
					} else {
						table.outcomes[level].set(key, [unconnected, 1]);
					}
				});
			}

			const present = [...new Set(local)].sort((a, b) => a - b);
			boardTypes.push(present);
			for (const type of present) {
				boardCounts[type] = (boardCounts[type] ?? 0) + 1;
			}
			for (const type of new Set(
				facts.repeated?.map((index) => local[index]),
			)) {
				if (type !== undefined) {
					repeatedCounts[type] = (repeatedCounts[type] ?? 0) + 1;
				}
			}
		}

		for (const table of Object.values(tables)) {
			table.popular = popularOrder(table.popularity);
		}
		return new NgramModel(
			types.values,
			pins.values,
			boardTypes.length,
			Array.from(types.values, (_, index) => boardCounts[index] ?? 0),
			countCooccurrence(boardTypes, repeatedCounts, types.values.length),
			tables,
		);
	}

	static deserialize(json: string): NgramModel {
		const data = JSON.parse(json) as SerializedModel;
		if (
			data?.version !== MODEL_FORMAT_VERSION ||
			data.factsVersion !== SUGGESTION_FORMAT_VERSION
		) {
			throw new Error(
				`Unsupported n-gram model format ${data?.version}/${data?.factsVersion}, expected ${MODEL_FORMAT_VERSION}/${SUGGESTION_FORMAT_VERSION}`,
			);
		}
		const readKind = (kind: SerializedKind): KindTables => {
			const popularity = new Map<number, number>();
			let popularityTotal = 0;
			for (let index = 0; index + 1 < kind.popularity.length; index += 2) {
				popularity.set(kind.popularity[index], kind.popularity[index + 1]);
				popularityTotal += kind.popularity[index + 1];
			}
			return {
				popularity,
				popularityTotal,
				popular: popularOrder(popularity),
				levels: kind.levels.map((rows, level) => readCounts(rows, level + 1)),
				upstream: readCounts(kind.upstream, 3),
				pins: readCounts(kind.pins, 3),
				dstPins: readCounts(kind.dstPins, 1),
				outcomes: kind.outcomes.map((rows, level) => readOutcomes(rows, level)),
			};
		};
		return new NgramModel(
			data.types,
			data.pins,
			data.boards,
			data.boardCounts,
			readCooccurrence(data.cooccurrence),
			{ exec: readKind(data.kinds.exec), data: readKind(data.kinds.data) },
		);
	}

	serialize(): string {
		const writeKind = (table: KindTables): SerializedKind => ({
			popularity: [...table.popularity].flat(),
			levels: table.levels.map(writeCounts),
			upstream: writeCounts(table.upstream),
			pins: writeCounts(table.pins),
			dstPins: writeCounts(table.dstPins),
			outcomes: table.outcomes.map(writeOutcomes),
		});
		const data: SerializedModel = {
			version: MODEL_FORMAT_VERSION,
			factsVersion: SUGGESTION_FORMAT_VERSION,
			types: this.types,
			pins: this.pinNames,
			boards: this.boards,
			boardCounts: this.boardCounts,
			cooccurrence: writeCooccurrence(this.cooccurrence),
			kinds: {
				exec: writeKind(this.tables.exec),
				data: writeKind(this.tables.data),
			},
		};
		return JSON.stringify(data);
	}

	/**
	 * The re-ranked top candidates share the interpolated mass of the top `RERANK` as a softmax of
	 * their log-linear scores; the tail keeps its interpolated probability, so the total stays ≤ 1.
	 */
	rank(context: SuggestionContext, limit: number): RankedCandidate[] {
		const table = this.tables[context.kind];
		if (!table || limit <= 0) return [];
		const src = this.typeId(context.src);
		const pin = this.pinId(context.srcPin);
		const pred0 = this.predId(context.pred?.[0]);
		const pred1 = this.predId(context.pred?.[1]);
		const chain = [
			table.levels[0].get(`${src}`),
			table.levels[1].get(`${src},${pin}`),
			table.levels[2].get(`${pred0},${src},${pin}`),
		];
		const pool = this.candidates(table, chain);
		if (pool.length === 0) return [];

		const top = pool.slice(0, RERANK);
		const weights = WEIGHTS[context.kind];
		const fourth = table.levels[3].get(`${pred1},${pred0},${src},${pin}`);
		const upstream = [...new Set(context.ups ?? [])]
			.map((type) => table.upstream.get(`${src},${pin},${this.typeId(type)}`))
			.filter((entry): entry is Counts => entry !== undefined);
		const bag = [...new Set(context.bag ?? [])];
		const bagIds = bag.map((type) => this.typeId(type));

		const scores = top.map(({ type, p }) => {
			const logP = Math.log(p);
			let score = logP;
			if (fourth) {
				score += weights.pred1 * (Math.log(smoothed(fourth, type, p)) - logP);
			}
			if (upstream.length > 0) {
				let mean = 0;
				for (const entry of upstream) mean += smoothed(entry, type, p);
				score += weights.ups * (Math.log(mean / upstream.length) - logP);
			}
			if (bag.length > 0) {
				score += (weights.bag * this.bagAffinity(type, bagIds)) / bag.length;
			}
			if (type === src) score += weights.repeat;
			return score;
		});

		const max = Math.max(...scores);
		let normalizer = 0;
		let mass = 0;
		for (let index = 0; index < top.length; index++) {
			normalizer += Math.exp(scores[index] - max);
			mass += top[index].p;
		}
		const ranked: RankedCandidate[] = top.map(({ type }, index) => ({
			type: this.types[type],
			probability: (mass * Math.exp(scores[index] - max)) / normalizer,
		}));
		for (let index = RERANK; index < pool.length; index++) {
			ranked.push({
				type: this.types[pool[index].type],
				probability: pool[index].p,
			});
		}
		return ranked.sort((a, b) => b.probability - a.probability).slice(0, limit);
	}

	/** Learned input pin of `dst` for this anchor pin, else the one `dst` is most often wired on. */
	predictPin(context: SuggestionContext, dst: string): string | null {
		const table = this.tables[context.kind];
		const target = this.typeIndex.get(dst);
		if (!table || target === undefined) return null;
		const src = this.typeId(context.src);
		const pin = this.pinId(context.srcPin);
		const best =
			mostCommon(table.pins.get(`${src},${pin},${target}`)) ??
			mostCommon(table.dstPins.get(`${target}`));
		return best === undefined ? null : this.pinNames[best];
	}

	/** Backoff over `kind → srcPin → (src, srcPin)` of how often such a pin stays unconnected. */
	pUnconnected(context: SuggestionContext): number {
		const table = this.tables[context.kind];
		if (!table) return 0.5;
		const src = this.typeId(context.src);
		const pin = this.pinId(context.srcPin);
		const [kind, byPin, bySrcPin] = table.outcomes;
		let p = backoff(kind.get(""), 0.5);
		p = backoff(byPin.get(`${pin}`), p);
		return backoff(bySrcPin.get(`${src},${pin}`), p);
	}

	private typeId(type: string | undefined): number {
		return type === undefined ? UNKNOWN : (this.typeIndex.get(type) ?? UNKNOWN);
	}

	private predId(type: string | undefined): number {
		return type === undefined ? NONE : this.typeId(type);
	}

	private pinId(pin: string | undefined): number {
		return pin === undefined ? UNKNOWN : (this.pinIndex.get(pin) ?? UNKNOWN);
	}

	private popularityPrior(table: KindTables, type: number): number {
		return (
			((table.popularity.get(type) ?? 0) + 1) /
			(table.popularityTotal + this.types.length)
		);
	}

	/** Every type seen after `src` plus the popular ones, by interpolated probability. */
	private candidates(
		table: KindTables,
		chain: (Counts | undefined)[],
	): Candidate[] {
		const pool: Candidate[] = [];
		const pooled = new Set<number>();
		const seen = chain[0];
		if (seen) {
			for (const type of seen.counts.keys()) {
				let p = this.popularityPrior(table, type);
				for (const level of chain) {
					if (level) p = smoothed(level, type, p);
				}
				pool.push({ type, p });
				pooled.add(type);
			}
		}
		let unseen = 1;
		for (const level of chain) {
			if (level) unseen *= BETA / (level.total + BETA);
		}
		let added = 0;
		for (const type of table.popular) {
			if (added >= POPULAR_TAIL) break;
			if (pooled.has(type)) continue;
			pool.push({ type, p: unseen * this.popularityPrior(table, type) });
			added++;
		}
		return pool.sort((a, b) => b.p - a.p);
	}

	/** Sum over bag types of `log(1 + co(t, c) / (ALPHA · p(c)))`, `p(c)` the share of boards holding `c`. */
	private bagAffinity(type: number, bag: number[]): number {
		const share = ((this.boardCounts[type] ?? 0) + 0.5) / (this.boards + 1);
		let sum = 0;
		for (const other of bag) {
			const count = this.cooccurrence.get(other)?.get(type);
			if (count) sum += Math.log1p(count / (ALPHA * share));
		}
		return sum;
	}
}

/** Dirichlet-smoothed `P(type | level)` with the lower-order estimate as prior. */
function smoothed(level: Counts, type: number, prior: number): number {
	return ((level.counts.get(type) ?? 0) + BETA * prior) / (level.total + BETA);
}
