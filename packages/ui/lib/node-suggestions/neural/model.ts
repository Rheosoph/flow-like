import { expandTransition } from "../facts";
import {
	type BoardFacts,
	type CatalogEntry,
	type RankedCandidate,
	SUGGESTION_FORMAT_VERSION,
	type SuggestionContext,
	type SuggestionModel,
} from "../types";
import {
	type EncodedContext,
	type MetaFeatures,
	PAD,
	buildMeta,
	buildVocab,
	encodeContext,
} from "./featurize";
import { decodeHalf, encodeHalf, roundToHalf } from "./half";
import {
	Activations,
	DENSE_TENSORS,
	type ExampleSet,
	type ForwardWeights,
	META_TENSORS,
	type ModelTensor,
	Network,
	type NetworkConfig,
	TABLE_TENSORS,
	createRandom,
	forward,
	packExamples,
	softmaxInPlace,
	tensorSizes,
} from "./mlp";

export interface NeuralTrainProgress {
	/** 1-based epoch that just finished. */
	epoch: number;
	/** Upper bound; early stopping may end training before it. */
	epochs: number;
	/** Weighted mean training cross-entropy of the epoch. */
	loss: number;
	/** Validation MRR@10 after the epoch; `NaN` when the corpus is too small for a validation split. */
	valMrr: number;
}

export interface NeuralTrainOptions {
	d?: number;
	hidden?: number;
	epochs?: number;
	batchSize?: number;
	learningRate?: number;
	weightDecay?: number;
	dropout?: number;
	/** Epochs without a validation MRR gain before training stops; only counted in the second half of `epochs`. */
	patience?: number;
	/** Share of boards held out for early stopping. */
	validationFraction?: number;
	seed?: number;
	/** Longest stretch of compute between yields to the event loop. */
	yieldEveryMs?: number;
	/** Aborting rejects `train` with `signal.reason`. */
	signal?: AbortSignal;
	onProgress?: (progress: NeuralTrainProgress) => void;
	/** Start from this model's weights (mapped by type and feature name); ignored when `d`/`hidden` differ. */
	warmStart?: NeuralModel;
}

const DEFAULTS = {
	d: 32,
	hidden: 128,
	epochs: 12,
	batchSize: 128,
	learningRate: 3e-3,
	weightDecay: 1e-4,
	dropout: 0.2,
	patience: 3,
	validationFraction: 0.1,
	seed: 0,
	yieldEveryMs: 25,
};
type Settings = typeof DEFAULTS & Pick<NeuralTrainOptions, "onProgress">;

function resolveSettings(options: NeuralTrainOptions): Settings {
	const settings: Settings = { ...DEFAULTS, onProgress: options.onProgress };
	for (const key of Object.keys(DEFAULTS) as (keyof typeof DEFAULTS)[]) {
		const value = options[key];
		if (typeof value === "number" && Number.isFinite(value)) {
			settings[key] = value;
		}
	}
	return settings;
}
const PIN_BUCKETS = 512;
const SCHEMA_BUCKETS = 64;
const MRR_CUTOFF = 10;

const MODEL_TENSORS: readonly ModelTensor[] = [
	...TABLE_TENSORS,
	...META_TENSORS,
	...DENSE_TENSORS,
];
const FORMAT = "flow-like/node-suggestions/neural";
const FORMAT_VERSION = 1;

interface SerializedNeuralModel {
	format: typeof FORMAT;
	version: number;
	suggestionFormat: number;
	config: NetworkConfig;
	vocab: string[];
	features: string[];
	/** Base64 little-endian binary16 per tensor. */
	tensors: Record<ModelTensor, string>;
}

interface NeuralState {
	config: NetworkConfig;
	vocab: string[];
	features: string[];
	tensors: Record<ModelTensor, Float32Array>;
}

const timeout = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

/**
 * Hands control back to the event loop. MessageChannel round trips avoid the 4 ms clamp on nested
 * timers, but Node and Bun run no timers while port messages keep arriving, so every fourth pause
 * goes through `setTimeout`.
 */
