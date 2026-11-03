const sources = new Map();
const encoders = new Map();
let nextId = 1;
let sourceReady = () => {};
let calibrationMuxer;
const audioCalibrations = new Map();
const MAX_BYTES = 200 * 1024 * 1024;
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const reason = (error) => error?.message || String(error);

// Apple readers need roll groups alongside AAC edit lists. The shared upstream
// muxer does not write them; preserve fast-start sample offsets when adding them.
export function addAacRollGroups(input) {
	const invalid = () =>
		new Error("The exported MP4 has invalid sample tables.");
	const kind = (bytes, at) =>
		String.fromCharCode(...bytes.subarray(at, at + 4));
	const view = (bytes) =>
		new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
	const boxes = (bytes, start = 0) => {
		const result = [];
		for (let at = start; at < bytes.length; ) {
			if (at + 8 > bytes.length) throw invalid();
			const size = view(bytes).getUint32(at);
			if (size < 8 || size > bytes.length - at) throw invalid();
			result.push({ at, end: at + size, type: kind(bytes, at + 4) });
			at += size;
		}
		return result;
	};
	const combine = (parts) => {
		const size = parts.reduce((total, part) => total + part.length, 0);
		if (size > MAX_BYTES)
			throw new Error("The exported video exceeds 200 MiB.");
		const result = new Uint8Array(size);
		let at = 0;
		for (const part of parts) {
			result.set(part, at);
			at += part.length;
		}
		return result;
	};
	const containers = new Set(["moov", "trak", "mdia", "minf", "stbl"]);
	const rewrite = (box) => {
		const type = kind(box, 4);
		if (!containers.has(type)) return box;
		const children = boxes(box, 8);
		const parts = [
			box.subarray(0, 8),
			...children.map(({ at, end }) => rewrite(box.subarray(at, end))),
		];
		if (type === "stbl") {
			const stsd = children.find((child) => child.type === "stsd");
			if (
				stsd &&
				stsd.end - stsd.at >= 24 &&
				kind(box, stsd.at + 20) === "mp4a"
			) {
				const groups = children.filter(
					({ at, end, type }) =>
						(type === "sgpd" || type === "sbgp") &&
						end - at >= 16 &&
						kind(box, at + 12) === "roll",
				);
				if (
					groups.length &&
					(groups.length !== 2 || groups[0].type === groups[1].type)
				)
					throw invalid();
				if (!groups.length) {
					const stsz = children.find((child) => child.type === "stsz");
					if (!stsz || stsz.end - stsz.at < 20) throw invalid();
					const count = view(box).getUint32(stsz.at + 16);
					if (count) {
						const groups = Uint8Array.of(
							0,
							0,
							0,
							26,
							115,
							103,
							112,
							100,
							1,
							0,
							0,
							0,
							114,
							111,
							108,
							108,
							0,
							0,
							0,
							2,
							0,
							0,
							0,
							1,
							255,
							255,
							0,
							0,
							0,
							28,
							115,
							98,
							103,
							112,
							0,
							0,
							0,
							0,
							114,
							111,
							108,
							108,
							0,
							0,
							0,
							1,
							0,
							0,
							0,
							0,
							0,
							0,
							0,
							1,
						);
						view(groups).setUint32(46, count);
						parts.push(groups);
					}
				}
			}
		}
		const result = combine(parts);
		view(result).setUint32(0, result.length);
		return result;
	};
	if (!(input instanceof Uint8Array) || input.length > MAX_BYTES)
		throw invalid();
	const top = boxes(input);
	const movies = top.filter((box) => box.type === "moov");
	if (movies.length !== 1) throw invalid();
	const movie = movies[0];
	const moov = rewrite(input.subarray(movie.at, movie.end));
	const delta = moov.length - (movie.end - movie.at);
	// Returning to Rust may grow WASM memory and detach the borrowed input view.
	if (!delta) return new Uint8Array(input);
	const relocate = (box) => {
		const type = kind(box, 4);
		if (containers.has(type)) {
			for (const { at, end } of boxes(box, 8)) relocate(box.subarray(at, end));
		} else if (type === "stco" || type === "co64") {
			if (box.length < 16) throw invalid();
			const data = view(box);
			const width = type === "stco" ? 4 : 8;
			const count = data.getUint32(12);
			if (count > (box.length - 16) / width) throw invalid();
			for (let index = 0; index < count; index++) {
				const at = 16 + index * width;
				const old =
					width === 4 ? BigInt(data.getUint32(at)) : data.getBigUint64(at);
				if (old >= BigInt(movie.at) && old < BigInt(movie.end)) throw invalid();
				if (old >= BigInt(movie.end)) {
					const next = old + BigInt(delta);
					if (next > (width === 4 ? 0xffffffffn : 0xffffffffffffffffn))
						throw invalid();
					if (width === 4) data.setUint32(at, Number(next));
					else data.setBigUint64(at, next);
				}
			}
		}
	};
	relocate(moov);
	return combine([
		input.subarray(0, movie.at),
		moov,
		input.subarray(movie.end),
	]);
}

