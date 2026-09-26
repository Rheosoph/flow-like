import {
	type BoardFacts,
	type CatalogEntry,
	MAX_BAG,
	MAX_PRED,
	MAX_UPS,
	type SuggestionContext,
} from "../types";

export const DATA_TYPES = [
	"Execution",
	"String",
	"Integer",
	"Float",
	"Boolean",
	"Date",
	"PathBuf",
	"Generic",
	"Struct",
	"Byte",
	"Geometry",
] as const;
export const VALUE_TYPES = ["Normal", "Array", "HashMap", "HashSet"] as const;
/** Embedding rows: every known value plus one row for anything else. */
export const DATA_TYPE_ROWS = DATA_TYPES.length + 1;
export const VALUE_TYPE_ROWS = VALUE_TYPES.length + 1;

const TEXT_BUCKETS = 256;
/** Puts the unit-length text block on the scale of the standardized structural block. */
const TEXT_SCALE = 4;
const ROOTS = 24;
const SCHEMA_FEATURE_BUCKETS = 32;

const STOP_WORDS = new Set(
	`a an the of to and or for in on at by with from into onto as is are be been this that these those it its
if then else when while use used using uses returns return given value values input inputs output outputs node nodes
new get set data optional default true false via per each any all not no your you can will which specified provided`.split(
		/\s+/,
	),
);

export function tokenize(text: string): string[] {
	const tokens: string[] = [];
	const words = text
		.replace(/([a-z])([A-Z])/g, "$1 $2")
		.toLowerCase()
		.split(/[^a-z0-9]+/);
	for (const word of words) {
		if (word.length < 2 || STOP_WORDS.has(word) || /^\d+$/.test(word)) continue;
		tokens.push(
			word.length > 4 && word.endsWith("s") && !word.endsWith("ss")
				? word.slice(0, -1)
				: word,
		);
	}
	return tokens;
}

/** FNV-1a over UTF-16 code units; stable across runtimes, so hashed buckets survive serialization. */
export function hashString(text: string, seed = 0x811c9dc5): number {
	let hash = seed >>> 0;
	for (let index = 0; index < text.length; index++) {
		hash ^= text.charCodeAt(index);
		hash = Math.imul(hash, 0x01000193);
	}
	return hash >>> 0;
}

const SECOND_SEED = 0x9747b28c;

/** Two rows per string: a collision then needs both hashes to collide. */
export function hashRows(text: string, rows: number): [number, number] {
	return [hashString(text) % rows, hashString(text, SECOND_SEED) % rows];
}

function schemaRows(schema: string, rows: number): [number, number] {
	if (!schema) return [0, 0];
	const [first, second] = hashRows(schema, rows - 1);
	return [first + 1, second + 1];
}

/** Sparse per-type metadata matrix in CSR form; row 0 (`<pad>`) and non-catalog types are empty. */
export interface MetaFeatures {
	names: string[];
	rowStart: Int32Array;
	cols: Int32Array;
	values: Float32Array;
}

function addTokens(
	counts: Map<string, number>,
	text: string,
	weight: number,
): void {
	for (const token of tokenize(text)) {
		counts.set(token, (counts.get(token) ?? 0) + weight);
	}
}

function termCounts(entry: CatalogEntry): Map<string, number> {
	const counts = new Map<string, number>();
	addTokens(counts, entry.name.replaceAll("_", " "), 2);
	addTokens(counts, entry.friendlyName, 2);
	addTokens(counts, entry.category, 1);
	addTokens(counts, entry.description, 1);
	for (const pin of entry.pins) {
		if (pin.dataType !== "Execution") {
			addTokens(counts, pin.name.replaceAll("_", " "), 1);
		}
	}
	return counts;
}

function categoryRoot(category: string): string {
	return category.split("/")[0]?.trim() || "?";
}

const DENSE_NAMES = [
	...DATA_TYPES.map((type) => `in:${type}`),
	...DATA_TYPES.map((type) => `out:${type}`),
	"noExecIn",
	"execOut",
	"errorOut",
	"sink",
	"pinCount",
];

