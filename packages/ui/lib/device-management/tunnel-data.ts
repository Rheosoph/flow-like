import type { ArtifactTransferStatus } from "./artifacts";
import type { ModelAssetStatus } from "./models";
import {
	type DeviceServiceStream,
	DeviceServiceTunnel,
	type TunnelServiceOptions,
} from "./tunnel";
import {
	TUNNEL_MODEL_ASSET_MAX_BYTES,
	TUNNEL_WINDOW,
	type TunnelDataOpen,
} from "./tunnel-protocol";
import type { TunnelConnectOptions } from "./tunnel-transport";
import type { ManagementResponse } from "./types";

export const TUNNEL_READ_COMMANDS: ReadonlySet<string> = new Set([
	"inspect",
	"inspect_page",
	"logs",
	"messages",
	"metrics_history",
	"telemetry_read",
	"telemetry_roster_read",
	"archive_read",
	"archive_roster_read",
]);
export const TUNNEL_RESPONSE_LIMIT = 1024 * 1024;
const VERIFY_MIN_MS = 15 * 60_000;
/** The device reads the whole file back to verify it: 10 MB/s, a slow disk's pace. */
const VERIFY_BYTES_PER_MS = 10_000;
const JOB_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const DIGEST_HEX = /^[0-9a-f]{64}$/;
const MODEL_ASSET_STATES: ReadonlySet<string> = new Set([
	"queued",
	"fetching",
	"verifying",
	"present",
	"awaiting_push",
	"failed",
]);

/** A bulk read the data stream carries: the listed types, and `models` usage statistics (up to 1 MiB). */
export function isTunnelReadCommand(command: Record<string, unknown>): boolean {
	if (TUNNEL_READ_COMMANDS.has(String(command.type))) return true;
	const request = command.request as Record<string, unknown> | undefined;
	return command.type === "models" && request?.kind === "stats";
}

/** Read in order, one window at a time; the stream asks for no byte twice. */
export interface TunnelUploadFile {
	readonly size: number;
	slice(start: number, end: number): { arrayBuffer(): Promise<ArrayBuffer> };
}

export interface TunnelArtifactUpload {
	projectId: string;
	transferId: string;
	fileIndex: number | null;
	offset: number;
	file: TunnelUploadFile;
	signal?: AbortSignal;
}

/**
 * The bytes of one model asset job from `offset` (where the device's copy
 * ends) to the end of `file`. Without `file` the stream carries no bytes: the
 * device opens its push session and answers where its copy ends.
 */
export interface TunnelModelAssetPush {
	jobId: string;
	offset: number;
	file?: TunnelUploadFile;
	signal?: AbortSignal;
	/** Bytes the device holds once each window is written. */
	onProgress?(bytes: number): void;
}

export class TunnelDataRequestError extends Error {
	constructor(
		readonly sent: boolean,
		readonly reason: unknown,
	) {
		super(
			reason instanceof Error
				? reason.message
				: "The encrypted device transfer failed.",
		);
		this.name = "TunnelDataRequestError";
	}
}

type DataTunnel = Pick<
	DeviceServiceTunnel,
	"openData" | "connected" | "close" | "kind"
> &
	Partial<Pick<DeviceServiceTunnel, "open" | "openModelGateway">>;
type Connector = (options: TunnelConnectOptions) => Promise<DataTunnel>;

