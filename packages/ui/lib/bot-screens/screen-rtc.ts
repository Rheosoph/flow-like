import type { IInboxSealedEnvelope } from "../../state/backend-state/inbox-state";
import { fromBase64Url, randomHex, toBase64Url } from "./base64";
import type { SignalBus } from "./screen-discovery";
import {
	MAX_MESSAGE_BYTES,
	MAX_SDP_BYTES,
	SCREEN_CHANNEL,
} from "./screen-protocol";
import { RecordStream, deriveStreamKeys } from "./screen-records";
import type { ScreenPinnedKey, ScreenSealing } from "./screen-sealing";
import type { ScreenStream } from "./screen-stream";

export const GATHER_MAX_MS = 4000;
export const SETUP_MAX_MS = 15_000;
const BUFFER_HIGH = 1024 * 1024;
const BUFFER_LOW = 256 * 1024;
const DEFAULT_ICE = [{ urls: "stun:stun.l.google.com:19302" }];

export interface ChannelLike {
	binaryType: string;
	readonly readyState: string;
	readonly bufferedAmount: number;
	bufferedAmountLowThreshold: number;
	onmessage: ((event: { data: unknown }) => void) | null;
	onclose: (() => void) | null;
	onerror: (() => void) | null;
	onbufferedamountlow: (() => void) | null;
	send(data: Uint8Array): void;
	close(): void;
}

/** The playground byte stream over one data channel: writes are cut into messages of at most 16 KiB. */
export class DataChannelStream implements ScreenStream {
	private readonly queue: Uint8Array[] = [];
	private waiting: ((chunk: Uint8Array | null) => void) | null = null;
	private ended = false;
	private drain: (() => void) | null = null;

	constructor(private readonly channel: ChannelLike) {
		channel.binaryType = "arraybuffer";
		channel.bufferedAmountLowThreshold = BUFFER_LOW;
		channel.onmessage = (event) => {
			if (event.data instanceof ArrayBuffer)
				this.deliver(new Uint8Array(event.data));
			else if (ArrayBuffer.isView(event.data))
				this.deliver(
					new Uint8Array(
						event.data.buffer,
						event.data.byteOffset,
						event.data.byteLength,
					),
				);
		};
		channel.onclose = channel.onerror = () => this.end();
		channel.onbufferedamountlow = () => this.drain?.();
	}

	private deliver(chunk: Uint8Array): void {
		if (this.waiting) {
			this.waiting(chunk);
			this.waiting = null;
		} else this.queue.push(chunk);
	}

	private end(): void {
		this.ended = true;
		this.waiting?.(null);
		this.waiting = null;
		this.drain?.();
	}

	read(): Promise<Uint8Array | null> {
		const chunk = this.queue.shift();
		if (chunk) return Promise.resolve(chunk);
		if (this.ended) return Promise.resolve(null);
		return new Promise((resolve) => {
			this.waiting = resolve;
		});
	}

	async write(bytes: Uint8Array): Promise<void> {
		for (let offset = 0; offset < bytes.length; offset += MAX_MESSAGE_BYTES) {
			while (!this.ended && this.channel.bufferedAmount > BUFFER_HIGH) {
				await new Promise<void>((resolve) => {
					this.drain = resolve;
				});
			}
			if (this.ended || this.channel.readyState !== "open")
				throw new Error("The connection to the bot's computer closed");
			this.channel.send(bytes.subarray(offset, offset + MAX_MESSAGE_BYTES));
		}
	}

	reset(): void {
		this.channel.onclose = this.channel.onerror = null;
		this.channel.close();
		this.end();
	}
}

export interface ScreenConnection {
	stream: ScreenStream;
	/** Resolves with the reason when the connection ends. */
	closed: Promise<string>;
	close(): void;
}

export class ScreenNotPairedError extends Error {
	readonly code = "not_paired";

	constructor() {
		super(
			"The computer that runs this bot does not know this device; pair it again",
		);
		this.name = "ScreenNotPairedError";
	}
}

export interface ConnectOptions {
	hostPeerId: string;
	/** The pinned key of that host; the offer is sealed to it and to nothing else. */
	hostKey: ScreenPinnedKey;
	sealing: ScreenSealing;
	botId: string;
	iceServers?: RTCIceServer[];
	iceExpiresAt?: number;
	now?: () => number;
	signal?: AbortSignal;
}

const withTimeout = <T>(promise: Promise<T>, ms: number, message: string) => {
	let timer: ReturnType<typeof setTimeout> | undefined;
	const timeout = new Promise<never>((_, reject) => {
		timer = setTimeout(() => reject(new Error(message)), ms);
	});
	return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
};

function gathered(pc: RTCPeerConnection): Promise<void> {
	if (pc.iceGatheringState === "complete") return Promise.resolve();
	return new Promise((resolve) => {
		const done = () => {
			if (pc.iceGatheringState !== "complete") return;
			pc.removeEventListener("icegatheringstatechange", done);
			resolve();
		};
		pc.addEventListener("icegatheringstatechange", done);
	});
}

