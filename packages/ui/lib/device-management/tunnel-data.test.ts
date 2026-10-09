import { afterEach, describe, expect, test } from "bun:test";
import type { DeviceServiceStream, TunnelServiceOptions } from "./tunnel";
import {
	DeviceTunnelDataClient,
	TUNNEL_RESPONSE_LIMIT,
	type TunnelArtifactUpload,
	isTunnelReadCommand,
} from "./tunnel-data";
import { TUNNEL_WINDOW, type TunnelDataOpen } from "./tunnel-protocol";
import type { TunnelConnectOptions } from "./tunnel-transport";

const transferId = "11111111-2222-4333-8444-555555555555";
const clients: DeviceTunnelDataClient[] = [];
afterEach(() => {
	for (const client of clients.splice(0)) client.close();
});
const options = { receipt: { device_id: "device-1" } } as TunnelConnectOptions;
const encoder = new TextEncoder();
const wait = async () => {
	for (let index = 0; index < 25; index++) await Promise.resolve();
};

class Stream {
	private close!: () => void;
	closed = new Promise<void>((resolve) => {
		this.close = resolve;
	});
	ended = false;
	resetCalled = false;
	finished = false;
	writes: Uint8Array[] = [];
	chunks: Uint8Array[] = [];
	async read() {
		const chunk = this.chunks.shift();
		if (!chunk) this.finished = true;
		return chunk ?? null;
	}
	async write(bytes: Uint8Array) {
		this.writes.push(bytes.slice());
	}
	async end() {
		this.ended = true;
	}
	reset() {
		this.resetCalled = true;
		this.close();
	}
	asStream(): DeviceServiceStream {
		return this as unknown as DeviceServiceStream;
	}
}
class Tunnel {
	kind = "websocket" as const;
	connected = true;
	inputs: TunnelDataOpen[] = [];
	streams: Stream[] = [];
	serviceInputs: {
		placement: string;
		service: string;
		options?: TunnelServiceOptions;
	}[] = [];
	serviceStreams: Stream[] = [];
	makeService: () => Stream | Promise<Stream> = () => new Stream();
	async open(
		placement: string,
		service: string,
		options?: TunnelServiceOptions,
	) {
		this.serviceInputs.push({ placement, service, options });
		const stream = await this.makeService();
		this.serviceStreams.push(stream);
		return stream.asStream();
	}
	gatewayStreams: Stream[] = [];
	async openModelGateway(_options?: { signal?: AbortSignal }) {
		const stream = new Stream();
		this.gatewayStreams.push(stream);
		return stream.asStream();
	}
	make: (input: TunnelDataOpen) => Stream | Promise<Stream> = (input) => {
		const stream = new Stream();
		const reply =
			input.kind === "request"
				? {
						operation_id: input.request.operation_id,
						state: "completed",
						result: { placements: [] },
					}
				: input.kind === "artifact"
					? receipt(0, input.file_index)
					: assetStatus(input.job_id, { state: "present" });
		stream.chunks.push(encoder.encode(JSON.stringify(reply)));
		return stream;
	};
	async openData(input: TunnelDataOpen) {
		this.inputs.push(input);
		const stream = await this.make(input);
		this.streams.push(stream);
		return stream.asStream();
	}
	close() {
		this.connected = false;
		for (const stream of this.serviceStreams) stream.reset();
	}
}
function client(tunnel = new Tunnel()) {
	let connects = 0;
	const data = new DeviceTunnelDataClient(options, async () => {
		connects++;
		return tunnel;
	});
	clients.push(data);
	return { data, tunnel, connects: () => connects };
}
const jobId = "12345678-1234-4234-8234-123456789abc";
function assetStatus(job: string, state: Record<string, unknown>) {
	return {
		digest: { algorithm: "blake3", hex: "b".repeat(64) },
		job_id: job,
		...state,
	};
}
function receipt(size: number, fileIndex: number | null = 0) {
	return {
		transfer_id: transferId,
		descriptor: {
			project_id: "project",
			manifest_sha256: "a".repeat(64),
			manifest_size: 1,
			file_count: 1,
			total_bytes: size,
		},
		state: "receiving",
		expires_at: 2000000000,
		manifest_ready: true,
		file_index: fileIndex,
		offset: size,
		complete: true,
		project_path: null,
	};
}

