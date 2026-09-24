import {
	DATA_TYPE_ROWS,
	type EncodedContext,
	type MetaFeatures,
	VALUE_TYPE_ROWS,
} from "./featurize";

export interface NetworkConfig {
	/** Embedding width; a multiple of 4 (the hot loops are unrolled by 4). */
	d: number;
	hidden: number;
	pinBuckets: number;
	schemaBuckets: number;
}

/** Context slots concatenated into the MLP input: source, pred0, pred1, mean(pred2..), mean(ups), mean(bag). */
const SLOTS = 6;

export const DENSE_TENSORS = [
	"pin",
	"dataType",
	"valueType",
	"schema",
	"kind",
	"w1",
	"b1",
	"w2",
	"b2",
	"lnGain",
	"lnBias",
	"wHead",
	"bHead",
	"wCopy",
	"bCopy",
] as const;
export const META_TENSORS = ["pIn", "pOut", "wB"] as const;
const FREE_TENSORS = ["freeIn", "freeOut", "bFree"] as const;
/** Per-type tables a trained model serves from: `typeIn`, `typeOut`, `typeBias` with masks and metadata folded in. */
export const TABLE_TENSORS = ["typeIn", "typeOut", "typeBias"] as const;

export type DenseTensor = (typeof DENSE_TENSORS)[number];
export type MetaTensor = (typeof META_TENSORS)[number];
type FreeTensor = (typeof FREE_TENSORS)[number];
export type TableTensor = (typeof TABLE_TENSORS)[number];
type TrainTensor = DenseTensor | MetaTensor | FreeTensor;
export type ModelTensor = DenseTensor | MetaTensor | TableTensor;

export function tensorSizes(
	config: NetworkConfig,
	vocabSize: number,
	featureCount: number,
): Record<DenseTensor | MetaTensor | FreeTensor | TableTensor, number> {
	const { d, hidden } = config;
	return {
		pin: config.pinBuckets * d,
		dataType: DATA_TYPE_ROWS * d,
		valueType: VALUE_TYPE_ROWS * d,
		schema: config.schemaBuckets * d,
		kind: 2 * d,
		w1: hidden * SLOTS * d,
		b1: hidden,
		w2: d * hidden,
		b2: d,
		lnGain: d,
		lnBias: d,
		wHead: d * d,
		bHead: d,
		wCopy: 2 * d,
		bCopy: 2,
		pIn: featureCount * d,
		pOut: featureCount * d,
		wB: featureCount,
		freeIn: vocabSize * d,
		freeOut: vocabSize * d,
		bFree: vocabSize,
		typeIn: vocabSize * d,
		typeOut: vocabSize * d,
		typeBias: vocabSize,
	};
}

/** Contexts packed into flat arrays so training touches no per-example objects. */
export interface ExampleSet {
	count: number;
	kind: Uint8Array;
	src: Int32Array;
	pins: Int32Array;
	dataType: Uint8Array;
	valueType: Uint8Array;
	schemas: Int32Array;
	pred0: Int32Array;
	pred1: Int32Array;
	predStart: Int32Array;
	predItems: Int32Array;
	upsStart: Int32Array;
	upsItems: Int32Array;
	bagStart: Int32Array;
	bagItems: Int32Array;
	dst: Int32Array;
	weight: Float32Array;
}

function packLists(
	encoded: EncodedContext[],
	pick: (context: EncodedContext) => number[],
): { start: Int32Array; items: Int32Array } {
	const start = new Int32Array(encoded.length + 1);
	let total = 0;
	for (let index = 0; index < encoded.length; index++) {
		start[index] = total;
		total += pick(encoded[index]).length;
	}
	start[encoded.length] = total;
	const items = new Int32Array(total);
	for (let index = 0; index < encoded.length; index++) {
		items.set(pick(encoded[index]), start[index]);
	}
	return { start, items };
}