export function onSourceReady(callback) {
	sourceReady = callback;
}

export function setCalibrationMuxer(callback) {
	calibrationMuxer = callback;
	audioCalibrations.clear();
}

function aacAudioSpecificConfig(bytes) {
	const invalid = () =>
		new Error("The browser returned an invalid AAC configuration.");
	const descriptor = (start, end) => {
		let offset = start;
		if (offset >= end) throw invalid();
		const tag = bytes[offset++];
		let length = 0;
		for (let index = 0; index < 4; index++) {
			if (offset >= end) throw invalid();
			const byte = bytes[offset++];
			length = length * 128 + (byte & 0x7f);
			if (!(byte & 0x80)) {
				if (offset + length > end) throw invalid();
				return { tag, start: offset, end: offset + length };
			}
		}
		throw invalid();
	};
	let specific = bytes;
	// WebKit can return an ES_Descriptor; the MP4 writer requires its bare AAC config.
	if (bytes[0] === 0x03 || bytes[0] === 0x04) {
		let current = descriptor(0, bytes.length);
		if (current.end !== bytes.length) throw invalid();
		if (current.tag === 0x03) {
			let offset = current.start + 3;
			if (offset > current.end) throw invalid();
			const flags = bytes[current.start + 2];
			if (flags & 0x80) offset += 2;
			if (flags & 0x40) {
				if (offset >= current.end) throw invalid();
				offset += 1 + bytes[offset];
			}
			if (flags & 0x20) offset += 2;
			current = descriptor(offset, current.end);
		}
		if (
			current.tag !== 0x04 ||
			current.start + 13 > current.end ||
			bytes[current.start] !== 0x40 ||
			bytes[current.start + 1] >> 2 !== 5
		)
			throw invalid();
		let offset = current.start + 13;
		specific = null;
		while (offset < current.end) {
			const child = descriptor(offset, current.end);
			if (child.tag === 0x05) {
				if (specific) throw invalid();
				specific = bytes.slice(child.start, child.end);
			}
			offset = child.end;
		}
	}
	if (!specific || specific.length < 2 || specific[0] >> 3 !== 2)
		throw invalid();
	return specific;
}

async function decodeCalibrationPackets(packets, config, description) {
	if (!globalThis.AudioDecoder || !globalThis.EncodedAudioChunk)
		throw new Error("This browser does not provide AAC packet decoding.");
	const decoderConfig = {
		codec: config.codec,
		sampleRate: config.sampleRate,
		numberOfChannels: config.numberOfChannels,
		description,
	};
	if (!(await AudioDecoder.isConfigSupported(decoderConfig)).supported)
		throw new Error(
			"This browser cannot decode the AAC synchronization check.",
		);
	const parts = [];
	let length = 0;
	let failure;
	const decoder = new AudioDecoder({
		output(data) {
			try {
				if (failure) return;
				if (
					data.sampleRate !== config.sampleRate ||
					data.numberOfChannels !== config.numberOfChannels ||
					!Number.isSafeInteger(data.numberOfFrames) ||
					data.numberOfFrames <= 0 ||
					length + data.numberOfFrames > 65536
				)
					throw new Error(
						"The browser returned invalid AAC calibration samples.",
					);
				const samples = new Float32Array(data.numberOfFrames);
				data.copyTo(samples, { planeIndex: 0, format: "f32-planar" });
				parts.push(samples);
				length += samples.length;
			} catch (error) {
				failure = error;
			} finally {
				data.close();
			}
		},
		error(error) {
			failure = error;
		},
	});
	try {
		decoder.configure(decoderConfig);
		for (const packet of packets) {
			if (failure) throw failure;
			decoder.decode(
				new EncodedAudioChunk({
					type: "key",
					timestamp: packet.timestamp,
					...(packet.duration == null ? {} : { duration: packet.duration }),
					data: packet.data,
				}),
			);
		}
		await deadline(
			decoder.flush(),
			"The browser timed out decoding the AAC synchronization check.",
			10000,
		);
		if (failure) throw failure;
		const samples = new Float32Array(length);
		let offset = 0;
		for (const part of parts) {
			samples.set(part, offset);
			offset += part.length;
		}
		return {
			sampleRate: config.sampleRate,
			length,
			getChannelData: () => samples,
		};
	} finally {
		if (decoder.state !== "closed") decoder.close();
	}
}