function denseStructure(entry: CatalogEntry): number[] {
	const dense = new Array<number>(DENSE_NAMES.length).fill(0);
	let execIn = 0;
	let execOut = 0;
	let errorOut = 0;
	let dataOut = 0;
	for (const pin of entry.pins) {
		const output = pin.pinType === "Output";
		const typeIndex = DATA_TYPES.indexOf(
			pin.dataType as (typeof DATA_TYPES)[number],
		);
		if (typeIndex >= 0) {
			dense[(output ? DATA_TYPES.length : 0) + typeIndex] += 1;
		}
		if (pin.dataType === "Execution") {
			if (output) {
				execOut++;
				if (pin.name.includes("error")) errorOut = 1;
			} else {
				execIn++;
			}
		} else if (output) {
			dataOut++;
		}
	}
	for (let index = 0; index < 2 * DATA_TYPES.length; index++) {
		dense[index] = Math.log1p(dense[index]);
	}
	const base = 2 * DATA_TYPES.length;
	dense[base] = execIn === 0 ? 1 : 0;
	dense[base + 1] = Math.log1p(execOut);
	dense[base + 2] = errorOut;
	dense[base + 3] = dataOut === 0 ? 1 : 0;
	dense[base + 4] = Math.log1p(entry.pins.length);
	return dense;
}

function uniqueEntries(catalog: CatalogEntry[]): Map<string, CatalogEntry> {
	const byName = new Map<string, CatalogEntry>();
	for (const entry of catalog) {
		if (!byName.has(entry.name)) byName.set(entry.name, entry);
	}
	return byName;
}

/**
 * Hashed TF-IDF text block (L2-normalized), category-root one-hot, standardized pin structure and
 * hashed schema titles. Sparse blocks stay uncentered so a row holds ~50 non-zeros instead of ~370;
 * the output table is rebuilt from this matrix on every training step.
 */
export function buildMeta(
	vocab: string[],
	catalog: CatalogEntry[],
): MetaFeatures {
	const byName = uniqueEntries(catalog);
	const entries = [...byName.values()];
	const docs = new Map<string, Map<string, number>>();
	const docFrequency = new Map<string, number>();
	const rootCounts = new Map<string, number>();
	for (const entry of entries) {
		const counts = termCounts(entry);
		docs.set(entry.name, counts);
		for (const term of counts.keys()) {
			docFrequency.set(term, (docFrequency.get(term) ?? 0) + 1);
		}
		const root = categoryRoot(entry.category);
		rootCounts.set(root, (rootCounts.get(root) ?? 0) + 1);
	}
	const roots = [...rootCounts.entries()]
		.sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1))
		.slice(0, ROOTS)
		.map(([root]) => root);

	const denseRows = new Map<string, number[]>();
	const denseMean = new Array<number>(DENSE_NAMES.length).fill(0);
	const denseSquare = new Array<number>(DENSE_NAMES.length).fill(0);
	for (const entry of entries) {
		const dense = denseStructure(entry);
		denseRows.set(entry.name, dense);
		for (let index = 0; index < dense.length; index++) {
			denseMean[index] += dense[index];
			denseSquare[index] += dense[index] * dense[index];
		}
	}
	const count = Math.max(1, entries.length);
	const denseStd = denseMean.map((sum, index) => {
		const mean = sum / count;
		denseMean[index] = mean;
		return (
			Math.sqrt(Math.max(0, denseSquare[index] / count - mean * mean)) + 1e-3
		);
	});

	const rootOffset = TEXT_BUCKETS;
	const denseOffset = rootOffset + roots.length;
	const schemaInOffset = denseOffset + DENSE_NAMES.length;
	const schemaOutOffset = schemaInOffset + SCHEMA_FEATURE_BUCKETS;
	const names = [
		...Array.from({ length: TEXT_BUCKETS }, (_, index) => `text:${index}`),
		...roots.map((root) => `root:${root}`),
		...DENSE_NAMES,
		...Array.from(
			{ length: SCHEMA_FEATURE_BUCKETS },
			(_, index) => `schemaIn:${index}`,
		),
		...Array.from(
			{ length: SCHEMA_FEATURE_BUCKETS },
			(_, index) => `schemaOut:${index}`,
		),
	];

	const rowStart = new Int32Array(vocab.length + 1);
	const cols: number[] = [];
	const values: number[] = [];
	const idfBase = entries.length + 1;
	for (let row = 0; row < vocab.length; row++) {
		rowStart[row] = cols.length;
		const entry = byName.get(vocab[row]);
		if (!entry) continue;
		const text = new Float64Array(TEXT_BUCKETS);
		for (const [term, termCount] of docs.get(entry.name) ?? []) {
			const frequency = docFrequency.get(term) ?? 0;
			if (frequency < 2) continue;
			text[hashString(term) % TEXT_BUCKETS] +=
				termCount * (Math.log(idfBase / (frequency + 1)) + 1);
		}
		let norm = 0;
		for (const value of text) norm += value * value;
		norm = Math.sqrt(norm);
		for (let index = 0; index < TEXT_BUCKETS; index++) {
			if (text[index] !== 0) {
				cols.push(index);
				values.push((text[index] / norm) * TEXT_SCALE);
			}
		}
		const rootIndex = roots.indexOf(categoryRoot(entry.category));
		if (rootIndex >= 0) {
			cols.push(rootOffset + rootIndex);
			values.push(1);
		}
		const dense = denseRows.get(entry.name) ?? [];
		for (let index = 0; index < dense.length; index++) {
			const value = (dense[index] - denseMean[index]) / denseStd[index];
			if (value !== 0) {
				cols.push(denseOffset + index);
				values.push(value);
			}
		}
		const schemaColumns = new Set<number>();
		for (const pin of entry.pins) {
			if (!pin.schema) continue;
			const offset =
				pin.pinType === "Output" ? schemaOutOffset : schemaInOffset;
			schemaColumns.add(
				offset + (hashString(pin.schema) % SCHEMA_FEATURE_BUCKETS),
			);
		}
		for (const column of [...schemaColumns].sort((a, b) => a - b)) {
			cols.push(column);
			values.push(1);
		}
	}
	rowStart[vocab.length] = cols.length;
	return {
		names,
		rowStart,
		cols: Int32Array.from(cols),
		values: Float32Array.from(values),
	};
}

