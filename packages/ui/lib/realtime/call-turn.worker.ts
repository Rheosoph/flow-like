import * as ort from "onnxruntime-web/webgpu";
import {
	type TurnDispatchPort,
	type TurnModels,
	createTurnDispatcher,
} from "./call-turn-dispatch";
import {
	TURN_MEL_BINS,
	TURN_MEL_FRAMES,
	createVadFramer,
	lastTurnWindow,
	turnLogMel,
} from "./call-turn-features";
import {
	TURN_MODEL_CACHE,
	TURN_MODEL_URLS,
	type TurnModelRole,
} from "./call-turn-protocol";

const bundledWasm =
	typeof process !== "undefined" &&
	process.env.NEXT_PUBLIC_BOT_EMOTION_BUNDLED_WASM === "true";
// Same-origin WASM where the app bundles it; elsewhere the runtime's own CDN copy, the one the
// emotion model's runtime loads there as well.
if (!bundledWasm && !ort.env.wasm.wasmPaths && ort.env.versions.web) {
	ort.env.wasm.wasmPaths = `https://cdn.jsdelivr.net/npm/onnxruntime-web@${ort.env.versions.web}/dist/`;
}
ort.env.wasm.numThreads = 1;

async function loadModelBytes(url: string): Promise<ArrayBuffer> {
	let cache: Cache | undefined;
	try {
		cache =
			typeof caches === "undefined"
				? undefined
				: await caches.open(TURN_MODEL_CACHE);
		const cached = await cache?.match(url);
		if (cached) return await cached.arrayBuffer();
	} catch {
		// Storage can be unavailable in a WebView or once its quota is exhausted.
	}
	const response = await fetch(url);
	if (!response.ok) {
		throw new Error(`Call model download failed (${response.status})`);
	}
	const bytes = await response.arrayBuffer();
	try {
		await cache?.put(
			url,
			new Response(bytes.slice(0), {
				headers: { "content-type": "application/octet-stream" },
			}),
		);
	} catch {
		// The downloaded bytes are usable even when they cannot be cached.
	}
	return bytes;
}

interface SpeechStream {
	framer: ReturnType<typeof createVadFramer>;
	state: ort.Tensor;
	last: number;
}

const freshState = () =>
	new ort.Tensor("float32", new Float32Array(2 * 128), [2, 1, 128]);
const SAMPLE_RATE = new ort.Tensor("int64", BigInt64Array.from([16_000n]), []);
const sessions = new Map<TurnModelRole, ort.InferenceSession>();
const streams = new Map<string, SpeechStream>();

function streamFor(id: string): SpeechStream {
	let stream = streams.get(id);
	if (!stream) {
		stream = { framer: createVadFramer(), state: freshState(), last: 0 };
		streams.set(id, stream);
	}
	return stream;
}

async function speechProbability(
	session: ort.InferenceSession,
	id: string,
	samples: Float32Array,
): Promise<number> {
	const stream = streamFor(id);
	let top = -1;
	for (const input of stream.framer.push(samples)) {
		const result = await session.run({
			input: new ort.Tensor("float32", input, [1, input.length]),
			state: stream.state,
			sr: SAMPLE_RATE,
		});
		stream.state = result.stateN as ort.Tensor;
		top = Math.max(top, (result.output.data as Float32Array)[0]);
	}
	if (top >= 0) stream.last = top;
	return stream.last;
}

async function turnProbability(
	session: ort.InferenceSession,
	samples: Float32Array,
): Promise<number> {
	const features = turnLogMel(lastTurnWindow(samples));
	const result = await session.run({
		input_features: new ort.Tensor("float32", features, [
			1,
			TURN_MEL_BINS,
			TURN_MEL_FRAMES,
		]),
	});
	return (result.logits.data as Float32Array)[0];
}

function sessionOf(role: TurnModelRole): ort.InferenceSession {
	const session = sessions.get(role);
	if (!session) throw new Error(`The ${role} model is not loaded`);
	return session;
}

/** This worker runs whichever model its tabs ask for; the page starts one worker per model. */
const models: TurnModels = {
	async load(role) {
		if (sessions.has(role)) return;
		const bytes = await loadModelBytes(TURN_MODEL_URLS[role]);
		const session = await ort.InferenceSession.create(bytes, {
			executionProviders: ["wasm"],
			// The turn model runs once per utterance: without the arena its working memory is
			// returned after each run instead of staying reserved.
			...(role === "turn"
				? { enableCpuMemArena: false, enableMemPattern: false }
				: {}),
		});
		// The first run pays for lazy initialisation, so do it before a caller waits on an answer.
		if (role === "turn") {
			await turnProbability(session, new Float32Array(1600)).catch(() => {});
		}
		sessions.set(role, session);
	},
	speech: (stream, samples) =>
		speechProbability(sessionOf("speech"), stream, samples),
	turn: (samples) => turnProbability(sessionOf("turn"), samples),
	reset(stream) {
		streams.delete(stream);
	},
	async release() {
		streams.clear();
		const open = [...sessions.values()];
		sessions.clear();
		await Promise.all(open.map((session) => session.release().catch(() => {})));
	},
};

const scope = self as unknown as TurnDispatchPort & {
	onconnect?: ((event: MessageEvent) => void) | null;
};
// Closing every port leaves a SharedWorker's WASM heap alive until its tabs close, so the scope
// ends itself once no call needs the models.
const dispatcher = createTurnDispatcher(models, () => scope.close?.());

if ("onconnect" in scope) {
	scope.onconnect = (event) => {
		const port = event.ports[0];
		if (port) dispatcher.attach(port);
	};
} else {
	dispatcher.attach(scope);
}