async function measureAudioPriming(config, expectedDescription) {
	const key = JSON.stringify([config, [...expectedDescription]]);
	if (audioCalibrations.has(key)) return audioCalibrations.get(key);
	if (!calibrationMuxer)
		throw new Error(
			"Audio synchronization is unavailable. Reload the video editor and try saving again.",
		);
	const AudioContext = globalThis.AudioContext || globalThis.webkitAudioContext;
	const reference = new Float32Array(8192);
	let seed = 0x13579bdf;
	let filtered = 0;
	for (let index = 0; index < reference.length; index++) {
		seed ^= seed << 13;
		seed ^= seed >>> 17;
		seed ^= seed << 5;
		filtered = filtered * 0.8 + ((seed >>> 0) / 0xffffffff - 0.5) * 0.2;
		reference[index] = filtered;
	}
	const planar = new Float32Array(reference.length * config.numberOfChannels);
	for (let channel = 0; channel < config.numberOfChannels; channel++)
		planar.set(reference, channel * reference.length);
	const packets = [];
	let description;
	let failure;
	const encoder = new AudioEncoder({
		output(chunk, metadata) {
			if (packets.length >= 64 || chunk.byteLength > 65536) {
				failure = new Error(
					"The browser returned an oversized audio calibration.",
				);
				return;
			}
			const data = new Uint8Array(chunk.byteLength);
			chunk.copyTo(data);
			packets.push({
				data,
				timestamp: chunk.timestamp,
				duration: chunk.duration,
			});
			const value = metadata?.decoderConfig?.description;
			if (value) {
				const nativeDescription = ArrayBuffer.isView(value)
					? new Uint8Array(
							value.buffer,
							value.byteOffset,
							value.byteLength,
						).slice()
					: new Uint8Array(value.slice(0));
				try {
					description = aacAudioSpecificConfig(nativeDescription);
				} catch (error) {
					failure = error;
				}
			}
		},
		error(error) {
			failure = error;
		},
	});
	let context;
	try {
		encoder.configure(config);
		const data = new AudioData({
			format: "f32-planar",
			sampleRate: config.sampleRate,
			numberOfFrames: reference.length,
			numberOfChannels: config.numberOfChannels,
			timestamp: 0,
			data: planar,
		});
		try {
			encoder.encode(data);
		} finally {
			data.close();
		}
		await deadline(
			encoder.flush(),
			"The browser timed out checking audio synchronization.",
			10000,
		);
		if (failure) throw failure;
		if (!description || !packets.length)
			throw new Error("The browser returned no audio calibration data.");
		if (
			description.length !== expectedDescription.length ||
			description.some((byte, index) => byte !== expectedDescription[index])
		)
			throw new Error(
				"The browser changed its AAC configuration during synchronization. The video was not saved.",
			);
		const encodedSamples = packets.reduce(
			(sum, packet) =>
				sum +
				Math.round(
					((packet.duration ?? (1024 * 1_000_000) / config.sampleRate) *
						config.sampleRate) /
						1_000_000,
				),
			0,
		);
		const bytes = await calibrationMuxer({
			packets,
			description,
			sampleRate: config.sampleRate,
			channels: config.numberOfChannels,
		});
		if (!(bytes instanceof Uint8Array))
			throw new Error(
				typeof bytes === "string"
					? bytes
					: "Audio synchronization could not be measured.",
			);
		let decoded;
		try {
			if (!AudioContext)
				throw new Error("This browser does not provide MP4 audio decoding.");
			context = new AudioContext({ sampleRate: config.sampleRate });
			decoded = await deadline(
				context.decodeAudioData(bytes.slice().buffer),
				"The browser timed out checking MP4 audio synchronization.",
				10000,
			);
			if (
				decoded.sampleRate !== config.sampleRate ||
				Math.abs(decoded.length - encodedSamples) > 1
			)
				throw new Error(
					"The MP4 decoder trims audio calibration samples internally.",
				);
		} catch (containerError) {
			// Packet decoding also avoids automatic MP4 trimming that conceals encoder delay.
			try {
				decoded = await decodeCalibrationPackets(packets, config, description);
			} catch (packetError) {
				throw new Error(
					`The browser could not verify AAC audio synchronization. The video was not saved. MP4 decoder: ${reason(containerError)} AAC decoder: ${reason(packetError)}`,
				);
			}
		}
		// Reject implicit decoder trimming. The explicit MP4 edit must account for every sample.
		if (
			decoded.sampleRate !== config.sampleRate ||
			Math.abs(decoded.length - encodedSamples) > 1
		)
			throw new Error(
				"This browser trims audio calibration samples internally. Try a different browser to save this video with synchronized audio.",
			);
		const samples = decoded.getChannelData(0);
		const measure = (start) => {
			const length = 1024;
			let referenceEnergy = 0;
			for (let index = 0; index < length; index++)
				referenceEnergy += reference[start + index] ** 2;
			let best = -1;
			let offset = -1;
			const limit = Math.min(8192, samples.length - start - length);
			for (let shift = 0; shift <= limit; shift++) {
				let cross = 0;
				let energy = 0;
				for (let index = 0; index < length; index++) {
					const value = samples[start + shift + index];
					cross += value * reference[start + index];
					energy += value * value;
				}
				const score = cross / Math.sqrt(energy * referenceEnergy);
				if (score > best) {
					best = score;
					offset = shift;
				}
			}
			if (best < 0.9)
				throw new Error(
					"The browser's audio encoder could not be synchronized reliably. Try a different browser to save this video.",
				);
			return offset;
		};
		// A generated probe avoids mistaking silence in the user's audio for encoder delay.
		const first = measure(1024);
		const second = measure(5120);
		if (Math.abs(first - second) > 1)
			throw new Error(
				"The browser returned inconsistent audio synchronization. The video was not saved.",
			);
		const priming = Math.round((first + second) / 2);
		audioCalibrations.set(key, priming);
		return priming;
	} finally {
		if (encoder.state !== "closed") encoder.close();
		if (context) {
			try {
				await context.close();
			} catch {
				// Releasing the probe must not replace its synchronization result.
			}
		}
	}
}

