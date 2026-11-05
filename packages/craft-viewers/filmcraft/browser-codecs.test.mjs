import assert from "node:assert/strict";
import test from "node:test";

globalThis.addEventListener ??= () => {};
const codecs = await import("./browser-codecs.js");

function mp4Box(type, ...payloads) {
	const result = Buffer.concat([
		Buffer.alloc(4),
		Buffer.from(type),
		...payloads,
	]);
	result.writeUInt32BE(result.length);
	return result;
}

function sampleTrack(codec, offset, wide = false) {
	const sampleDescription = Buffer.alloc(8);
	sampleDescription.writeUInt32BE(1, 4);
	const sizes = Buffer.alloc(12);
	sizes.writeUInt32BE(2, 8);
	const offsets = Buffer.alloc(wide ? 16 : 12);
	offsets.writeUInt32BE(1, 4);
	if (wide) offsets.writeBigUInt64BE(BigInt(offset), 8);
	else offsets.writeUInt32BE(offset, 8);
	return mp4Box(
		"trak",
		mp4Box(
			"mdia",
			mp4Box(
				"minf",
				mp4Box(
					"stbl",
					mp4Box("stsd", sampleDescription, mp4Box(codec)),
					mp4Box("stsz", sizes),
					mp4Box(wide ? "co64" : "stco", offsets),
				),
			),
		),
	);
}

for (const wide of [false, true]) {
	test(`AAC roll groups preserve fast-start ${wide ? "64" : "32"}-bit sample offsets`, () => {
		const ftyp = mp4Box("ftyp", Buffer.from("isom"));
		const payload = Buffer.from("encoded media");
		let moov = mp4Box(
			"moov",
			sampleTrack("avc1", 0, wide),
			sampleTrack("mp4a", 0, wide),
		);
		const oldOffset = ftyp.length + moov.length + 8;
		moov = mp4Box(
			"moov",
			sampleTrack("avc1", oldOffset, wide),
			sampleTrack("mp4a", oldOffset, wide),
		);
		const input = Buffer.concat([ftyp, moov, mp4Box("mdat", payload)]);
		const before = Buffer.from(input);
		const output = Buffer.from(codecs.addAacRollGroups(input));
		assert.deepEqual(input, before, "the original muxed bytes stay intact");
		assert.equal(output.length, input.length + 54);
		assert.equal(output.indexOf("mdat") + 4, oldOffset + 54);
		assert.deepEqual(output.subarray(oldOffset + 54), payload);
		let cursor = 0;
		for (let track = 0; track < 2; track++) {
			cursor = output.indexOf(wide ? "co64" : "stco", cursor) + 12;
			assert.equal(
				wide
					? Number(output.readBigUInt64BE(cursor))
					: output.readUInt32BE(cursor),
				oldOffset + 54,
			);
		}
		const sgpd = output.indexOf("sgpd");
		const sbgp = output.indexOf("sbgp");
		assert.equal(output.readInt16BE(sgpd + 20), -1);
		assert.equal(output.readUInt32BE(sbgp + 16), 2);
		assert.equal(
			output.indexOf("sgpd", sgpd + 4),
			-1,
			"video track gets no audio roll group",
		);
		assert.deepEqual(
			Buffer.from(codecs.addAacRollGroups(output)),
			output,
			"groups are added only once",
		);
	});
}

test("MP4 priming keeps trailing metadata offsets and video-only files unchanged", () => {
	const mdat = mp4Box("mdat", Buffer.from("encoded media"));
	const input = Buffer.concat([mdat, mp4Box("moov", sampleTrack("mp4a", 8))]);
	const output = Buffer.from(codecs.addAacRollGroups(input));
	assert.equal(output.readUInt32BE(output.indexOf("stco") + 12), 8);
	assert.deepEqual(output.subarray(0, mdat.length), mdat);
	const video = Buffer.concat([mdat, mp4Box("moov", sampleTrack("avc1", 8))]);
	assert.deepEqual(Buffer.from(codecs.addAacRollGroups(video)), video);
	assert.throws(
		() => codecs.addAacRollGroups(Uint8Array.of(0, 0, 0, 0)),
		/invalid sample tables/,
	);
});