/** Owned by the live device session, including its management-connection replacements. */
export class DeviceTunnelDataClient {
	private tunnel?: DataTunnel;
	private connecting?: Promise<DataTunnel>;
	private readonly abort = new AbortController();
	constructor(
		private readonly options: TunnelConnectOptions,
		private readonly connect: Connector = DeviceServiceTunnel.connect,
	) {
		this.options = {
			...options,
			receipt: {
				...options.receipt,
				identity: options.receipt.identity
					? {
							...options.receipt.identity,
							management_key: [...options.receipt.identity.management_key],
						}
					: options.receipt.identity,
			},
		};
	}
	matches(options: TunnelConnectOptions): boolean {
		const previous = this.options.receipt;
		const next = options.receipt;
		return (
			this.options.controller === options.controller &&
			(this.options.grantId ?? "owner") === (options.grantId ?? "owner") &&
			previous.device_id === next.device_id &&
			previous.auth_epoch === next.auth_epoch &&
			previous.identity.management_key.length ===
				next.identity.management_key.length &&
			previous.identity.management_key.every(
				(byte, index) => byte === next.identity.management_key[index],
			)
		);
	}
	close(): void {
		this.abort.abort(new Error("The device data session closed."));
		this.tunnel?.close();
		this.tunnel = undefined;
	}
	openService(
		placementId: string,
		serviceId: string,
		options: TunnelServiceOptions = {},
	): Promise<DeviceServiceStream> {
		return this.openStream(options.signal, (tunnel, signal) => {
			if (!tunnel.open) throw new Error("Service tunnels are unavailable.");
			return tunnel.open(placementId, serviceId, { ...options, signal });
		});
	}
	/** One HTTP stream to the device's model gateway. */
	openModelGateway(
		options: { signal?: AbortSignal } = {},
	): Promise<DeviceServiceStream> {
		return this.openStream(options.signal, (tunnel, signal) => {
			if (!tunnel.openModelGateway)
				throw new Error("Model gateway streams are unavailable.");
			return tunnel.openModelGateway({ signal });
		});
	}
	/** Opens one stream with `open`; a cancel from the caller or the session resets it. */
	private async openStream(
		signal: AbortSignal | undefined,
		open: (
			tunnel: DataTunnel,
			signal: AbortSignal,
		) => Promise<DeviceServiceStream>,
	): Promise<DeviceServiceStream> {
		const abort = new AbortController();
		let stream: DeviceServiceStream | undefined;
		let reject!: (error: unknown) => void;
		const stopped = new Promise<never>((_, fail) => {
			reject = fail;
		});
		void stopped.catch(() => {});
		const cancel = () => {
			abort.abort();
			stream?.reset();
			reject(
				new DOMException("The service connection was cancelled.", "AbortError"),
			);
		};
		const sources = [this.abort.signal, signal].filter(
			(value): value is AbortSignal => !!value,
		);
		const cleanup = () => {
			for (const source of sources) source.removeEventListener("abort", cancel);
		};
		for (const source of sources) {
			source.addEventListener("abort", cancel, { once: true });
			if (source.aborted) cancel();
		}
		try {
			return await Promise.race([
				stopped,
				(async () => {
					abort.signal.throwIfAborted();
					const tunnel = await this.get();
					abort.signal.throwIfAborted();
					stream = await open(tunnel, abort.signal);
					if (abort.signal.aborted) {
						stream.reset();
						abort.signal.throwIfAborted();
					}
					void stream.closed.then(cleanup);
					return stream;
				})(),
			]);
		} catch (error) {
			cleanup();
			stream?.reset();
			throw error;
		}
	}
	async request(
		command: Record<string, unknown>,
		operationId: string,
		signal?: AbortSignal,
	): Promise<ManagementResponse> {
		if (!isTunnelReadCommand(command))
			throw new Error("This operation requires the management connection.");
		let sent = false;
		try {
			const value = await this.run(signal, async (tunnel, operation) => {
				const issued = Math.floor(Date.now() / 1000);
				sent = true;
				const stream = await operation.open(tunnel, {
					kind: "request",
					request: {
						operation_id: operationId,
						device_id: this.options.receipt.device_id,
						issued_at: issued,
						expires_at: issued + 60,
						command,
					},
				});
				await stream.end();
				return readJson(stream, operation.touch, TUNNEL_RESPONSE_LIMIT);
			});
			if (
				value.operation_id !== operationId ||
				typeof value.state !== "string" ||
				!value.result ||
				typeof value.result !== "object"
			)
				throw new Error(
					"Device data response does not match the requested operation.",
				);
			return value as unknown as ManagementResponse;
		} catch (error) {
			throw new TunnelDataRequestError(sent, error);
		}
	}
	async uploadArtifact(
		input: TunnelArtifactUpload,
	): Promise<ArtifactTransferStatus> {
		if (
			!Number.isSafeInteger(input.file.size) ||
			input.file.size < 0 ||
			input.file.size > 4 * 1024 ** 3 ||
			!Number.isSafeInteger(input.offset) ||
			input.offset < 0 ||
			input.offset > input.file.size
		)
			throw new Error("Invalid device artifact upload range.");
		const value = await this.run(input.signal, async (tunnel, operation) => {
			const stream = await operation.open(tunnel, {
				kind: "artifact",
				project_id: input.projectId,
				transfer_id: input.transferId,
				file_index: input.fileIndex,
				offset: input.offset,
			});
			await writeSlices(stream, operation, input.file, input.offset, {
				changed: "Device artifact file changed while uploading.",
			});
			await stream.end();
			operation.verification();
			return readJson(stream, operation.touch, 64 * 1024);
		});
		const descriptor = value.descriptor as Record<string, unknown> | undefined;
		if (
			value.transfer_id !== input.transferId ||
			!descriptor ||
			descriptor.project_id !== input.projectId ||
			!["receiving", "committed", "aborted"].includes(String(value.state)) ||
			!Number.isSafeInteger(value.expires_at) ||
			Number(value.expires_at) <= 0 ||
			typeof value.manifest_ready !== "boolean" ||
			value.file_index !== input.fileIndex ||
			!Number.isSafeInteger(value.offset) ||
			Number(value.offset) < input.offset ||
			Number(value.offset) > input.file.size ||
			typeof value.complete !== "boolean" ||
			(value.complete && value.offset !== input.file.size) ||
			(value.project_path !== null && typeof value.project_path !== "string")
		)
			throw new Error("Device artifact response does not match this upload.");
		return value as unknown as ArtifactTransferStatus;
	}
	/** Sends one model asset job's missing bytes; the device answers the job's state once it has them. */
	async pushModelAsset(input: TunnelModelAssetPush): Promise<ModelAssetStatus> {
		const size = input.file?.size ?? 0;
		if (!pushRange(input.jobId, input.offset, size))
			throw new Error(
				`Invalid model asset push range: offset ${input.offset} of ${size} bytes for job ${input.jobId}.`,
			);
		const value = await this.run(input.signal, async (tunnel, operation) => {
			const stream = await operation.open(tunnel, {
				kind: "model_asset",
				job_id: input.jobId,
				offset: input.offset,
			});
			if (input.file)
				await writeSlices(stream, operation, input.file, input.offset, {
					changed: "The model file changed while it was being sent.",
					written: input.onProgress,
				});
			await stream.end();
			operation.verification(size);
			return readJson(stream, operation.touch, 64 * 1024);
		});
		return modelAssetReply(value, input.jobId);
	}
	private get(): Promise<DataTunnel> {
		if (this.abort.signal.aborted)
			return Promise.reject(this.abort.signal.reason);
		if (this.tunnel?.connected) return Promise.resolve(this.tunnel);
		if (this.connecting) return this.connecting;
		this.connecting = this.connect({
			...this.options,
			signal: this.abort.signal,
		})
			.then((tunnel) => {
				if (this.abort.signal.aborted) {
					tunnel.close();
					throw this.abort.signal.reason;
				}
				this.tunnel = tunnel;
				return tunnel;
			})
			.finally(() => {
				this.connecting = undefined;
			});
		return this.connecting;
	}
	private async run<T>(
		signal: AbortSignal | undefined,
		run: (tunnel: DataTunnel, operation: DataOperation) => Promise<T>,
	): Promise<T> {
		const operation = new DataOperation(signal, this.abort.signal);
		try {
			operation.assertLive();
			return await operation.race(
				(async () => {
					const tunnel = await this.get();
					operation.assertLive();
					operation.touch();
					return run(tunnel, operation);
				})(),
			);
		} finally {
			operation.dispose();
		}
	}
}