export function packExamples(
	encoded: EncodedContext[],
	dst?: ArrayLike<number>,
	weight?: ArrayLike<number>,
): ExampleSet {
	const count = encoded.length;
	const pins = new Int32Array(2 * count);
	const schemas = new Int32Array(2 * count);
	encoded.forEach((context, index) => {
		pins[2 * index] = context.pins[0];
		pins[2 * index + 1] = context.pins[1];
		schemas[2 * index] = context.schemas[0];
		schemas[2 * index + 1] = context.schemas[1];
	});
	const pred = packLists(encoded, (context) => context.predRest);
	const ups = packLists(encoded, (context) => context.ups);
	const bag = packLists(encoded, (context) => context.bag);
	return {
		count,
		kind: Uint8Array.from(encoded, (context) => context.kind),
		src: Int32Array.from(encoded, (context) => context.src),
		pins,
		dataType: Uint8Array.from(encoded, (context) => context.dataType),
		valueType: Uint8Array.from(encoded, (context) => context.valueType),
		schemas,
		pred0: Int32Array.from(encoded, (context) => context.pred0),
		pred1: Int32Array.from(encoded, (context) => context.pred1),
		predStart: pred.start,
		predItems: pred.items,
		upsStart: ups.start,
		upsItems: ups.items,
		bagStart: bag.start,
		bagItems: bag.items,
		dst: dst ? Int32Array.from(dst) : new Int32Array(count),
		weight: weight
			? Float32Array.from(weight)
			: new Float32Array(count).fill(1),
	};
}

/** Everything the forward pass reads; training points the tables at per-batch work buffers. */
export type ForwardWeights = Record<DenseTensor | TableTensor, Float32Array> & {
	d: number;
	hidden: number;
};

export class Activations {
	readonly x: Float32Array;
	readonly z1: Float32Array;
	readonly a1: Float32Array;
	readonly keep: Float32Array;
	readonly z2: Float32Array;
	readonly norm: Float32Array;
	readonly y: Float32Array;
	readonly h: Float32Array;
	readonly gate = new Float32Array(2);
	readonly logits: Float32Array;
	invStd = 1;

	constructor(config: NetworkConfig, vocabSize: number) {
		this.x = new Float32Array(SLOTS * config.d);
		this.z1 = new Float32Array(config.hidden);
		this.a1 = new Float32Array(config.hidden);
		this.keep = new Float32Array(config.hidden);
		this.z2 = new Float32Array(config.d);
		this.norm = new Float32Array(config.d);
		this.y = new Float32Array(config.d);
		this.h = new Float32Array(config.d);
		this.logits = new Float32Array(vocabSize);
	}
}

function dot(
	a: Float32Array,
	aOffset: number,
	b: Float32Array,
	bOffset: number,
	length: number,
): number {
	let s0 = 0;
	let s1 = 0;
	let s2 = 0;
	let s3 = 0;
	let k = 0;
	for (; k + 3 < length; k += 4) {
		s0 += a[aOffset + k] * b[bOffset + k];
		s1 += a[aOffset + k + 1] * b[bOffset + k + 1];
		s2 += a[aOffset + k + 2] * b[bOffset + k + 2];
		s3 += a[aOffset + k + 3] * b[bOffset + k + 3];
	}
	for (; k < length; k++) s0 += a[aOffset + k] * b[bOffset + k];
	return s0 + s1 + s2 + s3;
}

function axpy(
	scale: number,
	source: Float32Array,
	sourceOffset: number,
	target: Float32Array,
	targetOffset: number,
	length: number,
): void {
	for (let k = 0; k < length; k++) {
		target[targetOffset + k] += scale * source[sourceOffset + k];
	}
}

function meanRows(
	x: Float32Array,
	offset: number,
	table: Float32Array,
	items: Int32Array,
	from: number,
	to: number,
	d: number,
): void {
	if (to <= from) return;
	const scale = 1 / (to - from);
	for (let item = from; item < to; item++) {
		axpy(scale, table, items[item] * d, x, offset, d);
	}
}

const GELU_C = Math.sqrt(2 / Math.PI);

function gelu(x: number): number {
	return 0.5 * x * (1 + Math.tanh(GELU_C * (x + 0.044715 * x * x * x)));
}

function geluGrad(x: number): number {
	const x2 = x * x;
	const t = Math.tanh(GELU_C * (x + 0.044715 * x2 * x));
	return 0.5 * (1 + t) + 0.5 * x * (1 - t * t) * GELU_C * (1 + 0.134145 * x2);
}