function createYielder(): { pause: () => Promise<void>; close: () => void } {
	if (typeof MessageChannel === "undefined") {
		return { pause: timeout, close: () => {} };
	}
	const channel = new MessageChannel();
	const waiting: (() => void)[] = [];
	channel.port1.onmessage = () => waiting.shift()?.();
	let pauses = 0;
	return {
		pause: () => {
			if (++pauses % 4 === 0) return timeout();
			return new Promise<void>((resolve) => {
				waiting.push(resolve);
				channel.port2.postMessage(null);
			});
		},
		close: () => {
			channel.port1.close();
			channel.port2.close();
		},
	};
}

class Pacer {
	private last = performance.now();

	constructor(
		private readonly budgetMs: number,
		private readonly yielder: { pause: () => Promise<void> },
		private readonly signal: AbortSignal | undefined,
	) {}

	due(): boolean {
		return performance.now() - this.last >= this.budgetMs;
	}

	async pause(): Promise<void> {
		this.signal?.throwIfAborted();
		await this.yielder.pause();
		this.signal?.throwIfAborted();
		this.last = performance.now();
	}
}

function splitBoards(
	corpus: BoardFacts[],
	fraction: number,
	random: () => number,
): { train: BoardFacts[]; validation: BoardFacts[] } {
	const order = corpus.map((_, index) => index);
	for (let index = order.length - 1; index > 0; index--) {
		const swap = Math.floor(random() * (index + 1));
		[order[index], order[swap]] = [order[swap], order[index]];
	}
	const held = new Set(order.slice(0, Math.floor(corpus.length * fraction)));
	return {
		train: corpus.filter((_, index) => !held.has(index)),
		validation: corpus.filter((_, index) => held.has(index)),
	};
}

/** Examples of a board are weighted `1 / count` of identical (src, srcPin, pred0, pred1, ups, dst) rows. */
async function buildExamples(
	boards: BoardFacts[],
	index: Map<string, number>,
	config: NetworkConfig,
	pacer: Pacer,
): Promise<ExampleSet> {
	const encoded: EncodedContext[] = [];
	const dst: number[] = [];
	const weight: number[] = [];
	for (const facts of boards) {
		const keys: string[] = [];
		const counts = new Map<string, number>();
		for (const transition of facts.transitions) {
			const fact = expandTransition(facts, transition);
			const target = index.get(fact.dst);
			if (!target) continue;
			encoded.push(encodeContext(fact, index, config));
			dst.push(target);
			const key = [
				fact.src,
				fact.srcPin,
				fact.pred[0] ?? "",
				fact.pred[1] ?? "",
				fact.ups.join("\u0001"),
				fact.dst,
			].join("\u0000");
			keys.push(key);
			counts.set(key, (counts.get(key) ?? 0) + 1);
		}
		for (const key of keys) weight.push(1 / (counts.get(key) ?? 1));
		if (pacer.due()) await pacer.pause();
	}
	return packExamples(encoded, dst, weight);
}

function seenMasks(
	set: ExampleSet,
	vocabSize: number,
): { maskIn: Uint8Array; maskOut: Uint8Array } {
	const maskIn = new Uint8Array(vocabSize);
	const maskOut = new Uint8Array(vocabSize);
	const mark = (row: number) => {
		if (row > 0) maskIn[row] = 1;
	};
	for (let i = 0; i < set.count; i++) {
		mark(set.src[i]);
		mark(set.pred0[i]);
		mark(set.pred1[i]);
		maskOut[set.dst[i]] = 1;
	}
	for (const items of [set.predItems, set.upsItems, set.bagItems]) {
		for (const row of items) mark(row);
	}
	return { maskIn, maskOut };
}

/** PyTorch-style one-cycle: cosine warm-up over the first 10% from lr/25, cosine decay to lr/25e4. */
function oneCycle(peak: number, step: number, total: number): number {
	const warmup = Math.max(1, Math.floor(total * 0.1));
	const anneal = (from: number, to: number, t: number) =>
		to + ((from - to) * (1 + Math.cos(Math.PI * Math.min(1, t)))) / 2;
	if (step < warmup) return anneal(peak / 25, peak, step / warmup);
	return anneal(
		peak,
		peak / 25 / 1e4,
		(step - warmup) / Math.max(1, total - warmup),
	);
}

