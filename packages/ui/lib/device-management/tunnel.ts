import { base64url } from "./crypto";
import {
	TUNNEL_MAX_DATA,
	TUNNEL_MAX_STREAMS,
	TUNNEL_WINDOW,
	type TunnelDataOpen,
	type TunnelFrame,
	TunnelKind,
	decodeTunnelFrame,
	encodeTunnelFrame,
	readTunnelJson,
	tunnelJson,
} from "./tunnel-protocol";
import {
	type TunnelConnectOptions,
	type TunnelTransport,
	connectTunnelTransport,
} from "./tunnel-transport";
import type { NoiseHandshake } from "./types";

export type { TunnelConnectOptions } from "./tunnel-transport";
export type { TunnelArtifactUpload } from "./tunnel-data";

type Deferred<T> = {
	promise: Promise<T>;
	resolve(value: T): void;
	reject(error: Error): void;
};
function deferred<T>(): Deferred<T> {
	let resolve!: (value: T) => void;
	let reject!: (error: Error) => void;
	const promise = new Promise<T>((yes, no) => {
		resolve = yes;
		reject = no;
	});
	return { promise, resolve, reject };
}

export class DeviceTunnelError extends Error {
	constructor(
		readonly code: string,
		message: string,
	) {
		super(message);
		this.name = "DeviceTunnelError";
	}
}
const closedError = () =>
	new DeviceTunnelError(
		"connection_closed",
		"The device tunnel closed. Open a new connection before continuing.",
	);
const protocolError = () =>
	new DeviceTunnelError(
		"invalid_frame",
		"The device sent an invalid tunnel frame.",
	);
const EMPTY = new Uint8Array();

export interface TunnelServiceOptions {
	mode?: "tcp" | "http";
	signal?: AbortSignal;
}

type Send = (
	kind: TunnelKind,
	stream: number,
	body?: Uint8Array,
	priority?: boolean,
) => Promise<void>;