/** Fills `act.logits` for example `i`: MLP encoder, output table, copy gate; `<pad>` gets -Infinity. */
export function forward(
	w: ForwardWeights,
	set: ExampleSet,
	i: number,
	act: Activations,
	dropout: number,
	random: (() => number) | null,
): void {
	const { d, hidden } = w;
	const inputSize = SLOTS * d;
	const { x, z1, a1, keep, z2, norm, y, h, gate, logits } = act;
	const typeIn = w.typeIn;
	x.fill(0);
	const src = set.src[i];
	if (src >= 0) axpy(1, typeIn, src * d, x, 0, d);
	axpy(1, w.pin, set.pins[2 * i] * d, x, 0, d);
	axpy(1, w.pin, set.pins[2 * i + 1] * d, x, 0, d);
	axpy(1, w.dataType, set.dataType[i] * d, x, 0, d);
	axpy(1, w.valueType, set.valueType[i] * d, x, 0, d);
	axpy(1, w.schema, set.schemas[2 * i] * d, x, 0, d);
	axpy(1, w.schema, set.schemas[2 * i + 1] * d, x, 0, d);
	axpy(1, w.kind, set.kind[i] * d, x, 0, d);
	if (set.pred0[i] >= 0) axpy(1, typeIn, set.pred0[i] * d, x, d, d);
	if (set.pred1[i] >= 0) axpy(1, typeIn, set.pred1[i] * d, x, 2 * d, d);
	meanRows(
		x,
		3 * d,
		typeIn,
		set.predItems,
		set.predStart[i],
		set.predStart[i + 1],
		d,
	);
	meanRows(
		x,
		4 * d,
		typeIn,
		set.upsItems,
		set.upsStart[i],
		set.upsStart[i + 1],
		d,
	);
	meanRows(
		x,
		5 * d,
		typeIn,
		set.bagItems,
		set.bagStart[i],
		set.bagStart[i + 1],
		d,
	);

	const dropping = random !== null && dropout > 0;
	const survive = 1 / (1 - dropout);
	for (let j = 0; j < hidden; j++) {
		const z = w.b1[j] + dot(w.w1, j * inputSize, x, 0, inputSize);
		z1[j] = z;
		const k = dropping ? (random() < dropout ? 0 : survive) : 1;
		keep[j] = k;
		a1[j] = k === 0 ? 0 : gelu(z) * k;
	}
	let mean = 0;
	for (let k = 0; k < d; k++) {
		const z = w.b2[k] + dot(w.w2, k * hidden, a1, 0, hidden);
		z2[k] = z;
		mean += z;
	}
	mean /= d;
	let variance = 0;
	for (let k = 0; k < d; k++) {
		const centered = z2[k] - mean;
		variance += centered * centered;
	}
	const invStd = 1 / Math.sqrt(variance / d + 1e-5);
	act.invStd = invStd;
	for (let k = 0; k < d; k++) {
		norm[k] = (z2[k] - mean) * invStd;
		y[k] = norm[k] * w.lnGain[k] + w.lnBias[k];
	}
	for (let k = 0; k < d; k++) {
		h[k] = w.bHead[k] + dot(w.wHead, k * d, y, 0, d);
	}
	gate[0] = w.bCopy[0] + dot(w.wCopy, 0, h, 0, d);
	gate[1] = w.bCopy[1] + dot(w.wCopy, d, h, 0, d);

	const typeOut = w.typeOut;
	const typeBias = w.typeBias;
	const vocabSize = logits.length;
	for (let v = 0; v < vocabSize; v++) {
		const offset = v * d;
		let s0 = 0;
		let s1 = 0;
		let s2 = 0;
		let s3 = 0;
		for (let k = 0; k < d; k += 4) {
			s0 += h[k] * typeOut[offset + k];
			s1 += h[k + 1] * typeOut[offset + k + 1];
			s2 += h[k + 2] * typeOut[offset + k + 2];
			s3 += h[k + 3] * typeOut[offset + k + 3];
		}
		logits[v] = typeBias[v] + s0 + s1 + s2 + s3;
	}
	if (src >= 0) logits[src] += gate[0];
	for (let item = set.bagStart[i]; item < set.bagStart[i + 1]; item++) {
		logits[set.bagItems[item]] += gate[1];
	}
	logits[0] = Number.NEGATIVE_INFINITY;
}