function sameConfig(a: NetworkConfig, b: NetworkConfig): boolean {
	return (
		a.d === b.d &&
		a.hidden === b.hidden &&
		a.pinBuckets === b.pinBuckets &&
		a.schemaBuckets === b.schemaBuckets
	);
}

function isPositiveInteger(value: unknown): value is number {
	return typeof value === "number" && Number.isInteger(value) && value > 0;
}

function isStringArray(value: unknown): value is string[] {
	return (
		Array.isArray(value) && value.every((item) => typeof item === "string")
	);
}

function forwardWeights(state: NeuralState): ForwardWeights {
	const weights = {
		d: state.config.d,
		hidden: state.config.hidden,
	} as ForwardWeights;
	for (const name of [...TABLE_TENSORS, ...DENSE_TENSORS]) {
		weights[name] = state.tensors[name];
	}
	return weights;
}

/**
 * Next-node recommender: an MLP over the anchor pin, its predecessors, upstream providers and the
 * board's type bag, with a copy gate towards types already on the board. Type embeddings combine a
 * free row (types seen in training) with a projection of catalog metadata, so catalog nodes the
 * user never placed still get a score. Trains in the calling thread and yields between batches.
 */
export class NeuralModel implements SuggestionModel {
	private readonly state: NeuralState;
	private readonly index: Map<string, number>;
	private readonly weights: ForwardWeights;
	private readonly activations: Activations;

	private constructor(state: NeuralState) {
		this.state = state;
		this.index = new Map(state.vocab.map((type, row) => [type, row]));
		this.weights = forwardWeights(state);
		this.activations = new Activations(state.config, state.vocab.length);
	}

	static async train(
		corpus: BoardFacts[],
		catalog: CatalogEntry[],
		options: NeuralTrainOptions = {},
	): Promise<NeuralModel> {
		const settings = resolveSettings(options);
		options.signal?.throwIfAborted();
		const yielder = createYielder();
		try {
			return await NeuralModel.trainPaced(
				corpus,
				catalog,
				options,
				settings,
				new Pacer(settings.yieldEveryMs, yielder, options.signal),
			);
		} finally {
			yielder.close();
		}
	}

	private static async trainPaced(
		corpus: BoardFacts[],
		catalog: CatalogEntry[],
		options: NeuralTrainOptions,
		settings: Settings,
		pacer: Pacer,
	): Promise<NeuralModel> {
		const config: NetworkConfig = {
			d: settings.d,
			hidden: settings.hidden,
			pinBuckets: PIN_BUCKETS,
			schemaBuckets: SCHEMA_BUCKETS,
		};
		const random = createRandom(settings.seed);
		const vocab = buildVocab(catalog, corpus);
		const index = new Map(vocab.map((type, row) => [type, row]));
		const meta = buildMeta(vocab, catalog);
		const { train, validation } = splitBoards(
			corpus,
			settings.validationFraction,
			random,
		);
		const trainSet = await buildExamples(train, index, config, pacer);
		if (trainSet.count === 0) {
			throw new Error(
				`NeuralModel.train: no transitions in ${train.length} training boards`,
			);
		}
		const validationSet = await buildExamples(validation, index, config, pacer);
		const { maskIn, maskOut } = seenMasks(trainSet, vocab.length);
		const network = new Network(config, vocab.length, meta, maskIn, maskOut);
		network.initialize(random);
		if (options.warmStart) {
			NeuralModel.applyWarmStart(network, options.warmStart.state, vocab, meta);
		}
		await NeuralModel.fit(
			network,
			trainSet,
			validationSet,
			settings,
			random,
			pacer,
		);
		network.buildAllTables();
		const tables = network.tables();
		const tensors = {} as Record<ModelTensor, Float32Array>;
		for (const name of TABLE_TENSORS) {
			tensors[name] = roundToHalf(tables[name].slice());
		}
		for (const name of [...META_TENSORS, ...DENSE_TENSORS]) {
			tensors[name] = roundToHalf(network.tensors[name].slice());
		}
		return new NeuralModel({ config, vocab, features: meta.names, tensors });
	}