test("MP4 results survive growth of the caller's WASM memory", () => {
	const video = mp4Box("moov", sampleTrack("avc1", 0));
	const audio = codecs.addAacRollGroups(
		Buffer.concat([
			mp4Box("mdat", Buffer.from("encoded media")),
			mp4Box("moov", sampleTrack("mp4a", 8)),
		]),
	);
	for (const original of [video, audio]) {
		const memory = new WebAssembly.Memory({ initial: 1 });
		const input = new Uint8Array(memory.buffer, 0, original.length);
		input.set(original);
		const output = codecs.addAacRollGroups(input);
		assert.notEqual(output.buffer, input.buffer);
		memory.grow(1);
		assert.equal(input.byteLength, 0);
		assert.deepEqual(Buffer.from(output), Buffer.from(original));
	}
});

const WEBKIT_AAC_DESCRIPTION = Uint8Array.of(
	0x03,
	0x80,
	0x80,
	0x80,
	0x22,
	0,
	0,
	0,
	0x04,
	0x80,
	0x80,
	0x80,
	0x14,
	0x40,
	0x14,
	0,
	0x18,
	0,
	0,
	0,
	0,
	0,
	0,
	0,
	0,
	0,
	0x05,
	0x80,
	0x80,
	0x80,
	0x02,
	0x11,
	0x90,
	0x06,
	0x80,
	0x80,
	0x80,
	0x01,
	0x02,
);

function platform(t, failures = {}) {
	const names = [
		"VideoEncoder",
		"AudioEncoder",
		"VideoFrame",
		"AudioData",
		"AudioContext",
		"AudioDecoder",
		"EncodedAudioChunk",
	];
	const originals = names.map((name) =>
		Object.getOwnPropertyDescriptor(globalThis, name),
	);
	t.after(() =>
		names.forEach((name, index) => {
			if (originals[index])
				Object.defineProperty(globalThis, name, originals[index]);
			else delete globalThis[name];
		}),
	);
	const instances = [];
	const inputs = [];
	const contexts = [];
	const decoders = [];
	const decodedAudio = [];
	const decodedChunks = [];
	const muxed = [];
	let probe;
	const calibrationSamples = (trim = 0) => {
		const length =
			probe.numberOfFrames + (failures.calibrationPriming || 0) - trim;
		const samples = new Float32Array(length);
		if (!failures.calibrationNoise)
			samples.set(
				probe.data.slice(0, Math.min(probe.numberOfFrames, length)),
				failures.calibrationPriming || 0,
			);
		return samples;
	};
	function encoder(kind) {
		return class {
			static async isConfigSupported() {
				return { supported: !failures[`${kind}Unsupported`] };
			}
			constructor(callbacks) {
				this.callbacks = callbacks;
				this.state = "unconfigured";
				this.encodeQueueSize = 0;
				this.kind = kind;
				instances.push(this);
			}
			configure() {
				if (failures[`${kind}Configure`])
					throw new Error(`${kind} configuration failed`);
				this.state = "configured";
			}
			encode(input) {
				inputs.push({ kind, bytes: [...input.data] });
				this.input = input;
			}
			async flush() {
				if (failures[`${kind}Flush`]) throw new Error(`${kind} flush failed`);
				if (kind === "audio" && this.input?.numberOfFrames === 8192) {
					probe = this.input;
					output(
						this,
						failures.calibrationTimestamp ?? 0,
						((probe.numberOfFrames + (failures.calibrationPriming || 0)) *
							1_000_000) /
							probe.sampleRate,
						failures.calibrationDescription
							? Uint8Array.of(0x12, 0x10)
							: failures.calibrationMetadata || Uint8Array.of(0x11, 0x90),
					);
				}
			}
			close() {
				assert.notEqual(this.state, "closed");
				this.state = "closed";
			}
		};
	}
	globalThis.VideoEncoder = encoder("video");
	globalThis.AudioEncoder = encoder("audio");
	globalThis.VideoFrame = class {
		constructor(data) {
			this.data = data.slice();
		}
		close() {}
	};
	globalThis.AudioData = class {
		constructor({ data, sampleRate, numberOfFrames }) {
			this.data = data.slice();
			this.sampleRate = sampleRate;
			this.numberOfFrames = numberOfFrames;
		}
		close() {}
	};
	globalThis.AudioContext = class {
		constructor() {
			this.closed = false;
			contexts.push(this);
		}
		async decodeAudioData() {
			if (failures.calibrationDecode) throw new Error("Decoding failed");
			const samples = calibrationSamples(failures.calibrationTrim || 0);
			return {
				sampleRate: probe.sampleRate,
				length: samples.length,
				getChannelData: () => samples,
			};
		}
		async close() {
			this.closed = true;
			if (failures.contextClose) throw new Error("Context close failed");
		}
	};
	globalThis.EncodedAudioChunk = class {
		constructor(options) {
			Object.assign(this, options);
			decodedChunks.push(this);
		}
	};
	globalThis.AudioDecoder = class {
		static async isConfigSupported() {
			return { supported: !failures.packetUnsupported };
		}
		constructor(callbacks) {
			this.callbacks = callbacks;
			this.state = "unconfigured";
			decoders.push(this);
		}
		configure(config) {
			this.config = config;
			this.state = "configured";
		}
		decode() {}
		async flush() {
			if (failures.packetFlush) throw new Error("AAC decoder flush failed");
			if (failures.packetError) {
				this.callbacks.error(new Error("AAC decoding failed"));
				return;
			}
			const samples = calibrationSamples(failures.packetTrim || 0);
			for (const part of [samples.slice(0, 2048), samples.slice(2048)]) {
				const data = {
					closed: false,
					sampleRate: failures.packetSampleRate || this.config.sampleRate,
					numberOfChannels: this.config.numberOfChannels,
					numberOfFrames: part.length,
					copyTo(target, options) {
						assert.deepEqual(options, { planeIndex: 0, format: "f32-planar" });
						if (failures.packetCopy) throw new Error("AAC sample copy failed");
						target.set(part);
					},
					close() {
						assert.equal(this.closed, false);
						this.closed = true;
					},
				};
				decodedAudio.push(data);
				this.callbacks.output(data);
			}
		}
		close() {
			assert.notEqual(this.state, "closed");
			this.state = "closed";
		}
	};
	codecs.setCalibrationMuxer((probe) => {
		muxed.push(probe);
		return Uint8Array.of(1);
	});
	return {
		instances,
		inputs,
		contexts,
		decoders,
		decodedAudio,
		decodedChunks,
		muxed,
	};
}