/** Turns logits into probabilities in place. */
export function softmaxInPlace(logits: Float32Array): void {
	let max = Number.NEGATIVE_INFINITY;
	for (let v = 0; v < logits.length; v++) {
		if (logits[v] > max) max = logits[v];
	}
	let sum = 0;
	for (let v = 0; v < logits.length; v++) {
		const e = Math.exp(logits[v] - max);
		logits[v] = e;
		sum += e;
	}
	const inv = 1 / sum;
	for (let v = 0; v < logits.length; v++) logits[v] *= inv;
}

export function createRandom(seed: number): () => number {
	let state = seed >>> 0;
	return () => {
		state = (state + 0x6d2b79f5) >>> 0;
		let t = state;
		t = Math.imul(t ^ (t >>> 15), t | 1);
		t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
		return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
	};
}

function fillUniform(
	target: Float32Array,
	bound: number,
	random: () => number,
): void {
	for (let index = 0; index < target.length; index++) {
		target[index] = (random() * 2 - 1) * bound;
	}
}

function fillNormal(
	target: Float32Array,
	std: number,
	random: () => number,
): void {
	for (let index = 0; index < target.length; index++) {
		const u = Math.max(random(), 1e-12);
		target[index] =
			std * Math.sqrt(-2 * Math.log(u)) * Math.cos(2 * Math.PI * random());
	}
}

const ADAM_BETA1 = 0.9;
const ADAM_BETA2 = 0.999;
const ADAM_EPSILON = 1e-8;

/**
 * Trainable network. Per-type tables are `free * mask + P · meta` (+ `w_b · meta` for the bias),
 * rebuilt at the start of every batch: the output table fully (full softmax), the input table only
 * for the types the batch touches. Types never seen in training have their free rows masked, so
 * they are scored by metadata alone.
 */
export class Network {
	readonly config: NetworkConfig;
	readonly vocabSize: number;
	readonly meta: MetaFeatures;
	readonly maskIn: Uint8Array;
	readonly maskOut: Uint8Array;
	readonly params: Float32Array;
	readonly tensors: Record<TrainTensor, Float32Array>;
	private readonly grads: Float32Array;
	private readonly gradTensors: Record<TrainTensor, Float32Array>;
	private readonly moment1: Float32Array;
	private readonly moment2: Float32Array;
	private readonly typeIn: Float32Array;
	private readonly typeOut: Float32Array;
	private readonly typeBias: Float32Array;
	private readonly dTypeIn: Float32Array;
	private readonly dTypeOut: Float32Array;
	private readonly dTypeBias: Float32Array;
	private readonly stamp: Int32Array;
	private readonly touched: Int32Array;
	private touchedCount = 0;
	private batchId = 0;
	readonly weights: ForwardWeights;
	private readonly act: Activations;
	private readonly dh: Float32Array;
	private readonly dy: Float32Array;
	private readonly dn: Float32Array;
	private readonly dz2: Float32Array;
	private readonly da1: Float32Array;
	private readonly dz1: Float32Array;
	private readonly dx: Float32Array;