async function deadline(promise, message, milliseconds = 30000) {
	let timer;
	try {
		return await Promise.race([
			promise,
			new Promise((_, reject) => {
				timer = setTimeout(() => reject(new Error(message)), milliseconds);
			}),
		]);
	} finally {
		clearTimeout(timer);
	}
}

async function capacity(state, encoder, limit) {
	const started = performance.now();
	while (encoder.encodeQueueSize > limit) {
		if (state.error) throw state.error;
		if (performance.now() - started > 30000)
			throw new Error(
				"The browser encoder stopped responding. Try saving again or use a different browser.",
			);
		await delay(5);
	}
}

function event(target, name, signal, timeout = 15000) {
	return new Promise((resolve, reject) => {
		const timer = setTimeout(
			() =>
				finish(
					new Error(`The browser timed out loading this video (${name}).`),
				),
			timeout,
		);
		const done = () => finish();
		const aborted = () => finish(new Error("This video is no longer open."));
		const failed = () =>
			finish(
				new Error(
					"The browser cannot decode this video. Convert it to a format supported by your browser or operating system.",
				),
			);
		function finish(error) {
			clearTimeout(timer);
			target.removeEventListener(name, done);
			target.removeEventListener("error", failed);
			signal?.removeEventListener("abort", aborted);
			if (error) reject(error);
			else resolve();
		}
		target.addEventListener(name, done, { once: true });
		target.addEventListener("error", failed, { once: true });
		signal?.addEventListener("abort", aborted, { once: true });
		if (signal?.aborted) aborted();
	});
}