/** A half-closeable byte stream. Only consumed bytes replenish the peer's window. */
export class DeviceServiceStream {
	private readonly opened = deferred<void>();
	private readonly completed = deferred<void>();
	private readonly chunks: { bytes: Uint8Array; length: number }[] = [];
	private queuedBytes = 0;
	private receiveCredit = TUNNEL_WINDOW;
	private sendCredit = TUNNEL_WINDOW;
	private reader?: Deferred<Uint8Array | null>;
	private writer?: Deferred<void>;
	private writeCancelled?: Deferred<void>;
	private failure?: Error;
	private accepted = false;
	private writing = false;
	private localFin = false;
	private remoteFin = false;
	private readonly openTimer: ReturnType<typeof setTimeout>;
	constructor(
		readonly id: number,
		private readonly send: Send,
		private readonly release: () => void,
	) {
		this.openTimer = setTimeout(
			() =>
				this.reset("open_timeout", "The device service did not open in time."),
			15_000,
		);
	}
	/** Resolves only after the device authorizes and opens the configured service. */
	get ready(): Promise<void> {
		return this.opened.promise;
	}
	/** Settles after both halves finish or the stream is reset. Reads still expose failures. */
	get closed(): Promise<void> {
		return this.completed.promise;
	}
	get finished(): boolean {
		return this.localFin && this.remoteFin && this.queuedBytes === 0;
	}
	async read(): Promise<Uint8Array | null> {
		await this.ready;
		if (this.failure) throw this.failure;
		if (this.reader)
			throw new Error("A device stream supports one reader at a time.");
		const chunk = this.chunks.shift();
		if (chunk) {
			this.queuedBytes -= chunk.length;
			this.consumed(chunk.length);
			return chunk.bytes.subarray(0, chunk.length);
		}
		if (this.remoteFin) {
			this.maybeRelease();
			return null;
		}
		this.reader = deferred<Uint8Array | null>();
		return this.reader.promise;
	}
	/** Writes at most one window per call; awaiting each write bounds retained input. */
	async write(bytes: Uint8Array, onProgress?: () => void): Promise<void> {
		if (!(bytes instanceof Uint8Array) || bytes.length > TUNNEL_WINDOW)
			throw new Error(
				`Write at most ${TUNNEL_WINDOW} bytes to a device stream at once.`,
			);
		if (this.writing)
			throw new Error("Wait for the previous device stream write.");
		this.writing = true;
		const input = bytes.slice();
		try {
			await this.ready;
			if (this.failure) throw this.failure;
			if (this.localFin)
				throw new Error("The device stream's write side is closed.");
			this.writeCancelled = deferred<void>();
			void this.writeCancelled.promise.catch(() => {});
			let offset = 0;
			while (offset < input.length) {
				if (this.failure) throw this.failure;
				if (this.localFin)
					throw new Error("The device stream's write side is closed.");
				if (this.sendCredit === 0) {
					this.writer = deferred<void>();
					await this.writer.promise;
					this.writer = undefined;
					continue;
				}
				const length = Math.min(
					TUNNEL_MAX_DATA,
					this.sendCredit,
					input.length - offset,
				);
				this.sendCredit -= length;
				await Promise.race([
					this.send(
						TunnelKind.Data,
						this.id,
						input.slice(offset, offset + length),
					),
					this.writeCancelled.promise,
				]);
				offset += length;
				onProgress?.();
			}
		} finally {
			input.fill(0);
			this.writing = false;
			this.writeCancelled = undefined;
		}
	}
	async end(): Promise<void> {
		await this.ready;
		if (this.failure) throw this.failure;
		if (this.writing)
			throw new Error(
				"Wait for the previous device stream write before ending it.",
			);
		if (this.localFin) return;
		this.localFin = true;
		await this.send(TunnelKind.Fin, this.id);
		this.maybeRelease();
	}
	reset(
		code = "cancelled",
		message = "The service stream was cancelled.",
	): void {
		if (this.failure) return;
		const error = new DeviceTunnelError(code, message);
		this.fail(error);
		void this.send(
			TunnelKind.Reset,
			this.id,
			tunnelJson({
				code: "cancelled",
				message: "The service stream was cancelled.",
			}),
		).catch(() => {});
	}
	/** Internal peer dispatch. The connection validates and orders every frame first. */
	accept(frame: TunnelFrame): void {
		if (this.failure) return;
		if (frame.kind === TunnelKind.Reset) {
			const value = readTunnelJson(frame.body);
			if (
				typeof value.code !== "string" ||
				!/^[a-z_]{1,64}$/.test(value.code) ||
				typeof value.message !== "string" ||
				new TextEncoder().encode(value.message).length > 256
			)
				throw protocolError();
			this.fail(new DeviceTunnelError(value.code, value.message));
			return;
		}
		if (frame.kind === TunnelKind.Opened) {
			if (this.accepted) throw protocolError();
			this.accepted = true;
			clearTimeout(this.openTimer);
			this.opened.resolve();
			return;
		}
		if (!this.accepted) throw protocolError();
		if (frame.kind === TunnelKind.Data) {
			if (this.remoteFin || frame.body.length > this.receiveCredit)
				throw protocolError();
			this.receiveCredit -= frame.body.length;
			if (this.reader) {
				const reader = this.reader;
				this.reader = undefined;
				this.consumed(frame.body.length);
				reader.resolve(frame.body);
			} else {
				let offset = 0;
				while (offset < frame.body.length) {
					let tail = this.chunks.at(-1);
					if (!tail || tail.length === TUNNEL_MAX_DATA) {
						tail = { bytes: new Uint8Array(TUNNEL_MAX_DATA), length: 0 };
						this.chunks.push(tail);
					}
					const size = Math.min(
						TUNNEL_MAX_DATA - tail.length,
						frame.body.length - offset,
					);
					tail.bytes.set(
						frame.body.subarray(offset, offset + size),
						tail.length,
					);
					tail.length += size;
					offset += size;
				}
				this.queuedBytes += frame.body.length;
				frame.body.fill(0);
			}
			return;
		}
		if (frame.kind === TunnelKind.Window) {
			const credit = new DataView(
				frame.body.buffer,
				frame.body.byteOffset,
				4,
			).getUint32(0);
			if (credit === 0 || credit > TUNNEL_WINDOW - this.sendCredit)
				throw protocolError();
			this.sendCredit += credit;
			this.writer?.resolve();
			return;
		}
		if (frame.kind === TunnelKind.Fin) {
			if (this.remoteFin) throw protocolError();
			this.remoteFin = true;
			this.reader?.resolve(null);
			this.reader = undefined;
			this.maybeRelease();
			return;
		}
		throw protocolError();
	}
	fail(error: Error): void {
		if (this.failure) return;
		this.failure = error;
		clearTimeout(this.openTimer);
		this.opened.reject(error);
		this.reader?.reject(error);
		this.writer?.reject(error);
		this.writeCancelled?.reject(error);
		this.reader = undefined;
		this.writer = undefined;
		for (const chunk of this.chunks) chunk.bytes.fill(0);
		this.chunks.length = 0;
		this.queuedBytes = 0;
		this.release();
		this.completed.resolve();
	}
	private consumed(bytes: number): void {
		this.receiveCredit += bytes;
		if (!this.remoteFin) {
			const credit = new Uint8Array(4);
			new DataView(credit.buffer).setUint32(0, bytes);
			void this.send(TunnelKind.Window, this.id, credit).catch((error) =>
				this.fail(error),
			);
		}
		this.maybeRelease();
	}
	private maybeRelease(): void {
		if (this.localFin && this.remoteFin && this.queuedBytes === 0) {
			this.release();
			this.completed.resolve();
		}
	}
}