	constructor(
		config: NetworkConfig,
		vocabSize: number,
		meta: MetaFeatures,
		maskIn: Uint8Array,
		maskOut: Uint8Array,
	) {
		if (config.d % 4 !== 0) {
			throw new Error(`Network: d must be a multiple of 4, got ${config.d}`);
		}
		this.config = config;
		this.vocabSize = vocabSize;
		this.meta = meta;
		this.maskIn = maskIn;
		this.maskOut = maskOut;
		const sizes = tensorSizes(config, vocabSize, meta.names.length);
		const names: TrainTensor[] = [
			...DENSE_TENSORS,
			...META_TENSORS,
			...FREE_TENSORS,
		];
		const total = names.reduce((sum, name) => sum + sizes[name], 0);
		this.params = new Float32Array(total);
		this.grads = new Float32Array(total);
		this.moment1 = new Float32Array(total);
		this.moment2 = new Float32Array(total);
		const tensors = {} as Record<TrainTensor, Float32Array>;
		const gradTensors = {} as Record<TrainTensor, Float32Array>;
		let offset = 0;
		for (const name of names) {
			tensors[name] = this.params.subarray(offset, offset + sizes[name]);
			gradTensors[name] = this.grads.subarray(offset, offset + sizes[name]);
			offset += sizes[name];
		}
		this.tensors = tensors;
		this.gradTensors = gradTensors;
		const { d } = config;
		this.typeIn = new Float32Array(vocabSize * d);
		this.typeOut = new Float32Array(vocabSize * d);
		this.typeBias = new Float32Array(vocabSize);
		this.dTypeIn = new Float32Array(vocabSize * d);
		this.dTypeOut = new Float32Array(vocabSize * d);
		this.dTypeBias = new Float32Array(vocabSize);
		this.stamp = new Int32Array(vocabSize);
		this.touched = new Int32Array(vocabSize);
		const weights = {
			d,
			hidden: config.hidden,
			typeIn: this.typeIn,
			typeOut: this.typeOut,
			typeBias: this.typeBias,
		} as ForwardWeights;
		for (const name of DENSE_TENSORS) weights[name] = tensors[name];
		this.weights = weights;
		this.act = new Activations(config, vocabSize);
		this.dh = new Float32Array(d);
		this.dy = new Float32Array(d);
		this.dn = new Float32Array(d);
		this.dz2 = new Float32Array(d);
		this.da1 = new Float32Array(config.hidden);
		this.dz1 = new Float32Array(config.hidden);
		this.dx = new Float32Array(SLOTS * d);
	}

	initialize(random: () => number): void {
		const { d, hidden } = this.config;
		const t = this.tensors;
		const metaBound = 1 / Math.sqrt(Math.max(1, this.meta.names.length));
		fillUniform(t.pIn, metaBound, random);
		fillUniform(t.pOut, metaBound, random);
		fillUniform(t.wB, metaBound, random);
		fillNormal(t.pin, Math.SQRT1_2, random);
		fillNormal(t.schema, Math.SQRT1_2, random);
		fillNormal(t.dataType, 1, random);
		fillNormal(t.valueType, 1, random);
		fillNormal(t.kind, 1, random);
		const inputBound = 1 / Math.sqrt(SLOTS * d);
		fillUniform(t.w1, inputBound, random);
		fillUniform(t.b1, inputBound, random);
		const hiddenBound = 1 / Math.sqrt(hidden);
		fillUniform(t.w2, hiddenBound, random);
		fillUniform(t.b2, hiddenBound, random);
		t.lnGain.fill(1);
		t.lnBias.fill(0);
		const headBound = 1 / Math.sqrt(d);
		fillUniform(t.wHead, headBound, random);
		fillUniform(t.bHead, headBound, random);
		fillUniform(t.wCopy, headBound, random);
		fillUniform(t.bCopy, headBound, random);
		t.freeIn.fill(0);
		t.freeOut.fill(0);
		t.bFree.fill(0);
	}

	/** Adds `P · meta[row]` into `target[targetOffset..]`. */
	projectMeta(
		projection: Float32Array,
		row: number,
		target: Float32Array,
		targetOffset: number,
	): void {
		const { d } = this.config;
		const { rowStart, cols, values } = this.meta;
		for (let item = rowStart[row]; item < rowStart[row + 1]; item++) {
			axpy(values[item], projection, cols[item] * d, target, targetOffset, d);
		}
	}

	metaBias(row: number): number {
		const { rowStart, cols, values } = this.meta;
		const wB = this.tensors.wB;
		let bias = 0;
		for (let item = rowStart[row]; item < rowStart[row + 1]; item++) {
			bias += values[item] * wB[cols[item]];
		}
		return bias;
	}