export async function openSource(input, name, metadata) {
	// Async browser calls must not retain views into growable WASM memory.
	const bytes = input.slice();
	const info = JSON.parse(metadata);
	const url = URL.createObjectURL(new Blob([bytes]));
	const video = document.createElement("video");
	video.muted = true;
	video.playsInline = true;
	video.preload = "auto";
	try {
		const loaded = event(video, "loadeddata");
		video.src = url;
		video.load();
		await loaded;
		if (
			!video.videoWidth ||
			!video.videoHeight ||
			!Number.isFinite(video.duration) ||
			video.duration <= 0
		) {
			throw new Error("This file has no video stream the browser can edit.");
		}
		let audio = null;
		if (info.audio !== false) {
			const AudioContext =
				globalThis.AudioContext || globalThis.webkitAudioContext;
			if (!AudioContext)
				throw new Error(
					"This browser does not provide audio decoding for video editing.",
				);
			const context = new AudioContext({ sampleRate: 48000 });
			try {
				audio = await deadline(
					context.decodeAudioData(bytes.slice().buffer),
					"The browser timed out decoding this video's audio.",
				);
				if (audio.length * audio.numberOfChannels * 4 > MAX_BYTES)
					throw new Error(
						"The decoded audio exceeds the 200 MiB editing limit.",
					);
			} catch (error) {
				if (info.audio === true)
					throw new Error(
						`The browser cannot decode this video's audio. ${reason(error)}`,
					);
				// Unknown containers must expose audio presence to avoid silently losing a track.
				if (
					!(video.audioTracks && video.audioTracks.length === 0) &&
					video.mozHasAudio !== false
				) {
					throw new Error(
						"The browser could not decode audio or confirm that this video is silent. Use an MP4/MOV file with supported audio.",
					);
				}
			} finally {
				await context.close();
			}
		}
		const canvas = document.createElement("canvas");
		canvas.width = video.videoWidth;
		canvas.height = video.videoHeight;
		if (canvas.width * canvas.height * 4 > 64 * 1024 * 1024)
			throw new Error("The video frame exceeds the 64 MiB editing limit.");
		const ctx = canvas.getContext("2d", { willReadFrequently: true });
		if (!ctx)
			throw new Error("The browser cannot create a video rendering surface.");
		const id = nextId++;
		sources.set(id, {
			closed: false,
			controller: new AbortController(),
			video,
			url,
			canvas,
			ctx,
			audio,
			cache: new Map(),
			pending: new Map(),
			queue: Promise.resolve(),
			error: null,
		});
		return {
			id,
			width: canvas.width,
			height: canvas.height,
			duration: video.duration,
			channels: audio?.numberOfChannels || 0,
			sampleRate: audio?.sampleRate || 48000,
			frameRate: info.frameRate || 30,
		};
	} catch (error) {
		video.removeAttribute("src");
		video.load();
		URL.revokeObjectURL(url);
		throw error;
	}
}

export function frame(id, seconds) {
	const source = sources.get(id);
	if (!source) throw new Error("This video is no longer open.");
	if (source.error) throw source.error;
	const key = Math.round(seconds * 1000000);
	if (source.cache.has(key)) return source.cache.get(key);
	if (!source.pending.has(key) && source.pending.size >= 8) return null;
	if (!source.pending.has(key)) {
		const request = source.queue
			.then(async () => {
				if (source.closed) return;
				const target = Math.max(
					0,
					Math.min(seconds, source.video.duration - 0.000001),
				);
				if (Math.abs(source.video.currentTime - target) > 0.000001) {
					const seeked = event(
						source.video,
						"seeked",
						source.controller.signal,
					);
					source.video.currentTime = target;
					await seeked;
				}
				if (source.closed) return;
				source.ctx.drawImage(source.video, 0, 0);
				const pixels = source.ctx.getImageData(
					0,
					0,
					source.canvas.width,
					source.canvas.height,
				).data;
				source.cache.set(key, new Uint8Array(pixels.buffer));
				const capacity = Math.max(
					1,
					Math.min(12, Math.floor((64 * 1024 * 1024) / pixels.byteLength)),
				);
				while (source.cache.size > capacity)
					source.cache.delete(source.cache.keys().next().value);
			})
			.catch((error) => {
				if (!source.closed) source.error = error;
			})
			.finally(() => {
				source.pending.delete(key);
				if (!source.closed) sourceReady();
			});
		source.pending.set(key, request);
		source.queue = request;
	}
	return null;
}