	private static async fit(
		network: Network,
		trainSet: ExampleSet,
		validationSet: ExampleSet,
		settings: Settings,
		random: () => number,
		pacer: Pacer,
	): Promise<void> {
		const count = trainSet.count;
		const batchSize = Math.max(1, settings.batchSize);
		const order = Int32Array.from({ length: count }, (_, i) => i);
		const totalSteps = settings.epochs * Math.ceil(count / batchSize);
		let step = 0;
		let best = Number.NEGATIVE_INFINITY;
		let bestParams: Float32Array | null = null;
		let stale = 0;
		for (let epoch = 0; epoch < settings.epochs; epoch++) {
			for (let index = count - 1; index > 0; index--) {
				const swap = Math.floor(random() * (index + 1));
				const held = order[index];
				order[index] = order[swap];
				order[swap] = held;
			}
			let lossSum = 0;
			let weightSum = 0;
			for (let from = 0; from < count; from += batchSize) {
				const to = Math.min(count, from + batchSize);
				network.beginBatch(trainSet, order, from, to);
				let batchWeight = 0;
				for (let position = from; position < to; position++) {
					batchWeight += trainSet.weight[order[position]];
				}
				for (let position = from; position < to; position++) {
					const i = order[position];
					const weight = trainSet.weight[i];
					const loss = network.trainExample(
						trainSet,
						i,
						weight / batchWeight,
						settings.dropout,
						random,
					);
					lossSum += weight * loss;
					weightSum += weight;
					if (pacer.due()) await pacer.pause();
				}
				network.endBatch();
				step++;
				network.adamStep(
					oneCycle(settings.learningRate, step, totalSteps),
					settings.weightDecay,
					step,
				);
			}
			const valMrr =
				validationSet.count > 0
					? await NeuralModel.evaluate(network, validationSet, pacer)
					: Number.NaN;
			settings.onProgress?.({
				epoch: epoch + 1,
				epochs: settings.epochs,
				loss: lossSum / weightSum,
				valMrr,
			});
			if (Number.isNaN(valMrr)) continue;
			if (valMrr > best + 1e-4) {
				best = valMrr;
				stale = 0;
				bestParams = network.params.slice();
			} else if (
				2 * (epoch + 1) > settings.epochs &&
				++stale >= settings.patience
			) {
				// Validation MRR is noisy while the learning rate is high; stopping only counts in the decay half.
				break;
			}
		}
		if (bestParams) network.params.set(bestParams);
	}

	private static async evaluate(
		network: Network,
		set: ExampleSet,
		pacer: Pacer,
	): Promise<number> {
		network.buildAllTables();
		let sum = 0;
		for (let i = 0; i < set.count; i++) {
			const rank = network.rankOf(set, i);
			if (rank <= MRR_CUTOFF) sum += 1 / rank;
			if (pacer.due()) await pacer.pause();
		}
		return sum / set.count;
	}

	/**
	 * Continues from `previous`: dense layers are copied, metadata projections are mapped by feature
	 * name, and each type's free rows are set so its embedding equals the previous one under the new
	 * metadata.
	 */
	private static applyWarmStart(
		network: Network,
		previous: NeuralState,
		vocab: string[],
		meta: MetaFeatures,
	): void {
		if (!sameConfig(previous.config, network.config)) return;
		const { d } = network.config;
		const t = network.tensors;
		const old = previous.tensors;
		for (const name of DENSE_TENSORS) t[name].set(old[name]);
		const featureIndex = new Map(
			previous.features.map((name, row) => [name, row]),
		);
		t.pIn.fill(0);
		t.pOut.fill(0);
		t.wB.fill(0);
		meta.names.forEach((name, row) => {
			const from = featureIndex.get(name);
			if (from === undefined) return;
			t.pIn.set(old.pIn.subarray(from * d, from * d + d), row * d);
			t.pOut.set(old.pOut.subarray(from * d, from * d + d), row * d);
			t.wB[row] = old.wB[from];
		});
		const typeIndex = new Map(previous.vocab.map((type, row) => [type, row]));
		const projection = new Float32Array(d);
		for (let row = 1; row < vocab.length; row++) {
			const from = typeIndex.get(vocab[row]);
			if (from === undefined) continue;
			projection.fill(0);
			network.projectMeta(t.pIn, row, projection, 0);
			for (let k = 0; k < d; k++) {
				t.freeIn[row * d + k] = old.typeIn[from * d + k] - projection[k];
			}
			projection.fill(0);
			network.projectMeta(t.pOut, row, projection, 0);
			for (let k = 0; k < d; k++) {
				t.freeOut[row * d + k] = old.typeOut[from * d + k] - projection[k];
			}
			t.bFree[row] = old.typeBias[from] - network.metaBias(row);
		}
	}