	private buildTypeInRow(row: number): void {
		const { d } = this.config;
		const offset = row * d;
		if (this.maskIn[row]) {
			this.typeIn.set(this.tensors.freeIn.subarray(offset, offset + d), offset);
		} else {
			this.typeIn.fill(0, offset, offset + d);
		}
		this.projectMeta(this.tensors.pIn, row, this.typeIn, offset);
	}

	private buildOutputTables(): void {
		const { d } = this.config;
		const { freeOut, bFree, pOut } = this.tensors;
		for (let v = 0; v < this.vocabSize; v++) {
			const offset = v * d;
			if (this.maskOut[v]) {
				this.typeOut.set(freeOut.subarray(offset, offset + d), offset);
				this.typeBias[v] = bFree[v] + this.metaBias(v);
			} else {
				this.typeOut.fill(0, offset, offset + d);
				this.typeBias[v] = this.metaBias(v);
			}
			this.projectMeta(pOut, v, this.typeOut, offset);
		}
	}

	/** Builds every per-type table; used for evaluation and for exporting a model. */
	buildAllTables(): void {
		for (let v = 0; v < this.vocabSize; v++) this.buildTypeInRow(v);
		this.buildOutputTables();
	}

	private touch(row: number): void {
		if (row < 0 || this.stamp[row] === this.batchId) return;
		this.stamp[row] = this.batchId;
		this.touched[this.touchedCount++] = row;
	}

	private touchList(items: Int32Array, from: number, to: number): void {
		for (let item = from; item < to; item++) this.touch(items[item]);
	}

	beginBatch(
		set: ExampleSet,
		order: ArrayLike<number>,
		from: number,
		to: number,
	): void {
		this.grads.fill(0);
		this.buildOutputTables();
		this.dTypeOut.fill(0);
		this.dTypeBias.fill(0);
		this.batchId++;
		this.touchedCount = 0;
		for (let position = from; position < to; position++) {
			const i = order[position];
			this.touch(set.src[i]);
			this.touch(set.pred0[i]);
			this.touch(set.pred1[i]);
			this.touchList(set.predItems, set.predStart[i], set.predStart[i + 1]);
			this.touchList(set.upsItems, set.upsStart[i], set.upsStart[i + 1]);
			this.touchList(set.bagItems, set.bagStart[i], set.bagStart[i + 1]);
		}
		const { d } = this.config;
		for (let index = 0; index < this.touchedCount; index++) {
			const row = this.touched[index];
			this.buildTypeInRow(row);
			this.dTypeIn.fill(0, row * d, row * d + d);
		}
	}