export const PAD = "<pad>";

/** `<pad>`, the catalog in its own order, then corpus-only types (removed or foreign nodes) sorted. */
export function buildVocab(
	catalog: CatalogEntry[],
	corpus: BoardFacts[],
): string[] {
	const vocab = [PAD];
	const seen = new Set(vocab);
	for (const entry of catalog) {
		if (!seen.has(entry.name)) {
			seen.add(entry.name);
			vocab.push(entry.name);
		}
	}
	const extra = new Set<string>();
	for (const facts of corpus) {
		for (const type of facts.types) {
			if (!seen.has(type)) extra.add(type);
		}
	}
	vocab.push(...[...extra].sort());
	return vocab;
}

export interface EncoderConfig {
	pinBuckets: number;
	schemaBuckets: number;
}

/** A context as embedding-row indices; `-1` marks an absent or unknown type. */
export interface EncodedContext {
	kind: number;
	src: number;
	pins: [number, number];
	dataType: number;
	valueType: number;
	schemas: [number, number];
	pred0: number;
	pred1: number;
	predRest: number[];
	ups: number[];
	bag: number[];
}

function lookup(index: Map<string, number>, type: string | undefined): number {
	return type === undefined ? -1 : (index.get(type) ?? -1);
}

function knownTypes(
	index: Map<string, number>,
	types: readonly string[] | undefined,
	from: number,
	to: number,
): number[] {
	const out: number[] = [];
	if (!types) return out;
	for (let position = from; position < Math.min(to, types.length); position++) {
		const row = lookup(index, types[position]);
		if (row > 0) out.push(row);
	}
	return out;
}

export function encodeContext(
	context: SuggestionContext,
	index: Map<string, number>,
	config: EncoderConfig,
): EncodedContext {
	const dataType = DATA_TYPES.indexOf(
		context.dataType as (typeof DATA_TYPES)[number],
	);
	const valueType = VALUE_TYPES.indexOf(
		context.valueType as (typeof VALUE_TYPES)[number],
	);
	const pred = context.pred ?? [];
	const src = lookup(index, context.src);
	const pred0 = lookup(index, pred[0]);
	const pred1 = lookup(index, pred[1]);
	return {
		kind: context.kind === "exec" ? 0 : 1,
		src: src > 0 ? src : -1,
		pins: hashRows(context.srcPin ?? "", config.pinBuckets),
		dataType: dataType >= 0 ? dataType : DATA_TYPES.length,
		valueType: valueType >= 0 ? valueType : VALUE_TYPES.length,
		schemas: schemaRows(context.schema ?? "", config.schemaBuckets),
		pred0: pred0 > 0 ? pred0 : -1,
		pred1: pred1 > 0 ? pred1 : -1,
		predRest: knownTypes(index, pred, 2, MAX_PRED),
		ups: knownTypes(index, context.ups, 0, MAX_UPS),
		bag: [...new Set(knownTypes(index, context.bag, 0, MAX_BAG))],
	};
}