test("unsupported platform codecs fail before allocating an encoder", async (t) => {
	const failures = { videoUnsupported: true };
	const state = platform(t, failures);
	await assert.rejects(
		codecs.createEncoder(16, 16, 30, 48000, 2),
		/cannot encode H.264/,
	);
	assert.equal(state.instances.length, 0);
	failures.videoUnsupported = false;
	failures.audioUnsupported = true;
	await assert.rejects(
		codecs.createEncoder(16, 16, 30, 48000, 2),
		/cannot encode AAC/,
	);
	assert.equal(state.instances.length, 0);
});

test("configuration failures close every allocated platform encoder", async (t) => {
	const failures = { videoConfigure: true };
	const state = platform(t, failures);
	await assert.rejects(
		codecs.createEncoder(16, 16, 30, 48000, 2),
		/video configuration failed/,
	);
	assert.deepEqual(
		state.instances.map((instance) => instance.state),
		["closed"],
	);
	failures.videoConfigure = false;
	failures.audioConfigure = true;
	await assert.rejects(
		codecs.createEncoder(16, 16, 30, 48000, 2),
		/audio configuration failed/,
	);
	assert.deepEqual(
		state.instances.map((instance) => instance.state),
		["closed", "closed", "closed"],
	);
});

test("async encoding retains pixels and PCM when WASM backing buffers detach", async (t) => {
	const state = platform(t);
	const id = await codecs.createEncoder(1, 1, 30, 48000, 2);
	const pixels = Uint8Array.of(1, 2, 3, 255);
	const video = codecs.encodeFrame(id, pixels, 1, 1, 0, 33333);
	structuredClone(pixels.buffer, { transfer: [pixels.buffer] });
	await video;
	const samples = Float32Array.of(0.25, -0.5);
	const audio = codecs.encodeAudio(id, samples, 1, 0);
	structuredClone(samples.buffer, { transfer: [samples.buffer] });
	await audio;
	assert.deepEqual(state.inputs, [
		{ kind: "video", bytes: [1, 2, 3, 255] },
		{ kind: "audio", bytes: [0.25, -0.5] },
	]);
	codecs.cancelEncoder(id);
	codecs.cancelEncoder(id);
	assert.ok(state.instances.every((instance) => instance.state === "closed"));
});