	/** Forward + backward of one example; `scale` is its share of the batch loss. Returns its cross-entropy. */
	trainExample(
		set: ExampleSet,
		i: number,
		scale: number,
		dropout: number,
		random: (() => number) | null,
	): number {
		const { d, hidden } = this.config;
		const inputSize = SLOTS * d;
		const act = this.act;
		const w = this.weights;
		const g = this.gradTensors;
		forward(w, set, i, act, dropout, random);
		const p = act.logits;
		softmaxInPlace(p);
		const dst = set.dst[i];
		const loss = -Math.log(Math.max(p[dst], 1e-30));
		const { h, y, norm, a1, keep, z1, x } = act;

		const src = set.src[i];
		let dGate0 = 0;
		if (src >= 0) dGate0 = scale * (p[src] - (src === dst ? 1 : 0));
		let dGate1 = 0;
		for (let item = set.bagStart[i]; item < set.bagStart[i + 1]; item++) {
			const t = set.bagItems[item];
			dGate1 += scale * (p[t] - (t === dst ? 1 : 0));
		}

		const typeOut = this.typeOut;
		const dTypeOut = this.dTypeOut;
		const dTypeBias = this.dTypeBias;
		const dh = this.dh;
		dh.fill(0);
		const vocabSize = this.vocabSize;
		for (let v = 1; v < vocabSize; v++) {
			const gv = v === dst ? scale * (p[v] - 1) : scale * p[v];
			dTypeBias[v] += gv;
			const offset = v * d;
			for (let k = 0; k < d; k += 4) {
				dTypeOut[offset + k] += gv * h[k];
				dTypeOut[offset + k + 1] += gv * h[k + 1];
				dTypeOut[offset + k + 2] += gv * h[k + 2];
				dTypeOut[offset + k + 3] += gv * h[k + 3];
			}
		}
		for (let v = 1; v < vocabSize; v++) {
			const gv = v === dst ? scale * (p[v] - 1) : scale * p[v];
			const offset = v * d;
			for (let k = 0; k < d; k += 4) {
				dh[k] += gv * typeOut[offset + k];
				dh[k + 1] += gv * typeOut[offset + k + 1];
				dh[k + 2] += gv * typeOut[offset + k + 2];
				dh[k + 3] += gv * typeOut[offset + k + 3];
			}
		}

		const wCopy = w.wCopy;
		for (let k = 0; k < d; k++) {
			dh[k] += dGate0 * wCopy[k] + dGate1 * wCopy[d + k];
			g.wCopy[k] += dGate0 * h[k];
			g.wCopy[d + k] += dGate1 * h[k];
		}
		g.bCopy[0] += dGate0;
		g.bCopy[1] += dGate1;

		const dy = this.dy;
		dy.fill(0);
		for (let k = 0; k < d; k++) {
			const gk = dh[k];
			g.bHead[k] += gk;
			axpy(gk, y, 0, g.wHead, k * d, d);
			axpy(gk, w.wHead, k * d, dy, 0, d);
		}

		const dn = this.dn;
		let meanDn = 0;
		let meanDnNorm = 0;
		for (let k = 0; k < d; k++) {
			g.lnGain[k] += dy[k] * norm[k];
			g.lnBias[k] += dy[k];
			const value = dy[k] * w.lnGain[k];
			dn[k] = value;
			meanDn += value;
			meanDnNorm += value * norm[k];
		}
		meanDn /= d;
		meanDnNorm /= d;
		const dz2 = this.dz2;
		for (let k = 0; k < d; k++) {
			dz2[k] = act.invStd * (dn[k] - meanDn - norm[k] * meanDnNorm);
		}

		const da1 = this.da1;
		da1.fill(0);
		for (let k = 0; k < d; k++) {
			const gk = dz2[k];
			g.b2[k] += gk;
			axpy(gk, a1, 0, g.w2, k * hidden, hidden);
			axpy(gk, w.w2, k * hidden, da1, 0, hidden);
		}

		const dz1 = this.dz1;
		for (let j = 0; j < hidden; j++) {
			dz1[j] = keep[j] === 0 ? 0 : da1[j] * keep[j] * geluGrad(z1[j]);
		}

		const dx = this.dx;
		dx.fill(0);
		for (let j = 0; j < hidden; j++) {
			const gj = dz1[j];
			if (gj === 0) continue;
			g.b1[j] += gj;
			axpy(gj, x, 0, g.w1, j * inputSize, inputSize);
			axpy(gj, w.w1, j * inputSize, dx, 0, inputSize);
		}

		const dTypeIn = this.dTypeIn;
		if (src >= 0) axpy(1, dx, 0, dTypeIn, src * d, d);
		axpy(1, dx, 0, g.pin, set.pins[2 * i] * d, d);
		axpy(1, dx, 0, g.pin, set.pins[2 * i + 1] * d, d);
		axpy(1, dx, 0, g.dataType, set.dataType[i] * d, d);
		axpy(1, dx, 0, g.valueType, set.valueType[i] * d, d);
		axpy(1, dx, 0, g.schema, set.schemas[2 * i] * d, d);
		axpy(1, dx, 0, g.schema, set.schemas[2 * i + 1] * d, d);
		axpy(1, dx, 0, g.kind, set.kind[i] * d, d);
		if (set.pred0[i] >= 0) axpy(1, dx, d, dTypeIn, set.pred0[i] * d, d);
		if (set.pred1[i] >= 0) axpy(1, dx, 2 * d, dTypeIn, set.pred1[i] * d, d);
		this.spreadMean(
			dx,
			3 * d,
			set.predItems,
			set.predStart[i],
			set.predStart[i + 1],
		);
		this.spreadMean(
			dx,
			4 * d,
			set.upsItems,
			set.upsStart[i],
			set.upsStart[i + 1],
		);
		this.spreadMean(
			dx,
			5 * d,
			set.bagItems,
			set.bagStart[i],
			set.bagStart[i + 1],
		);
		return loss;
	}