function iceConfig(options: ConnectOptions): RTCIceServer[] {
	const now = (options.now ?? Date.now)() / 1000;
	const fresh =
		options.iceExpiresAt === undefined || options.iceExpiresAt > now + 30;
	return options.iceServers && fresh ? options.iceServers : DEFAULT_ICE;
}

function refusal(
	code: "busy" | "failed" | "no_key" | "not_paired",
	message: string,
): Error {
	if (code === "not_paired") return new ScreenNotPairedError();
	if (code === "busy")
		return new Error("The bot's computer is serving too many viewers");
	if (code === "no_key")
		return new Error(
			"The desktop app running the bot could not open the end-to-end encrypted offer: this device's key is not the one it paired with",
		);
	return new Error(`The bot's computer refused the connection: ${message}`);
}

function awaitAnswer(
	bus: SignalBus,
	options: ConnectOptions,
	session: string,
): { promise: Promise<IInboxSealedEnvelope>; cancel: () => void } {
	let cancel = () => {};
	const promise = new Promise<IInboxSealedEnvelope>((resolve, reject) => {
		const off = bus.subscribe((signal) => {
			if (
				(signal.type !== "answer" && signal.type !== "refused") ||
				signal.to !== bus.viewerId ||
				signal.from !== options.hostPeerId ||
				signal.session !== session
			)
				return;
			cancel();
			if (signal.type === "answer") resolve(signal.sealed);
			else reject(refusal(signal.code, signal.message));
		});
		const abort = () => {
			cancel();
			reject(new Error("closed"));
		};
		cancel = () => {
			off();
			options.signal?.removeEventListener("abort", abort);
		};
		options.signal?.addEventListener("abort", abort);
	});
	promise.catch(() => {});
	return { promise, cancel: () => cancel() };
}

function answerBody(body: unknown): { sdp: string; secret: Uint8Array } {
	const { sdp, secret } = (body ?? {}) as { sdp?: unknown; secret?: unknown };
	if (typeof sdp !== "string" || sdp.length === 0 || typeof secret !== "string")
		throw new Error("The bot's computer sent an unusable answer");
	const bytes = fromBase64Url(secret);
	if (bytes.length !== 32)
		throw new Error("The bot's computer sent an unusable answer");
	return { sdp, secret: bytes };
}

/** Offers a data channel to a desktop host and resolves once it is open. */
export async function connectScreen(
	bus: SignalBus,
	options: ConnectOptions,
): Promise<ScreenConnection> {
	const pc = new RTCPeerConnection({ iceServers: iceConfig(options) });
	const channel = pc.createDataChannel(SCREEN_CHANNEL, {
		ordered: true,
		protocol: SCREEN_CHANNEL,
	});
	const stream = new DataChannelStream(channel as unknown as ChannelLike);
	let finish!: (reason: string) => void;
	const closed = new Promise<string>((resolve) => {
		finish = resolve;
	});
	const close = () => {
		finish("closed");
		pc.close();
	};
	pc.addEventListener("connectionstatechange", () => {
		if (["failed", "closed", "disconnected"].includes(pc.connectionState)) {
			finish(`connection ${pc.connectionState}`);
			if (pc.connectionState !== "closed") pc.close();
		}
	});
	let answer: ReturnType<typeof awaitAnswer> | undefined;
	let keys!: Awaited<ReturnType<typeof deriveStreamKeys>>;
	try {
		const session = randomHex(16);
		const open = new Promise<void>((resolve, reject) => {
			channel.addEventListener("open", () => resolve());
			channel.addEventListener("error", () =>
				reject(new Error("The data channel failed to open")),
			);
		});
		open.catch(() => {});
		await pc.setLocalDescription(await pc.createOffer());
		await Promise.race([
			gathered(pc),
			new Promise((resolve) => setTimeout(resolve, GATHER_MAX_MS)),
		]);
		const sdp = pc.localDescription?.sdp ?? "";
		if (!sdp || new TextEncoder().encode(sdp).length > MAX_SDP_BYTES)
			throw new Error("The connection offer is unusable");
		const viewerSecret = crypto.getRandomValues(new Uint8Array(32));
		const own = await options.sealing.own();
		const sealed = await options.sealing.seal(
			{ session, message: "offer" },
			{
				sdp,
				secret: toBase64Url(viewerSecret),
				bot_id: options.botId,
				key_id: own.key_id,
			},
			options.hostKey,
		);
		answer = awaitAnswer(bus, options, session);
		await bus.send({
			type: "offer",
			from: bus.viewerId,
			to: options.hostPeerId,
			bot_id: options.botId,
			session,
			sealed,
		});
		const reply = await options.sealing.open(
			{ session, message: "answer" },
			await withTimeout(
				answer.promise,
				SETUP_MAX_MS,
				"The bot's computer did not answer",
			),
		);
		const { sdp: answerSdp, secret } = answerBody(reply);
		await pc.setRemoteDescription({ type: "answer", sdp: answerSdp });
		await withTimeout(open, SETUP_MAX_MS, "The bot's computer did not connect");
		keys = await deriveStreamKeys(viewerSecret, secret, session);
	} catch (error) {
		answer?.cancel();
		close();
		throw error;
	}
	channel.addEventListener("close", () => finish("channel closed"));
	return { stream: new RecordStream(stream, keys), closed, close };
}