test("finishing failures close encoders and cannot return a partial export", async (t) => {
	const state = platform(t, { videoFlush: true });
	const id = await codecs.createEncoder(16, 16, 30, 48000, 2);
	await assert.rejects(codecs.finishEncoder(id), /video flush failed/);
	assert.ok(state.instances.every((instance) => instance.state === "closed"));
	await assert.rejects(codecs.finishEncoder(id), /no longer active/);
});

function output(
	encoder,
	timestamp,
	duration,
	description = Uint8Array.of(0x11, 0x90),
) {
	encoder.callbacks.output(
		{
			byteLength: 1,
			timestamp,
			duration,
			type: "key",
			copyTo(bytes) {
				bytes[0] = 1;
			},
		},
		{ decoderConfig: { description } },
	);
}

test("dropped video frames cannot produce a shortened successful export", async (t) => {
	const state = platform(t);
	const id = await codecs.createEncoder(1, 1, 30, 48000, 0);
	await codecs.encodeFrame(id, new Uint8Array(4), 1, 1, 0, 33333);
	await codecs.encodeFrame(id, new Uint8Array(4), 1, 1, 33333, 33334);
	output(state.instances[0], 0, 33333);
	await assert.rejects(codecs.finishEncoder(id), /dropped video frames/);
	assert.equal(state.instances[0].state, "closed");
});

test("audio gaps fail export while AAC priming and end padding are accepted", async (t) => {
	const state = platform(t);
	let id = await codecs.createEncoder(1, 1, 30, 48000, 2);
	await codecs.encodeFrame(id, new Uint8Array(4), 1, 1, 0, 33333);
	await codecs.encodeAudio(id, new Float32Array(3200), 1600, 0);
	output(state.instances[0], 0, 33333);
	output(state.instances[1], -21333, 21333);
	output(state.instances[1], 0, 21333);
	output(state.instances[1], 21333, 21333);
	const exported = await codecs.finishEncoder(id);
	assert.equal(exported.video.length, 1);
	assert.equal(exported.audio.length, 3);
	const next = state.instances.length;
	id = await codecs.createEncoder(1, 1, 30, 48000, 2);
	await codecs.encodeFrame(id, new Uint8Array(4), 1, 1, 0, 33333);
	await codecs.encodeAudio(id, new Float32Array(3200), 1600, 0);
	output(state.instances[next], 0, 33333);
	output(state.instances[next + 1], 0, 21333);
	output(state.instances[next + 1], 42666, 21333);
	await assert.rejects(codecs.finishEncoder(id), /incomplete audio timing/);
	assert.ok(state.instances.every((instance) => instance.state === "closed"));
});

test("implicit calibration trimming fails rather than guessing an audio offset", async (t) => {
	const state = platform(t, { calibrationTrim: 128, packetTrim: 128 });
	const id = await codecs.createEncoder(1, 1, 30, 48000, 2);
	await codecs.encodeFrame(id, new Uint8Array(4), 1, 1, 0, 33333);
	await codecs.encodeAudio(id, new Float32Array(3200), 1600, 0);
	output(state.instances[0], 0, 33333);
	output(state.instances[1], 0, 42667);
	await assert.rejects(
		codecs.finishEncoder(id),
		/trims audio calibration samples/,
	);
	assert.ok(state.instances.every((instance) => instance.state === "closed"));
	assert.ok(state.contexts.every((context) => context.closed));
});