	private spreadMean(
		dx: Float32Array,
		offset: number,
		items: Int32Array,
		from: number,
		to: number,
	): void {
		if (to <= from) return;
		const { d } = this.config;
		const scale = 1 / (to - from);
		for (let item = from; item < to; item++) {
			axpy(scale, dx, offset, this.dTypeIn, items[item] * d, d);
		}
	}

	/** Pushes the per-type table gradients into the free rows and the metadata projections. */
	endBatch(): void {
		const { d } = this.config;
		const g = this.gradTensors;
		const { rowStart, cols, values } = this.meta;
		for (let index = 0; index < this.touchedCount; index++) {
			const row = this.touched[index];
			const offset = row * d;
			if (this.maskIn[row]) axpy(1, this.dTypeIn, offset, g.freeIn, offset, d);
			for (let item = rowStart[row]; item < rowStart[row + 1]; item++) {
				axpy(values[item], this.dTypeIn, offset, g.pIn, cols[item] * d, d);
			}
		}
		for (let v = 1; v < this.vocabSize; v++) {
			const offset = v * d;
			const biasGrad = this.dTypeBias[v];
			if (this.maskOut[v]) {
				axpy(1, this.dTypeOut, offset, g.freeOut, offset, d);
				g.bFree[v] += biasGrad;
			}
			for (let item = rowStart[v]; item < rowStart[v + 1]; item++) {
				axpy(values[item], this.dTypeOut, offset, g.pOut, cols[item] * d, d);
				g.wB[cols[item]] += values[item] * biasGrad;
			}
		}
	}

	/** AdamW with PyTorch semantics (decoupled decay on every tensor). */
	adamStep(learningRate: number, weightDecay: number, step: number): void {
		const params = this.params;
		const grads = this.grads;
		const m = this.moment1;
		const v = this.moment2;
		const stepSize = learningRate / (1 - ADAM_BETA1 ** step);
		const correction2 = 1 / (1 - ADAM_BETA2 ** step);
		const decay = 1 - learningRate * weightDecay;
		for (let index = 0; index < params.length; index++) {
			const grad = grads[index];
			const m1 = ADAM_BETA1 * m[index] + (1 - ADAM_BETA1) * grad;
			const m2 = ADAM_BETA2 * v[index] + (1 - ADAM_BETA2) * grad * grad;
			m[index] = m1;
			v[index] = m2;
			params[index] =
				params[index] * decay -
				(stepSize * m1) / (Math.sqrt(m2 * correction2) + ADAM_EPSILON);
		}
	}

	/** Gradient of the last batch (for tests). */
	gradient(): Float32Array {
		return this.grads;
	}

	/** Weighted mean cross-entropy of `order[from..to)` without dropout (for tests). */
	batchLoss(
		set: ExampleSet,
		order: ArrayLike<number>,
		from: number,
		to: number,
	): number {
		this.buildAllTables();
		let loss = 0;
		let weight = 0;
		for (let position = from; position < to; position++) {
			const i = order[position];
			forward(this.weights, set, i, this.act, 0, null);
			softmaxInPlace(this.act.logits);
			loss -=
				set.weight[i] * Math.log(Math.max(this.act.logits[set.dst[i]], 1e-30));
			weight += set.weight[i];
		}
		return loss / weight;
	}

	/** Rank of the true type among all types (1 = best); `buildAllTables` must have run. */
	rankOf(set: ExampleSet, i: number): number {
		forward(this.weights, set, i, this.act, 0, null);
		const logits = this.act.logits;
		const target = logits[set.dst[i]];
		let rank = 1;
		for (let v = 1; v < logits.length; v++) {
			if (logits[v] > target) rank++;
		}
		return rank;
	}

	/** Per-type tables after `buildAllTables`. */
	tables(): Record<TableTensor, Float32Array> {
		return {
			typeIn: this.typeIn,
			typeOut: this.typeOut,
			typeBias: this.typeBias,
		};
	}
}