export function audio(id, start, count, rate) {
	const source = sources.get(id);
	if (!source?.audio) return [];
	const input = source.audio;
	const channels = [];
	for (let channel = 0; channel < input.numberOfChannels; channel++) {
		const samples = input.getChannelData(channel);
		const output = new Float32Array(count);
		for (let index = 0; index < count; index++) {
			const position = ((start + index) * input.sampleRate) / rate;
			const lower = Math.floor(position);
			if (lower >= 0 && lower < samples.length) {
				const fraction = position - lower;
				output[index] =
					samples[lower] * (1 - fraction) +
					(samples[lower + 1] ?? samples[lower]) * fraction;
			}
		}
		channels.push(output);
	}
	return channels;
}

export function closeSource(id) {
	const source = sources.get(id);
	if (!source) return;
	sources.delete(id);
	source.closed = true;
	source.controller.abort();
	source.audio = null;
	source.cache.clear();
	source.canvas.width = 0;
	source.canvas.height = 0;
	source.video.removeAttribute("src");
	source.video.load();
	URL.revokeObjectURL(source.url);
}

export async function createEncoder(
	width,
	height,
	frameRate,
	sampleRate,
	channels,
) {
	if (!globalThis.VideoEncoder || !globalThis.VideoFrame)
		throw new Error(
			"MP4 saving requires a browser or operating system with H.264 encoding. Try a current Chrome, Edge, or Safari version.",
		);
	let config;
	for (const codec of ["avc1.640033", "avc1.4d0033", "avc1.420033"]) {
		const candidate = {
			codec,
			width,
			height,
			framerate: frameRate,
			bitrate: Math.min(
				20000000,
				Math.max(1000000, width * height * frameRate * 0.12),
			),
			latencyMode: "realtime",
			avc: { format: "avc" },
		};
		try {
			if ((await VideoEncoder.isConfigSupported(candidate)).supported) {
				config = candidate;
				break;
			}
		} catch {}
	}
	if (!config)
		throw new Error(
			"Your browser cannot encode H.264 at this video's size and frame rate. Use a browser or operating system that supports this configuration.",
		);
	const audioConfig = {
		codec: "mp4a.40.2",
		sampleRate,
		numberOfChannels: channels,
		bitrate: 192000,
	};
	if (
		channels &&
		(!globalThis.AudioEncoder ||
			!globalThis.AudioData ||
			!(await AudioEncoder.isConfigSupported(audioConfig)).supported)
	) {
		throw new Error(
			"Your browser cannot encode AAC audio for MP4 saving. Try a current Chrome, Edge, or Safari version.",
		);
	}
	const state = {
		video: [],
		audio: [],
		expectedVideo: [],
		audioSamples: 0,
		description: null,
		audioDescription: null,
		size: 0,
		error: null,
		frameRate,
		sampleRate,
		channels,
		audioConfig,
	};
	const output = (list, description) => (chunk, metadata) => {
		state.size += chunk.byteLength;
		if (state.size > MAX_BYTES) {
			state.error = new Error("The exported video exceeds 200 MiB.");
			return;
		}
		const data = new Uint8Array(chunk.byteLength);
		chunk.copyTo(data);
		state[list].push({
			data,
			timestamp: chunk.timestamp,
			duration: chunk.duration,
			key: chunk.type === "key",
		});
		const config = metadata?.decoderConfig?.description;
		if (config) {
			const bytes = ArrayBuffer.isView(config)
				? new Uint8Array(
						config.buffer,
						config.byteOffset,
						config.byteLength,
					).slice()
				: new Uint8Array(config.slice(0));
			try {
				state[description] =
					description === "audioDescription"
						? aacAudioSpecificConfig(bytes)
						: bytes;
			} catch (error) {
				state.error = error;
			}
		}
	};
	state.venc = new VideoEncoder({
		output: output("video", "description"),
		error: (error) => {
			state.error = error;
		},
	});
	try {
		state.venc.configure(config);
		if (channels) {
			state.aenc = new AudioEncoder({
				output: output("audio", "audioDescription"),
				error: (error) => {
					state.error = error;
				},
			});
			state.aenc.configure(audioConfig);
		}
	} catch (error) {
		if (state.venc.state !== "closed") state.venc.close();
		if (state.aenc && state.aenc.state !== "closed") state.aenc.close();
		throw error;
	}
	const id = nextId++;
	encoders.set(id, state);
	return id;
}