test("calibration reports a measured nonzero delay for the MP4 edit", async (t) => {
	const state = platform(t, { calibrationPriming: 2112 });
	const id = await codecs.createEncoder(1, 1, 30, 48000, 2);
	await codecs.encodeFrame(id, new Uint8Array(4), 1, 1, 0, 33333);
	await codecs.encodeAudio(id, new Float32Array(3200), 1600, 0);
	output(state.instances[0], 0, 33333);
	output(state.instances[1], 0, 100000);
	const exported = await codecs.finishEncoder(id);
	assert.equal(exported.audioPriming, 2112);
	assert.ok(state.instances.every((instance) => instance.state === "closed"));
	assert.ok(state.contexts.every((context) => context.closed));
});

async function finishAudioCalibration(state, description) {
	const next = state.instances.length;
	const id = await codecs.createEncoder(1, 1, 30, 48000, 2);
	await codecs.encodeFrame(id, new Uint8Array(4), 1, 1, 0, 33333);
	await codecs.encodeAudio(id, new Float32Array(3200), 1600, 0);
	output(state.instances[next], 0, 33333);
	output(state.instances[next + 1], 0, 100000, description);
	return codecs.finishEncoder(id);
}

test("WebKit AAC descriptors produce bare configs and measure delay despite MP4 trimming", async (t) => {
	const backing = new Uint8Array(WEBKIT_AAC_DESCRIPTION.length + 4);
	backing.set(WEBKIT_AAC_DESCRIPTION, 2);
	const description = new DataView(
		backing.buffer,
		2,
		WEBKIT_AAC_DESCRIPTION.length,
	);
	const state = platform(t, {
		calibrationMetadata: description,
		calibrationPriming: 2112,
		calibrationTrim: 2112,
	});
	const exported = await finishAudioCalibration(state, description);
	assert.equal(exported.audioPriming, 2112);
	assert.deepEqual(exported.audioDescription, Uint8Array.of(0x11, 0x90));
	assert.deepEqual(state.muxed[0].description, Uint8Array.of(0x11, 0x90));
	assert.deepEqual(
		state.decoders[0].config.description,
		Uint8Array.of(0x11, 0x90),
	);
	backing.fill(0);
	assert.deepEqual(exported.audioDescription, Uint8Array.of(0x11, 0x90));
	assert.ok(state.decodedAudio.every((data) => data.closed));
});

test("AAC descriptor parsing skips optional ES fields and bounded decoder extensions", async (t) => {
	const descriptor = (tag, bytes) => [
		tag,
		0x80,
		0x80,
		0x80,
		bytes.length,
		...bytes,
	];
	const config = descriptor(0x04, [
		...WEBKIT_AAC_DESCRIPTION.slice(13, 26),
		...descriptor(0x7f, [0x05, 0x02, 0xff, 0xff]),
		...descriptor(0x05, [0x11, 0x90]),
	]);
	const description = Uint8Array.from(
		descriptor(0x03, [
			0,
			0,
			0xe0,
			0,
			0x05,
			3,
			0x05,
			0,
			0xff,
			0,
			0x05,
			...config,
			...descriptor(0x06, [0x02]),
		]),
	);
	const state = platform(t, { calibrationMetadata: description });
	const exported = await finishAudioCalibration(state, description);
	assert.deepEqual(exported.audioDescription, Uint8Array.of(0x11, 0x90));
});

for (const [name, description] of [
	["truncated tag", Uint8Array.of(0x03)],
	["unterminated length", Uint8Array.of(0x03, 0x80, 0x80, 0x80, 0x80)],
	["truncated payload", WEBKIT_AAC_DESCRIPTION.slice(0, -1)],
	["truncated optional URL", Uint8Array.of(0x03, 0x05, 0, 0, 0x40, 10, 0)],
	[
		"child beyond parent",
		Uint8Array.from(WEBKIT_AAC_DESCRIPTION, (value, index) =>
			index === 30 ? 9 : value,
		),
	],
]) {
	test(`${name} rejects malformed AAC metadata before muxing`, async (t) => {
		const state = platform(t);
		await assert.rejects(
			finishAudioCalibration(state, description),
			/invalid AAC configuration/,
		);
		assert.equal(state.muxed.length, 0);
		assert.ok(state.instances.every((instance) => instance.state === "closed"));
	});
}