class DataOperation {
	private readonly abort = new AbortController();
	private stream?: DeviceServiceStream;
	private timer?: ReturnType<typeof setTimeout>;
	private idleMs = 60_000;
	private readonly failure: Promise<never>;
	private reject!: (error: Error) => void;
	private readonly listeners: [AbortSignal, () => void][] = [];
	constructor(...signals: (AbortSignal | undefined)[]) {
		this.failure = new Promise((_, reject) => {
			this.reject = reject;
		});
		void this.failure.catch(() => {});
		for (const signal of signals) {
			if (!signal) continue;
			const fail = () =>
				this.fail(
					new DOMException("The device transfer was cancelled.", "AbortError"),
				);
			this.listeners.push([signal, fail]);
			signal.addEventListener("abort", fail, { once: true });
			if (signal.aborted) fail();
		}
		this.touch();
	}
	readonly touch = () => {
		clearTimeout(this.timer);
		if (!this.abort.signal.aborted)
			this.timer = setTimeout(
				() =>
					this.fail(new Error("The device data transfer stopped responding.")),
				this.idleMs,
			);
	};
	/** After the last byte the device verifies `bytes` before it answers; a large file gets longer. */
	verification(bytes = 0): void {
		this.idleMs = Math.max(
			VERIFY_MIN_MS,
			Math.ceil(bytes / VERIFY_BYTES_PER_MS),
		);
		this.touch();
	}
	assertLive(): void {
		if (this.abort.signal.aborted) throw this.abort.signal.reason;
	}
	async open(
		tunnel: DataTunnel,
		input: TunnelDataOpen,
	): Promise<DeviceServiceStream> {
		this.assertLive();
		const stream = await tunnel.openData(input, this.abort.signal);
		this.stream = stream;
		if (this.abort.signal.aborted) stream.reset();
		this.assertLive();
		this.touch();
		return stream;
	}
	race<T>(result: Promise<T>): Promise<T> {
		return Promise.race([result, this.failure]);
	}
	dispose(): void {
		clearTimeout(this.timer);
		for (const [signal, listener] of this.listeners)
			signal.removeEventListener("abort", listener);
		if (this.stream && !this.stream.finished) this.stream.reset();
	}
	private fail(error: Error): void {
		if (this.abort.signal.aborted) return;
		this.abort.abort(error);
		this.stream?.reset();
		this.reject(error);
	}
}