describe("production tunnel data adapter", () => {
	test("service opens share the cached tunnel and keep independent cancellation", async () => {
		const { data, tunnel, connects } = client();
		const abort = new AbortController();
		const [first, second] = await Promise.all([
			data.openService("placement-a", "hosting", {
				mode: "http",
				signal: abort.signal,
			}),
			data.openService("placement-b", "database", { mode: "tcp" }),
		]);
		expect(connects()).toBe(1);
		expect(
			tunnel.serviceInputs.map(({ placement, service, options }) => [
				placement,
				service,
				options?.mode,
			]),
		).toEqual([
			["placement-a", "hosting", "http"],
			["placement-b", "database", "tcp"],
		]);
		abort.abort();
		await first.closed;
		expect(tunnel.serviceStreams[0].resetCalled).toBe(true);
		expect(tunnel.serviceStreams[1].resetCalled).toBe(false);
		data.close();
		await second.closed;
		expect(tunnel.serviceStreams[1].resetCalled).toBe(true);
	});

	test("model gateway opens share the cached tunnel and reset when cancelled", async () => {
		const { data, tunnel, connects } = client();
		const abort = new AbortController();
		const [gateway, service] = await Promise.all([
			data.openModelGateway({ signal: abort.signal }),
			data.openService("placement", "hosting"),
		]);
		expect(connects()).toBe(1);
		expect(tunnel.gatewayStreams).toHaveLength(1);
		abort.abort();
		await gateway.closed;
		expect(tunnel.gatewayStreams[0].resetCalled).toBe(true);
		expect(tunnel.serviceStreams[0].resetCalled).toBe(false);
		data.close();
		await service.closed;
	});

	test("cancelled service opens reset a stream that arrives after cancellation", async () => {
		const { data, tunnel } = client();
		const abort = new AbortController();
		let resolve!: (stream: Stream) => void;
		tunnel.makeService = () =>
			new Promise((done) => {
				resolve = done;
			});
		const opening = data.openService("placement", "hosting", {
			signal: abort.signal,
		});
		await wait();
		abort.abort();
		await expect(opening).rejects.toThrow("cancelled");
		const late = new Stream();
		resolve(late);
		await wait();
		expect(late.resetCalled).toBe(true);
	});

	test("closing a service owner cancels a late connection without opening the service", async () => {
		const tunnel = new Tunnel();
		let resolve!: (tunnel: Tunnel) => void;
		let connectingSignal: AbortSignal | undefined;
		const data = new DeviceTunnelDataClient(options, (input) => {
			connectingSignal = input.signal;
			return new Promise((done) => {
				resolve = done;
			});
		});
		clients.push(data);
		const opening = data.openService("placement", "hosting");
		data.close();
		await expect(opening).rejects.toThrow("cancelled");
		expect(connectingSignal?.aborted).toBe(true);
		resolve(tunnel);
		await wait();
		expect(tunnel.connected).toBe(false);
		expect(tunnel.serviceInputs).toHaveLength(0);
	});

	test("a stalled shared connection expires and a late result cannot replace its successor", async () => {
		const deadlines: { delay: number; fire: () => void }[] = [];
		const original = globalThis.setTimeout;
		globalThis.setTimeout = ((callback: () => void, delay: number) => {
			deadlines.push({ delay, fire: callback });
			return original(callback, delay);
		}) as typeof setTimeout;
		try {
			let resolve!: (tunnel: Tunnel) => void;
			let connectingSignal: AbortSignal | undefined;
			let connects = 0;
			const current = new Tunnel();
			const data = new DeviceTunnelDataClient(options, (input) => {
				connects++;
				if (connects > 1) return Promise.resolve(current);
				connectingSignal = input.signal;
				return new Promise((done) => {
					resolve = done;
				});
			});
			clients.push(data);
			const opening = data
				.openService("placement", "hosting")
				.catch((error) => error);
			const reading = data
				.request({ type: "inspect_page" }, "first")
				.catch((error) => error);
			expect(connects).toBe(1);
			expect(deadlines[0].delay).toBe(60_000);
			deadlines[0].fire();
			expect((await opening).message).toBe(
				"The device data connection timed out.",
			);
			expect(await reading).toMatchObject({ sent: false });
			expect(connectingSignal?.aborted).toBe(true);

			expect(
				(await data.request({ type: "inspect_page" }, "second")).state,
			).toBe("completed");
			expect(connects).toBe(2);
			const late = new Tunnel();
			resolve(late);
			await wait();
			expect(late.connected).toBe(false);
			expect(late.serviceInputs).toHaveLength(0);
			expect(late.inputs).toHaveLength(0);
			await data.request({ type: "inspect_page" }, "third");
			expect(current.inputs).toHaveLength(2);
			expect(current.connected).toBe(true);
			expect(connects).toBe(2);
		} finally {
			globalThis.setTimeout = original;
		}
	});

	test("cancelling a reader leaves shared connection setup available to other callers", async () => {
		const tunnel = new Tunnel();
		let resolve!: (tunnel: Tunnel) => void;
		let connectingSignal: AbortSignal | undefined;
		let connects = 0;
		const data = new DeviceTunnelDataClient(options, (input) => {
			connects++;
			connectingSignal = input.signal;
			return new Promise((done) => {
				resolve = done;
			});
		});
		clients.push(data);
		const abort = new AbortController();
		const reading = data
			.request({ type: "inspect_page" }, "first", abort.signal)
			.catch((error) => error);
		const opening = data.openService("placement", "hosting");
		abort.abort();
		expect((await reading).message).toContain("cancelled");
		expect(connectingSignal?.aborted).toBe(false);
		resolve(tunnel);
		await opening;
		await data.request({ type: "inspect_page" }, "second");
		expect(connects).toBe(1);
		expect(tunnel.serviceInputs).toHaveLength(1);
		expect(tunnel.inputs).toHaveLength(1);
	});

	test("a tunnel owner cannot move to another grant, device identity or auth epoch", () => {
		const controller = {} as TunnelConnectOptions["controller"];
		const identity = {
			...options,
			controller,
			grantId: "owner",
			receipt: {
				device_id: "device",
				auth_epoch: 1,
				identity: { management_key: [1, 2, 3] },
			},
		} as TunnelConnectOptions;
		const data = new DeviceTunnelDataClient(identity);
		clients.push(data);
		expect(data.matches({ ...identity, grantId: undefined })).toBe(true);
		expect(data.matches({ ...identity, grantId: "different" })).toBe(false);
		expect(
			data.matches({
				...identity,
				controller: {} as TunnelConnectOptions["controller"],
			}),
		).toBe(false);
		identity.receipt.identity.management_key[0] = 9;
		expect(data.matches(identity)).toBe(false);
		expect(
			data.matches({
				...identity,
				receipt: { ...identity.receipt, auth_epoch: 2 },
			}),
		).toBe(false);
		expect(
			data.matches({
				...identity,
				receipt: {
					...identity.receipt,
					identity: { ...identity.receipt.identity, management_key: [1, 2, 4] },
				},
			}),
		).toBe(false);
	});
	test("lazily shares one tunnel across concurrent typed reads and finishes each request stream", async () => {
		const { data, tunnel, connects } = client();
		expect(connects()).toBe(0);
		const replies = await Promise.all([
			data.request({ type: "inspect_page", limit: 2 }, "op-1"),
			data.request({ type: "logs", limit: 100 }, "op-2"),
		]);
		expect(connects()).toBe(1);
		expect(replies.map((value) => value.operation_id)).toEqual([
			"op-1",
			"op-2",
		]);
		expect(tunnel.inputs[0]).toMatchObject({
			kind: "request",
			request: {
				device_id: "device-1",
				operation_id: "op-1",
				command: { type: "inspect_page", limit: 2 },
			},
		});
		expect(
			tunnel.streams.every(
				(stream) => stream.ended && stream.writes.length === 0,
			),
		).toBe(true);
		await expect(data.request({ type: "restart" }, "op-3")).rejects.toThrow(
			"management connection",
		);
		expect(tunnel.inputs).toHaveLength(2);
	});
	test("resumes an artifact above 4 GiB and a model above 64 GiB with only the missing byte", async () => {
		for (const kind of ["artifact", "model"] as const) {
			const { data, tunnel } = client();
			const size = (kind === "artifact" ? 5 : 65) * 1024 ** 3;
			const ranges: [number, number][] = [];
			const file = {
				size,
				slice(start: number, end: number) {
					ranges.push([start, end]);
					return new Blob([new Uint8Array(end - start)]);
				},
			};
			if (kind === "artifact") {
				tunnel.make = () => {
					const stream = new Stream();
					stream.chunks.push(encoder.encode(JSON.stringify(receipt(size))));
					return stream;
				};
				await data.uploadArtifact({
					projectId: "project",
					transferId,
					fileIndex: 0,
					offset: size - 1,
					file,
				});
			} else {
				await data.pushModelAsset({ jobId, offset: size - 1, file });
			}
			expect(ranges).toEqual([[size - 1, size]]);
			expect(tunnel.streams[0].writes).toHaveLength(1);
			expect(tunnel.streams[0].writes[0]).toHaveLength(1);
		}
	});
	test("streams only the unconfirmed artifact suffix in bounded slices", async () => {
		const { data, tunnel } = client();
		const size = TUNNEL_WINDOW * 3 + 7;
		const ranges: [number, number][] = [];
		tunnel.make = () => {
			const stream = new Stream();
			stream.chunks.push(encoder.encode(JSON.stringify(receipt(size))));
			return stream;
		};
		const input: TunnelArtifactUpload = {
			projectId: "project",
			transferId,
			fileIndex: 0,
			offset: 19,
			file: {
				size,
				slice(start, end) {
					ranges.push([start, end]);
					return {
						async arrayBuffer() {
							return new Uint8Array(end - start).fill(7).buffer;
						},
					};
				},
			},
		};
		const result = await data.uploadArtifact(input);
		expect(result.offset).toBe(size);
		expect(ranges[0][0]).toBe(19);
		expect(ranges.at(-1)?.[1]).toBe(size);
		expect(ranges.every(([start, end]) => end - start <= TUNNEL_WINDOW)).toBe(
			true,
		);
		expect(tunnel.inputs).toEqual([
			{
				kind: "artifact",
				project_id: "project",
				transfer_id: transferId,
				file_index: 0,
				offset: 19,
			},
		]);
		expect(
			tunnel.streams[0].writes.reduce((sum, bytes) => sum + bytes.length, 0),
		).toBe(size - 19);
		expect(
			tunnel.streams[0].writes.every((bytes) =>
				bytes.every((byte) => byte === 7),
			),
		).toBe(true);
		expect(tunnel.streams[0].ended).toBe(true);
	});
	test("bounds JSON accumulation and clears each one-byte chunk after consumption", async () => {
		const { data, tunnel } = client();
		let previous: Uint8Array | undefined;
		const payload = encoder.encode(
			JSON.stringify({
				operation_id: "op",
				state: "completed",
				result: { text: "a".repeat(10_000) },
			}),
		);
		tunnel.make = () => {
			const stream = new Stream();
			let at = 0;
			stream.read = async () => {
				if (previous) expect(previous[0]).toBe(0);
				if (at === payload.length) {
					stream.finished = true;
					return null;
				}
				previous = payload.slice(at, ++at);
				return previous;
			};
			return stream;
		};
		expect((await data.request({ type: "logs" }, "op")).state).toBe(
			"completed",
		);
		tunnel.make = () => {
			const stream = new Stream();
			stream.chunks.push(new Uint8Array(TUNNEL_RESPONSE_LIMIT + 1));
			return stream;
		};
		await expect(data.request({ type: "logs" }, "op")).rejects.toThrow(
			"byte limit",
		);
		expect(tunnel.streams.at(-1)?.resetCalled).toBe(true);
	});
	test("an aborted pending open resets its stream when that open eventually completes", async () => {
		const { data, tunnel } = client();
		const stream = new Stream();
		let finish: (stream: Stream) => void = () => {};
		tunnel.make = () =>
			new Promise((resolve) => {
				finish = resolve;
			});
		const signal = new AbortController();
		const request = data
			.request({ type: "logs" }, "op", signal.signal)
			.catch((error) => error);
		await wait();
		signal.abort();
		expect(await request).toBeInstanceOf(Error);
		finish(stream);
		await wait();
		expect(stream.resetCalled).toBe(true);
	});
	test("a lost request is not replayed; a future request opens a fresh tunnel", async () => {
		const first = new Tunnel();
		const second = new Tunnel();
		let connects = 0;
		first.make = () => {
			first.connected = false;
			throw new Error("Tunnel disconnected.");
		};
		const data = new DeviceTunnelDataClient(options, async () =>
			++connects === 1 ? first : second,
		);
		clients.push(data);
		await expect(data.request({ type: "logs" }, "op-1")).rejects.toThrow(
			"disconnected",
		);
		expect(connects).toBe(1);
		expect(first.inputs).toHaveLength(1);
		await data.request({ type: "logs" }, "op-2");
		expect(connects).toBe(2);
	});
	test("models statistics take the data stream; other models requests don't", () => {
		expect(isTunnelReadCommand({ type: "logs" })).toBe(true);
		expect(
			isTunnelReadCommand({ type: "models", request: { kind: "stats" } }),
		).toBe(true);
		for (const kind of ["jobs", "overview", "ensure", "install"])
			expect(isTunnelReadCommand({ type: "models", request: { kind } })).toBe(
				false,
			);
		expect(isTunnelReadCommand({ type: "models" })).toBe(false);
		expect(isTunnelReadCommand({ type: "restart" })).toBe(false);
	});
	test("a model asset push streams only the missing suffix and answers the job's state", async () => {
		const { data, tunnel } = client();
		const size = TUNNEL_WINDOW * 2 + 5;
		const ranges: [number, number][] = [];
		const progress: number[] = [];
		const status = await data.pushModelAsset({
			jobId,
			offset: 11,
			file: {
				size,
				slice(start, end) {
					ranges.push([start, end]);
					return {
						async arrayBuffer() {
							return new Uint8Array(end - start).fill(3).buffer;
						},
					};
				},
			},
			onProgress: (bytes) => progress.push(bytes),
		});
		expect(status).toMatchObject({ job_id: jobId, state: "present" });
		expect(tunnel.inputs).toEqual([
			{ kind: "model_asset", job_id: jobId, offset: 11 },
		]);
		expect(ranges[0]).toEqual([11, 11 + TUNNEL_WINDOW]);
		expect(ranges.at(-1)?.[1]).toBe(size);
		expect(progress.at(-1)).toBe(size);
		expect(
			tunnel.streams[0].writes.reduce((sum, bytes) => sum + bytes.length, 0),
		).toBe(size - 11);
		expect(tunnel.streams[0].ended).toBe(true);
	});
	test("a push without bytes only asks where the device's copy ends", async () => {
		const { data, tunnel } = client();
		tunnel.make = (input) => {
			const stream = new Stream();
			const job = input.kind === "model_asset" ? input.job_id : "";
			stream.chunks.push(
				encoder.encode(
					JSON.stringify(
						assetStatus(job, { state: "awaiting_push", bytes: 4096 }),
					),
				),
			);
			return stream;
		};
		const status = await data.pushModelAsset({ jobId, offset: 0 });
		expect(status).toMatchObject({ state: "awaiting_push", bytes: 4096 });
		expect(tunnel.streams[0].writes).toHaveLength(0);
		expect(tunnel.streams[0].ended).toBe(true);
	});
	test("a push refuses an invalid range and an answer about another job", async () => {
		const { data, tunnel } = client();
		const file = new Blob([new Uint8Array(8)]);
		await expect(
			data.pushModelAsset({ jobId, offset: 9, file }),
		).rejects.toThrow("offset 9 of 8 bytes");
		await expect(
			data.pushModelAsset({ jobId: "job", offset: 0, file }),
		).rejects.toThrow("Invalid model asset push range");
		expect(tunnel.inputs).toHaveLength(0);
		tunnel.make = () => {
			const stream = new Stream();
			stream.chunks.push(
				encoder.encode(
					JSON.stringify(
						assetStatus("87654321-1234-4234-8234-123456789abc", {
							state: "present",
						}),
					),
				),
			);
			return stream;
		};
		await expect(
			data.pushModelAsset({ jobId, offset: 0, file }),
		).rejects.toThrow("does not match");
	});
	test("a large model file gets as long to verify as the device needs to read it back", async () => {
		const { data, tunnel } = client();
		const timers: number[] = [];
		const original = globalThis.setTimeout;
		globalThis.setTimeout = ((callback: () => void, delay: number) => {
			timers.push(delay);
			return original(callback, delay);
		}) as typeof setTimeout;
		try {
			tunnel.make = () => {
				const stream = new Stream();
				stream.chunks.push(
					encoder.encode(
						JSON.stringify(assetStatus(jobId, { state: "present" })),
					),
				);
				return stream;
			};
			const size = 40_000_000_000;
			await data.pushModelAsset({
				jobId,
				offset: size,
				file: {
					size,
					slice: () => {
						throw new Error("nothing is left to send");
					},
				},
			});
			expect(timers).toContain(4_000_000);
			expect(timers).not.toContain(15 * 60_000);
		} finally {
			globalThis.setTimeout = original;
		}
	});
	test("artifact verification gets a bounded longer deadline than a stalled upload", async () => {
		const { data, tunnel } = client();
		const timers: number[] = [];
		const original = globalThis.setTimeout;
		globalThis.setTimeout = ((callback: () => void, delay: number) => {
			timers.push(delay);
			return original(callback, delay);
		}) as typeof setTimeout;
		try {
			tunnel.make = () => {
				const stream = new Stream();
				stream.chunks.push(encoder.encode(JSON.stringify(receipt(0))));
				return stream;
			};
			await data.uploadArtifact({
				projectId: "project",
				transferId,
				fileIndex: 0,
				offset: 0,
				file: new Blob(),
			});
			expect(timers).toContain(60_000);
			expect(timers).toContain(15 * 60_000);
		} finally {
			globalThis.setTimeout = original;
		}
	});
});