test("AAC packet decoding recovers a rejected MP4 probe and measures its actual delay", async (t) => {
	const state = platform(t, {
		calibrationDecode: true,
		calibrationPriming: 2112,
		calibrationTimestamp: -44000,
		contextClose: true,
	});
	const exported = await finishAudioCalibration(state);
	assert.equal(exported.audioPriming, 2112);
	assert.equal(state.decodedChunks.length, 1);
	assert.equal(state.decodedChunks[0].timestamp, -44000);
	assert.equal(state.decodedChunks[0].type, "key");
	assert.deepEqual(state.decoders[0].config, {
		codec: "mp4a.40.2",
		sampleRate: 48000,
		numberOfChannels: 2,
		description: Uint8Array.of(0x11, 0x90),
	});
	assert.equal(state.decodedAudio.length, 2);
	assert.ok(state.decodedAudio.every((data) => data.closed));
	assert.ok(state.decoders.every((decoder) => decoder.state === "closed"));
	assert.ok(state.instances.every((instance) => instance.state === "closed"));
	assert.ok(state.contexts.every((context) => context.closed));
});

for (const [failure, value, expected] of [
	["packetUnsupported", true, /cannot decode the AAC synchronization check/],
	["packetFlush", true, /AAC decoder flush failed/],
	["packetError", true, /AAC decoding failed/],
	["packetSampleRate", 44100, /invalid AAC calibration samples/],
	["packetCopy", true, /AAC sample copy failed/],
]) {
	test(`${failure} reports both decoder failures and releases calibration resources`, async (t) => {
		const state = platform(t, {
			calibrationDecode: true,
			contextClose: true,
			[failure]: value,
		});
		await assert.rejects(finishAudioCalibration(state), (error) => {
			assert.match(error.message, /could not verify AAC audio synchronization/);
			assert.match(error.message, /MP4 decoder: Decoding failed/);
			assert.match(error.message, expected);
			return true;
		});
		assert.ok(state.decodedAudio.every((data) => data.closed));
		assert.ok(state.decoders.every((decoder) => decoder.state === "closed"));
		assert.ok(state.instances.every((instance) => instance.state === "closed"));
		assert.ok(state.contexts.every((context) => context.closed));
	});
}

for (const [failure, value, expected] of [
	["packetTrim", 128, /trims audio calibration samples/],
	["calibrationNoise", true, /could not be synchronized reliably/],
]) {
	test(`${failure} cannot bypass synchronization validation through AAC packet fallback`, async (t) => {
		const failures = { calibrationDecode: true, [failure]: value };
		const state = platform(t, failures);
		await assert.rejects(finishAudioCalibration(state), expected);
		assert.ok(state.decodedAudio.every((data) => data.closed));
		assert.ok(state.decoders.every((decoder) => decoder.state === "closed"));
		assert.ok(state.instances.every((instance) => instance.state === "closed"));
		delete failures[failure];
		assert.equal((await finishAudioCalibration(state)).audioPriming, 0);
		assert.equal(
			state.decoders.length,
			2,
			"Failed calibration must not be cached",
		);
	});
}

for (const [failure, expected] of [
	["calibrationNoise", /could not be synchronized reliably/],
	["calibrationDescription", /changed its AAC configuration/],
]) {
	test(`${failure} rejects export and releases calibration resources`, async (t) => {
		const state = platform(t, { [failure]: true });
		const id = await codecs.createEncoder(1, 1, 30, 48000, 2);
		await codecs.encodeFrame(id, new Uint8Array(4), 1, 1, 0, 33333);
		await codecs.encodeAudio(id, new Float32Array(3200), 1600, 0);
		output(state.instances[0], 0, 33333);
		output(state.instances[1], 0, 100000);
		await assert.rejects(codecs.finishEncoder(id), expected);
		assert.ok(state.instances.every((instance) => instance.state === "closed"));
		assert.ok(state.contexts.every((context) => context.closed));
	});
}