interface PendingFrame extends Deferred<void> {
	kind: TunnelKind;
	stream: number;
	body: Uint8Array;
	priority: boolean;
}

/** Multiplexes bounded service streams and preserves them across authenticated key renewal. */
export class DeviceServiceTunnel {
	private readonly streams = new Map<number, DeviceServiceStream>();
	private nextStream = 1;
	private sendSequence = 0n;
	private receiveSequence = 1n;
	private readonly queue: PendingFrame[] = [];
	private queuedBytes = 0;
	private sending = false;
	private activeSend?: PendingFrame;
	private failure?: Error;
	private renewal?: {
		handshake: NoiseHandshake;
		stage: "reply" | "confirmation";
		timer: ReturnType<typeof setTimeout>;
	};
	private renewTimer?: ReturnType<typeof setTimeout>;
	private heartbeat: ReturnType<typeof setInterval>;
	private ping?: { bytes: Uint8Array; at: number };
	private constructor(private readonly transport: TunnelTransport) {
		this.heartbeat = setInterval(() => this.tick(), 15_000);
		this.scheduleRenewal();
		void this.receive().catch((error) =>
			this.close(error instanceof Error ? error : closedError()),
		);
	}
	static async connect(
		options: TunnelConnectOptions,
	): Promise<DeviceServiceTunnel> {
		return new DeviceServiceTunnel(await connectTunnelTransport(options));
	}
	/** A transport seam for other trusted endpoint adapters and protocol tests. */
	static fromTransport(transport: TunnelTransport): DeviceServiceTunnel {
		return new DeviceServiceTunnel(transport);
	}
	get kind(): "webrtc" | "websocket" {
		return this.transport.kind;
	}
	get expiresAt(): number {
		return this.transport.expiresAt;
	}
	get connected(): boolean {
		return this.failure === undefined;
	}
	async open(
		placementId: string,
		serviceId = "hosting",
		options: TunnelServiceOptions = {},
	): Promise<DeviceServiceStream> {
		if (this.failure) throw this.failure;
		if (
			!/^[A-Za-z0-9._:-]{1,128}$/.test(placementId) ||
			!/^[A-Za-z0-9._:-]{1,128}$/.test(serviceId)
		)
			throw new Error(
				"A configured device placement and service are required.",
			);
		return this.openStream(
			TunnelKind.Open,
			{
				placement_id: placementId,
				service_id: serviceId,
				...(options.mode ? { mode: options.mode } : {}),
			},
			options.signal,
		);
	}
	/** One HTTP stream to the device's model gateway; only agents with `model_host` know the target. */
	openModelGateway(
		options: { signal?: AbortSignal } = {},
	): Promise<DeviceServiceStream> {
		return this.openStream(
			TunnelKind.Open,
			{ target: "model_gateway", mode: "http" },
			options.signal,
		);
	}
	openData(
		input: TunnelDataOpen,
		signal?: AbortSignal,
	): Promise<DeviceServiceStream> {
		return this.openStream(TunnelKind.OpenData, input, signal);
	}
	private async openStream(
		kind: TunnelKind,
		input: unknown,
		signal?: AbortSignal,
	): Promise<DeviceServiceStream> {
		if (this.failure) throw this.failure;
		if (signal?.aborted)
			throw signal.reason ?? new Error("Device transfer cancelled.");
		if (
			this.streams.size >= TUNNEL_MAX_STREAMS ||
			this.nextStream > 0xffff_ffff
		)
			throw new Error("The device tunnel has no free service streams.");
		const id = this.nextStream;
		this.nextStream += 2;
		const stream = new DeviceServiceStream(id, this.send.bind(this), () =>
			this.streams.delete(id),
		);
		this.streams.set(id, stream);
		const ready = stream.ready;
		// Observe rejection while the Open frame waits for transport capacity.
		void ready.catch(() => {});
		const abort = () => stream.reset();
		signal?.addEventListener("abort", abort, { once: true });
		try {
			await Promise.race([this.send(kind, id, tunnelJson(input)), ready]);
			await ready;
			return stream;
		} catch (error) {
			stream.fail(error instanceof Error ? error : closedError());
			throw error;
		} finally {
			signal?.removeEventListener("abort", abort);
		}
	}
	close(error: Error = closedError()): void {
		if (this.failure) return;
		this.failure = error;
		clearInterval(this.heartbeat);
		clearTimeout(this.renewTimer);
		if (this.renewal) {
			clearTimeout(this.renewal.timer);
			if (this.renewal.stage === "reply") {
				this.renewal.handshake.close();
				this.renewal.handshake.free();
			}
			this.renewal = undefined;
		}
		this.transport.close();
		this.activeSend?.reject(error);
		for (const stream of this.streams.values()) stream.fail(error);
		for (const frame of this.queue.splice(0)) {
			frame.body.fill(0);
			frame.reject(error);
		}
		this.queuedBytes = 0;
	}
	private send(
		kind: TunnelKind,
		stream: number,
		body: Uint8Array = EMPTY,
		priority = false,
	): Promise<void> {
		if (this.failure) return Promise.reject(this.failure);
		if (
			this.queue.length >= 512 ||
			this.queuedBytes + body.length > TUNNEL_WINDOW * TUNNEL_MAX_STREAMS
		) {
			this.close(
				new DeviceTunnelError(
					"capacity",
					"The device tunnel exceeded its bounded send queue.",
				),
			);
			return Promise.reject(this.failure);
		}
		const pending = { ...deferred<void>(), kind, stream, body, priority };
		this.queue.push(pending);
		this.queuedBytes += body.length;
		void this.flush();
		return pending.promise;
	}
	private async flush(): Promise<void> {
		if (this.sending || this.failure) return;
		this.sending = true;
		try {
			while (!this.failure && this.queue.length) {
				const index = this.renewal
					? this.queue.findIndex((frame) => frame.priority)
					: 0;
				if (index < 0) break;
				const frame = this.queue.splice(index, 1)[0];
				this.activeSend = frame;
				this.queuedBytes -= frame.body.length;
				let plaintext: Uint8Array | undefined;
				try {
					plaintext = encodeTunnelFrame({
						...frame,
						sequence: this.sendSequence++,
					});
					await this.transport.send(plaintext);
					frame.resolve();
				} catch (error) {
					const failure = error instanceof Error ? error : closedError();
					frame.reject(failure);
					this.close(failure);
				} finally {
					this.activeSend = undefined;
					plaintext?.fill(0);
					frame.body.fill(0);
				}
			}
		} finally {
			this.sending = false;
		}
	}
	private async receive(): Promise<void> {
		while (!this.failure) {
			const bytes = await this.transport.next();
			let frame: TunnelFrame;
			try {
				frame = decodeTunnelFrame(bytes);
			} finally {
				bytes.fill(0);
			}
			if (frame.sequence !== this.receiveSequence++) throw protocolError();
			if (
				this.renewal?.stage === "confirmation" &&
				frame.kind !== TunnelKind.Renewed
			)
				throw protocolError();
			if (frame.stream !== 0) {
				const stream = this.streams.get(frame.stream);
				if (stream) stream.accept(frame);
				else if (frame.stream >= this.nextStream) throw protocolError();
				continue;
			}
			await this.control(frame);
		}
	}
	private async control(frame: TunnelFrame): Promise<void> {
		if (frame.kind === TunnelKind.Ping) {
			void this.send(TunnelKind.Pong, 0, frame.body).catch(() => {});
			return;
		}
		if (frame.kind === TunnelKind.Pong) {
			if (
				!this.ping ||
				!frame.body.every((byte, index) => byte === this.ping?.bytes[index])
			)
				throw protocolError();
			this.ping = undefined;
			return;
		}
		if (
			frame.kind === TunnelKind.RenewReply &&
			this.renewal?.stage === "reply"
		) {
			const renewal = this.renewal;
			renewal.handshake.read(frame.body, Math.floor(Date.now() / 1000));
			await this.send(
				TunnelKind.RenewFinish,
				0,
				renewal.handshake.write(Math.floor(Date.now() / 1000)),
				true,
			);
			renewal.stage = "confirmation";
			const session = renewal.handshake.finish(Math.floor(Date.now() / 1000));
			this.transport.installSession(session);
			return;
		}
		if (
			frame.kind === TunnelKind.Renewed &&
			this.renewal?.stage === "confirmation"
		) {
			const expiry = readTunnelJson(frame.body).expires_at;
			if (typeof expiry !== "number" || expiry <= this.transport.expiresAt)
				throw protocolError();
			this.transport.confirmRenewal(expiry);
			clearTimeout(this.renewal.timer);
			this.renewal = undefined;
			if (this.ping) this.ping.at = Date.now();
			this.scheduleRenewal();
			void this.flush();
			return;
		}
		throw protocolError();
	}
	private scheduleRenewal(): void {
		clearTimeout(this.renewTimer);
		this.renewTimer = setTimeout(
			() => this.renew(),
			Math.max(
				1_000,
				(this.transport.expiresAt - Math.floor(Date.now() / 1000) - 60) * 1_000,
			),
		);
	}
	private renew(): void {
		if (this.failure || this.renewal) return;
		try {
			const handshake = this.transport.beginRenewal();
			this.renewal = {
				handshake,
				stage: "reply",
				timer: setTimeout(
					() =>
						this.close(
							new DeviceTunnelError(
								"renewal_timeout",
								"Device tunnel authorization renewal timed out.",
							),
						),
					15_000,
				),
			};
			void this.send(
				TunnelKind.RenewStart,
				0,
				tunnelJson({
					certificate_jws: handshake.certificate(),
					data: base64url(handshake.write(Math.floor(Date.now() / 1000))),
				}),
				true,
			).catch((error) => this.close(error));
		} catch (error) {
			this.close(error instanceof Error ? error : closedError());
		}
	}
	private tick(): void {
		if (this.failure) return;
		if (this.transport.expiresAt <= Math.floor(Date.now() / 1000)) {
			this.close(
				new DeviceTunnelError(
					"expired",
					"Device tunnel authorization expired.",
				),
			);
			return;
		}
		if (this.renewal) return;
		if (this.ping) {
			if (Date.now() - this.ping.at >= 30_000)
				this.close(
					new DeviceTunnelError(
						"heartbeat_timeout",
						"The device tunnel stopped responding.",
					),
				);
			return;
		}
		const bytes = crypto.getRandomValues(new Uint8Array(8));
		this.ping = { bytes: bytes.slice(), at: Date.now() };
		void this.send(TunnelKind.Ping, 0, bytes).catch((error) =>
			this.close(error),
		);
	}
}