/** Writes `file` from `from` on in window-sized slices, each read only once the previous one was accepted. */
async function writeSlices(
	stream: DeviceServiceStream,
	operation: DataOperation,
	file: TunnelUploadFile,
	from: number,
	report: { changed: string; written?: (bytes: number) => void },
): Promise<void> {
	for (let offset = from; offset < file.size; ) {
		operation.assertLive();
		const end = Math.min(offset + TUNNEL_WINDOW, file.size);
		const bytes = new Uint8Array(await file.slice(offset, end).arrayBuffer());
		try {
			operation.assertLive();
			if (bytes.length !== end - offset) throw new Error(report.changed);
			await stream.write(bytes, operation.touch);
		} finally {
			bytes.fill(0);
		}
		offset = end;
		operation.touch();
		report.written?.(offset);
	}
}

/** A canonical job and an offset within a file of at most 64 GiB. */
function pushRange(jobId: string, offset: number, size: number) {
	return (
		JOB_ID.test(jobId) &&
		Number.isSafeInteger(size) &&
		size >= 0 &&
		size <= TUNNEL_MODEL_ASSET_MAX_BYTES &&
		Number.isSafeInteger(offset) &&
		offset >= 0 &&
		offset <= size
	);
}

function isDigest(value: unknown) {
	const digest = value as Record<string, unknown> | null | undefined;
	return (
		typeof digest === "object" &&
		digest !== null &&
		(digest.algorithm === "sha256" || digest.algorithm === "blake3") &&
		typeof digest.hex === "string" &&
		DIGEST_HEX.test(digest.hex)
	);
}

/** The device's answer to a push: one job's state, for the job pushed. */
export function modelAssetReply(
	value: Record<string, unknown>,
	jobId: string,
): ModelAssetStatus {
	if (
		!isDigest(value.digest) ||
		(value.job_id !== undefined && value.job_id !== jobId) ||
		!MODEL_ASSET_STATES.has(String(value.state))
	)
		throw new Error(
			`The device's answer to the push of job ${jobId} does not match it.`,
		);
	return value as unknown as ModelAssetStatus;
}

async function readJson(
	stream: DeviceServiceStream,
	touch: () => void,
	maximum: number,
): Promise<Record<string, unknown>> {
	let bytes: Uint8Array = new Uint8Array(Math.min(16_384, maximum));
	let total = 0;
	try {
		for (;;) {
			const chunk = await stream.read();
			touch();
			if (chunk === null) break;
			const required = total + chunk.length;
			if (required > maximum) {
				chunk.fill(0);
				throw new Error("Device data response exceeded its byte limit.");
			}
			if (required > bytes.length) {
				const grown = new Uint8Array(
					Math.min(maximum, Math.max(required, bytes.length * 2)),
				);
				grown.set(bytes.subarray(0, total));
				bytes.fill(0);
				bytes = grown;
			}
			bytes.set(chunk, total);
			total = required;
			chunk.fill(0);
		}
		try {
			const value: unknown = JSON.parse(
				new TextDecoder("utf-8", { fatal: true }).decode(
					bytes.subarray(0, total),
				),
			);
			if (!value || typeof value !== "object" || Array.isArray(value))
				throw new Error("Invalid device data response.");
			return value as Record<string, unknown>;
		} finally {
			bytes.fill(0);
		}
	} finally {
		bytes.fill(0);
	}
}