	static deserialize(json: string): NeuralModel {
		const data = JSON.parse(json) as Partial<SerializedNeuralModel>;
		if (data.format !== FORMAT || data.version !== FORMAT_VERSION) {
			throw new Error(
				`NeuralModel.deserialize: unsupported format ${String(data.format)} v${String(data.version)}`,
			);
		}
		if (data.suggestionFormat !== SUGGESTION_FORMAT_VERSION) {
			throw new Error(
				`NeuralModel.deserialize: suggestion format ${String(data.suggestionFormat)} != ${SUGGESTION_FORMAT_VERSION}`,
			);
		}
		const config = data.config;
		if (
			!config ||
			!isPositiveInteger(config.d) ||
			config.d % 4 !== 0 ||
			!isPositiveInteger(config.hidden) ||
			!isPositiveInteger(config.pinBuckets) ||
			!isPositiveInteger(config.schemaBuckets)
		) {
			throw new Error(
				`NeuralModel.deserialize: invalid config ${JSON.stringify(config)}`,
			);
		}
		if (!isStringArray(data.vocab) || data.vocab[0] !== PAD) {
			throw new Error(
				"NeuralModel.deserialize: vocab missing or without <pad>",
			);
		}
		if (!isStringArray(data.features)) {
			throw new Error("NeuralModel.deserialize: features missing");
		}
		const sizes = tensorSizes(config, data.vocab.length, data.features.length);
		const tensors = {} as Record<ModelTensor, Float32Array>;
		for (const name of MODEL_TENSORS) {
			const encoded = data.tensors?.[name];
			if (typeof encoded !== "string") {
				throw new Error(`NeuralModel.deserialize: tensor ${name} missing`);
			}
			const values = decodeHalf(encoded);
			if (values.length !== sizes[name]) {
				throw new Error(
					`NeuralModel.deserialize: tensor ${name} has ${values.length} values, expected ${sizes[name]}`,
				);
			}
			tensors[name] = values;
		}
		return new NeuralModel({
			config,
			vocab: data.vocab,
			features: data.features,
			tensors,
		});
	}

	/** Top `limit` types by softmax probability over the whole vocabulary. Never throws. */
	rank(context: SuggestionContext, limit: number): RankedCandidate[] {
		try {
			const { vocab } = this.state;
			const count = Math.min(Math.floor(limit), vocab.length - 1);
			if (!(count > 0)) return [];
			const encoded = encodeContext(context, this.index, this.state.config);
			const set = packExamples([encoded]);
			forward(this.weights, set, 0, this.activations, 0, null);
			const probabilities = this.activations.logits;
			softmaxInPlace(probabilities);
			const rows = new Int32Array(count).fill(-1);
			const scores = new Float64Array(count).fill(-1);
			for (let v = 1; v < probabilities.length; v++) {
				const p = probabilities[v];
				if (p <= scores[count - 1]) continue;
				let slot = count - 1;
				while (slot > 0 && scores[slot - 1] < p) {
					scores[slot] = scores[slot - 1];
					rows[slot] = rows[slot - 1];
					slot--;
				}
				scores[slot] = p;
				rows[slot] = v;
			}
			const ranked: RankedCandidate[] = [];
			for (let slot = 0; slot < count && rows[slot] >= 0; slot++) {
				ranked.push({ type: vocab[rows[slot]], probability: scores[slot] });
			}
			return ranked;
		} catch {
			return [];
		}
	}

	serialize(): string {
		const tensors = {} as Record<ModelTensor, string>;
		for (const name of MODEL_TENSORS) {
			tensors[name] = encodeHalf(this.state.tensors[name]);
		}
		const data: SerializedNeuralModel = {
			format: FORMAT,
			version: FORMAT_VERSION,
			suggestionFormat: SUGGESTION_FORMAT_VERSION,
			config: this.state.config,
			vocab: this.state.vocab,
			features: this.state.features,
			tensors,
		};
		return JSON.stringify(data);
	}
}