export async function encodeFrame(
	id,
	input,
	width,
	height,
	timestamp,
	duration,
) {
	const pixels = input.slice();
	const state = encoders.get(id);
	if (!state || state.error)
		throw state?.error || new Error("The video export is no longer active.");
	await capacity(state, state.venc, 4);
	const frame = new VideoFrame(pixels, {
		format: "RGBA",
		codedWidth: width,
		codedHeight: height,
		timestamp,
		duration,
	});
	try {
		state.venc.encode(frame, {
			keyFrame:
				timestamp === 0 ||
				(Math.floor(timestamp / 1000000) % 2 === 0 &&
					timestamp % 1000000 < duration),
		});
		state.expectedVideo.push({ timestamp, duration });
	} finally {
		frame.close();
	}
}

export async function encodeAudio(id, input, count, timestamp) {
	const planar = input.slice();
	const state = encoders.get(id);
	if (!state?.aenc || state.error)
		throw state?.error || new Error("The audio encoder is unavailable.");
	await capacity(state, state.aenc, 8);
	const data = new AudioData({
		format: "f32-planar",
		sampleRate: state.sampleRate,
		numberOfFrames: count,
		numberOfChannels: state.channels,
		timestamp,
		data: planar,
	});
	try {
		state.aenc.encode(data);
		state.audioSamples += count;
	} finally {
		data.close();
	}
}

export async function finishEncoder(id) {
	const state = encoders.get(id);
	if (!state) throw new Error("The video export is no longer active.");
	try {
		await deadline(
			state.venc.flush(),
			"The browser timed out finishing the video encode.",
		);
		if (state.aenc)
			await deadline(
				state.aenc.flush(),
				"The browser timed out finishing the audio encode.",
			);
		state.venc.close();
		if (state.aenc) state.aenc.close();
		if (state.error) throw state.error;
		if (!state.description || !state.video.length)
			throw new Error("The browser returned no H.264 video configuration.");
		if (state.channels && (!state.audioDescription || !state.audio.length))
			throw new Error("The browser returned no AAC audio configuration.");
		if (state.video.length !== state.expectedVideo.length)
			throw new Error(
				"The browser dropped video frames while encoding. The video was not saved. Try a different browser or a smaller sequence size.",
			);
		for (let index = 0; index < state.video.length; index++) {
			const packet = state.video[index];
			const expected = state.expectedVideo[index];
			if (
				packet.timestamp !== expected.timestamp ||
				(packet.duration != null &&
					Math.abs(packet.duration - expected.duration) > 1)
			)
				throw new Error(
					"The browser changed video frame timing while encoding. The video was not saved.",
				);
			packet.duration = expected.duration;
		}
		if (state.channels) {
			state.audioPriming = await measureAudioPriming(
				state.audioConfig,
				state.audioDescription,
			);
			const tolerance = 2_000_000 / state.sampleRate;
			let end = state.audio[0].timestamp;
			if (end > tolerance)
				throw new Error(
					"The browser omitted the beginning of the audio. The video was not saved.",
				);
			for (const packet of state.audio) {
				packet.duration ??= (1024 * 1_000_000) / state.sampleRate;
				if (
					!(packet.duration > 0) ||
					Math.abs(packet.timestamp - end) > tolerance
				)
					throw new Error(
						"The browser returned incomplete audio timing. The video was not saved.",
					);
				end = packet.timestamp + packet.duration;
			}
			if (
				end - state.audio[0].timestamp + tolerance <
				((state.audioSamples + state.audioPriming) * 1_000_000) /
					state.sampleRate
			)
				throw new Error(
					"The browser omitted the end of the audio. The video was not saved.",
				);
		}
		return {
			video: state.video,
			audio: state.audio,
			description: state.description,
			audioDescription: state.audioDescription,
			audioPriming: state.audioPriming || 0,
		};
	} finally {
		cancelEncoder(id);
	}
}

export function cancelEncoder(id) {
	const state = encoders.get(id);
	if (!state) return;
	encoders.delete(id);
	if (state.venc.state !== "closed") state.venc.close();
	if (state.aenc && state.aenc.state !== "closed") state.aenc.close();
}

addEventListener("pagehide", () => {
	for (const id of sources.keys()) closeSource(id);
	for (const id of encoders.keys()) cancelEncoder(id);
});
