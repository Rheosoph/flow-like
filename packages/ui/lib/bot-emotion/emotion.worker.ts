import { AutoTokenizer, env } from "@huggingface/transformers";
import * as ort from "onnxruntime-web/webgpu";
import { calibrateEmotionScores } from "./calibration";
import {
	BOT_EMOTION_MAX_LENGTH,
	BOT_EMOTION_MODEL,
	BOT_EMOTION_REVISION,
	type EmotionWorkerRequest,
	type EmotionWorkerResponse,
	scoresFromOutput,
} from "./protocol";
import { EmotionScorer } from "./scoring";

const MODEL_URL = `https://huggingface.co/${BOT_EMOTION_MODEL}/resolve/${BOT_EMOTION_REVISION}/onnx/model_quantized.onnx`;
const MODEL_CACHE = "flow-like-emotion";
const CONTENT_TOKENS = BOT_EMOTION_MAX_LENGTH - 2;

// All text stays in this worker. Only the pinned model and runtime assets are fetched.
env.allowLocalModels = false;
env.useBrowserCache = true;
// Transformers.js omits revision on tokenizer metadata probes. Pin their URLs too.
env.remotePathTemplate = `{model}/resolve/${BOT_EMOTION_REVISION}/`;
if (env.backends.onnx.wasm) {
	env.backends.onnx.wasm.numThreads = 1;
	if (
		typeof process !== "undefined" &&
		process.env.NEXT_PUBLIC_BOT_EMOTION_BUNDLED_WASM === "true"
	) {
		// Use the bundled factory and same-origin WASM instead of the CDN defaults.
		env.backends.onnx.wasm.wasmPaths = undefined;
		ort.env.wasm.wasmPaths = undefined;
	}
}
ort.env.wasm.numThreads = 1;

function send(message: EmotionWorkerResponse) {
	self.postMessage(message);
}

function reportDownload(loaded: number, total: number) {
	if (total <= 0) return;
	send({ type: "progress", progress: Math.min(1, loaded / total) });
}

async function loadModelBytes(): Promise<ArrayBuffer> {
	const cache = typeof caches === "undefined" ? null : await caches.open(MODEL_CACHE);
	const cached = await cache?.match(MODEL_URL);
	if (cached) {
		reportDownload(1, 1);
		return cached.arrayBuffer();
	}
	const response = await fetch(MODEL_URL);
	if (!response.ok || !response.body) {
		throw new Error(`Emotion model download failed (${response.status})`);
	}
	const total = Number(response.headers.get("content-length")) || 0;
	const reader = response.body.getReader();
	const chunks: Uint8Array[] = [];
	let received = 0;
	while (true) {
		const { done, value } = await reader.read();
		if (done) break;
		if (!value) continue;
		chunks.push(value);
		received += value.byteLength;
		reportDownload(received, total);
	}
	const bytes = new Uint8Array(received);
	let offset = 0;
	for (const chunk of chunks) {
		bytes.set(chunk, offset);
		offset += chunk.byteLength;
	}
	if (cache) {
		await cache.put(
			MODEL_URL,
			new Response(bytes.slice().buffer, {
				headers: { "content-type": "application/octet-stream" },
			}),
		);
	}
	reportDownload(1, 1);
	return bytes.buffer;
}

function tokenIds(encoded: unknown): number[] {
	if (Array.isArray(encoded)) return encoded.map((id) => Number(id));
	if (encoded && typeof encoded === "object" && "data" in encoded) {
		return Array.from(
			(encoded as { data: ArrayLike<number | bigint> }).data,
			(id) => Number(id),
		);
	}
	throw new Error("Emotion tokenizer returned no token ids");
}

type Encoded = {
	input_ids: { data: ArrayLike<number | bigint>; dims: readonly number[] };
	attention_mask: { data: ArrayLike<number | bigint>; dims: readonly number[] };
};

function int64Tensor(source: Encoded["input_ids"]) {
	const data = new BigInt64Array(source.data.length);
	for (let index = 0; index < source.data.length; index++) {
		data[index] = BigInt(source.data[index] ?? 0);
	}
	return new ort.Tensor("int64", data, [...source.dims]);
}

let scorer: Promise<EmotionScorer> | undefined;

function loadScorer() {
	scorer ??= (async () => {
		send({ type: "progress" });
		const [tokenizer, modelBytes] = await Promise.all([
			AutoTokenizer.from_pretrained(BOT_EMOTION_MODEL, {
				revision: BOT_EMOTION_REVISION,
			}),
			loadModelBytes(),
		]);
		const session = await ort.InferenceSession.create(modelBytes, {
			executionProviders: ["wasm"],
		});
		const outputName = session.outputNames[0];
		if (!outputName) throw new Error("Emotion model has no output");
		return new EmotionScorer(async (text) => {
			const encodedIds = tokenIds(
				tokenizer.encode(text, { add_special_tokens: false }),
			);
			const recentText =
				encodedIds.length > CONTENT_TOKENS
					? tokenizer.decode(encodedIds.slice(-CONTENT_TOKENS), {
							skip_special_tokens: true,
						})
					: text;
			const inputs = tokenizer(recentText, {
				padding: true,
				truncation: true,
				max_length: BOT_EMOTION_MAX_LENGTH,
			}) as Encoded;
			const result = await session.run({
				input_ids: int64Tensor(inputs.input_ids),
				attention_mask: int64Tensor(inputs.attention_mask),
			});
			const output = result[outputName];
			if (!output) throw new Error("Emotion model returned no scores");
			return calibrateEmotionScores(
				scoresFromOutput(output.data as ArrayLike<number>),
			);
		});
	})();
	return scorer;
}

// The client serializes requests so one session owns inference and its context cache.
self.onmessage = async (event: MessageEvent<EmotionWorkerRequest>) => {
	const input = event.data;
	try {
		const model = await loadScorer();
		if (input.warmup) {
			send({ type: "ready" });
			return;
		}
		const scores = await model.score(input);
		send({ type: "result", id: input.id, scores });
	} catch (error) {
		console.error("Emotion model failed", error);
		send({ type: "error", id: input.id });
	}
};
