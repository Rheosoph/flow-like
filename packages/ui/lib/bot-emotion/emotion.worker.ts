import {
	AutoModelForSequenceClassification,
	AutoTokenizer,
	env,
} from "@huggingface/transformers";
import { calibrateEmotionScores } from "./calibration";
import {
	BOT_EMOTION_MODEL,
	BOT_EMOTION_REVISION,
	type EmotionScores,
	type EmotionWorkerRequest,
	type EmotionWorkerResponse,
} from "./protocol";
import { EmotionScorer } from "./scoring";

// All text stays in this worker. Only the pinned model and runtime assets are fetched.
env.allowLocalModels = false;
env.useBrowserCache = true;
// Transformers.js omits revision on tokenizer metadata probes. Pin their URLs too.
env.remotePathTemplate = `{model}/resolve/${BOT_EMOTION_REVISION}/`;
if (env.backends.onnx.wasm) env.backends.onnx.wasm.numThreads = 1;

function send(message: EmotionWorkerResponse) {
	self.postMessage(message);
}

let scorer: Promise<EmotionScorer> | undefined;

function loadScorer() {
	scorer ??= (async () => {
		send({ type: "progress" });
		let reportedPercent = -1;
		const [tokenizer, model] = await Promise.all([
			AutoTokenizer.from_pretrained(BOT_EMOTION_MODEL, {
				revision: BOT_EMOTION_REVISION,
			}),
			AutoModelForSequenceClassification.from_pretrained(BOT_EMOTION_MODEL, {
				revision: BOT_EMOTION_REVISION,
				dtype: "q8",
				device: "wasm",
				progress_callback: (event) => {
					if (event.status === "progress" && event.file.endsWith(".onnx")) {
						const percent = Math.floor(
							Math.min(100, Math.max(0, event.progress)),
						);
						if (!Number.isFinite(percent) || percent === reportedPercent)
							return;
						reportedPercent = percent;
						send({
							type: "progress",
							progress: percent / 100,
						});
					}
				},
			}),
		]);
		return new EmotionScorer(async (text) => {
			// Preserve the newest words when token-heavy scripts exceed the character cap.
			const tokens = tokenizer.encode(text, {
				add_special_tokens: false,
			});
			const recentText =
				tokens.length > 254
					? tokenizer.decode(tokens.slice(-254), {
							skip_special_tokens: true,
						})
					: text;
			const inputs = tokenizer(recentText, {
				padding: true,
				truncation: true,
				max_length: 256,
			});
			const output = await model(inputs);
			const probabilities = output.logits.sigmoid().data;
			const config = model.config;
			const labels = "id2label" in config ? config.id2label : undefined;
			if (!labels || typeof labels !== "object")
				throw new Error("Emotion labels missing");
			const scores: EmotionScores = {};
			for (let index = 0; index < probabilities.length; index++) {
				const label = (labels as Record<string, unknown>)[index];
				if (typeof label === "string")
					scores[label.toLowerCase()] = Number(probabilities[index]);
			}
			return calibrateEmotionScores(scores);
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
	} catch {
		send({ type: "error", id: input.id });
	}
};
