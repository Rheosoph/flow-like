import type { AgentRead } from "../../../../lib/device-management/agent-reads";
import type {
	PreparedModelAsset,
	PreparedProjectArtifact,
} from "../../../../lib/device-management/artifacts";
import type { AgentFeatures } from "../../../../lib/device-management/model/types";
import {
	MODELS_PAGE_MAX,
	MODEL_ENSURE_MAX_PINS,
	type ModelAssetDigest,
	type ModelAssetState,
	type ModelJob,
	modelAssetSummarySchema,
	modelsCommand,
	readModelJobs,
} from "../../../../lib/device-management/models";
import type { ManagementCall } from "../../../../lib/device-management/telemetry";
import { requestOrReject } from "../workspace";
import { DeployRunFailure } from "./update-path";

/*
 * "Ensure models" (plan §3.2), after the commit of a version whose Bit
 * metadata is v2: the device starts or joins a download per model file, this
 * step follows the downloads, and a file the device can't get is sent from
 * this computer automatically. The version is applied once every file is
 * present.
 */

/**
 * Test seam. `pollMs`: between two reads of the device's downloads.
 * `confirmMs`: a list too long to read whole is confirmed this often at most.
 */
export const ensureModelsSeams = { pollMs: 2_000, confirmMs: 15_000 };

/**
 * The agent keeps up to 256 open jobs, in pages that may hold fewer rows than
 * asked when the rows are long; this many pages read all of them.
 */
const MAX_JOB_PAGES = 64;

export type ModelFileState =
	| "waiting"
	| "downloading"
	| "verifying"
	| "sending"
	| "present"
	| "failed"
	| "checking";

/** One model file of a deploy, as the rollout shows it. */
export interface ModelFileRow {
	key: string;
	fileName: string;
	size: number;
	state: ModelFileState;
	/** Bytes the device holds. */
	bytes: number;
	sourceHost?: string;
	bytesPerSecond?: number;
	/** `failed` by the device: its reason (`egress_blocked`, …). */
	reason?: Extract<ModelAssetState, { state: "failed" }>["reason"];
	/** Why sending it from this computer failed. */
	error?: string;
	/**
	 * `sending`: this computer's own copy, a download streamed through, or
	 * (`local_download`) the download into this computer's Bit store first.
	 */
	from?: SendSource;
}

export type SendSource = "bit_store" | "download" | "local_download";

/**
 * Sends one asset from this computer; `progress` reports the bytes the
 * device holds, or with `local_download` the bytes this computer downloaded.
 */
export type ModelAssetSender = (
	asset: PreparedModelAsset,
	jobId: string,
	progress: (bytes: number, from: SendSource) => void,
	signal: AbortSignal,
) => Promise<void>;

export interface EnsureModelsInput {
	call: ManagementCall;
	features(): AgentFeatures | undefined;
	projectId: string;
	models: NonNullable<PreparedProjectArtifact["models"]>;
	/** Absent where this window can't send model files. */
	send?: ModelAssetSender;
	signal: AbortSignal;
	/** Throws while nothing may be sent (locked keys, an abandoned run). */
	guard(): void;
	report(rows: readonly ModelFileRow[]): void;
}

const keyOf = (digest: ModelAssetDigest) => `${digest.algorithm}/${digest.hex}`;

const ACTIVE: ReadonlySet<ModelFileState> = new Set([
	"waiting",
	"downloading",
	"verifying",
	"sending",
]);

function pause(ms: number, signal: AbortSignal): Promise<void> {
	return new Promise((resolve, reject) => {
		const abort = () => {
			clearTimeout(timer);
			reject(signal.reason);
		};
		const timer = setTimeout(() => {
			signal.removeEventListener("abort", abort);
			resolve();
		}, ms);
		signal.addEventListener("abort", abort, { once: true });
	});
}

function rowOf(asset: PreparedModelAsset): ModelFileRow {
	return {
		key: keyOf(asset.descriptor.digest),
		fileName: asset.descriptor.file_name,
		size: asset.descriptor.size,
		state: "waiting",
		bytes: 0,
	};
}

/** The row as the device's state of its download says. */
function fromState(row: ModelFileRow, state: ModelAssetState): ModelFileRow {
	const base = { key: row.key, fileName: row.fileName, size: row.size };
	switch (state.state) {
		case "queued":
			return { ...base, state: "waiting", bytes: 0 };
		case "fetching":
			return { ...base, state: "downloading", bytes: state.bytes };
		case "verifying":
			return { ...base, state: "verifying", bytes: row.size };
		case "present":
			return { ...base, state: "present", bytes: row.size };
		case "awaiting_push":
			return { ...base, state: "sending", bytes: state.bytes };
		case "failed":
			return {
				...base,
				state: "failed",
				bytes: row.bytes,
				reason: state.reason,
			};
	}
}

interface JobsRead {
	jobs: ModelJob[];
	/** Every job was read: a file without one is present. */
	complete: boolean;
}

/**
 * The device's jobs page by page, until every file of this deploy was seen
 * or the list ended. Reading only the first pages could miss a failed file
 * among other downloads, and then it would never be sent.
 */
async function readJobsOf(
	call: ManagementCall,
	features: AgentFeatures | undefined,
	wanted: ReadonlySet<string>,
): Promise<AgentRead<JobsRead>> {
	const jobs: ModelJob[] = [];
	const unseen = new Set(wanted);
	let after: string | undefined;
	for (let page = 0; page < MAX_JOB_PAGES; page++) {
		const read = await readModelJobs(call, features, {
			...(after ? { after } : {}),
			limit: MODELS_PAGE_MAX,
		});
		if (read.kind !== "ok") return read;
		for (const job of read.data.jobs) {
			jobs.push(job);
			unseen.delete(keyOf(job.digest));
		}
		if (read.data.next === null)
			return { kind: "ok", data: { jobs, complete: true } };
		if (!unseen.size) break;
		after = read.data.next;
	}
	return { kind: "ok", data: { jobs, complete: false } };
}

function fromJob(row: ModelFileRow, job: ModelJob): ModelFileRow {
	const next = fromState(row, job);
	if (next.state !== "downloading") return next;
	return {
		...next,
		...(job.source_host ? { sourceHost: job.source_host } : {}),
		...(job.bytes_per_second !== undefined
			? { bytesPerSecond: job.bytes_per_second }
			: {}),
	};
}

/** Follows the device's downloads of a deploy's model files and sends what the device can't get. */
class ModelsEnsure {
	private rows: ModelFileRow[];
	private confirmedAt = 0;
	constructor(private readonly input: EnsureModelsInput) {
		this.rows = input.models.assets.map(rowOf);
	}

	/** Asking again restarts a failed download, so the device is asked once, then only to confirm the end. */
	async run(): Promise<void> {
		let done = await this.confirm();
		while (!done) {
			const jobs = await this.read();
			const stuck = this.stuck(jobs);
			if (stuck) {
				await this.sendFromHere(stuck.asset, stuck.job);
				continue;
			}
			if (this.settled() && this.dueForConfirmation()) {
				done = await this.confirm();
				if (done) break;
			}
			await pause(ensureModelsSeams.pollMs, this.input.signal);
		}
		this.update(
			() => true,
			(row) => ({ ...row, state: "present", bytes: row.size }),
		);
	}

	/** Every file looks present, or the device's list was too long to tell for a while. */
	private dueForConfirmation(): boolean {
		return (
			this.rows.every((row) => row.state === "present") ||
			Date.now() - this.confirmedAt >= ensureModelsSeams.confirmMs
		);
	}

	private emit() {
		this.input.report(this.rows);
	}

	private update(
		matches: (row: ModelFileRow) => boolean,
		change: (row: ModelFileRow) => ModelFileRow,
	) {
		this.rows = this.rows.map((row) => (matches(row) ? change(row) : row));
		this.emit();
	}

	/** Asks the device to start or join every download; true once each file is present. */
	private async confirm(): Promise<boolean> {
		const { input } = this;
		let missing = 0;
		for (
			let at = 0;
			at < input.models.pins.length;
			at += MODEL_ENSURE_MAX_PINS
		) {
			input.signal.throwIfAborted();
			input.guard();
			const response = await requestOrReject(
				input.call,
				modelsCommand({
					kind: "ensure",
					project_id: input.projectId,
					pins: input.models.pins.slice(at, at + MODEL_ENSURE_MAX_PINS),
				}),
				crypto.randomUUID(),
			);
			const summary = modelAssetSummarySchema.safeParse(response.result);
			if (!summary.success)
				throw new DeployRunFailure(
					"model_reply",
					"The device answered the model check with an invalid summary.",
				);
			missing += summary.data.total - summary.data.present;
			for (const status of summary.data.pending)
				this.update(
					(row) => row.key === keyOf(status.digest),
					(row) => fromState(row, status),
				);
		}
		this.confirmedAt = Date.now();
		return missing === 0;
	}

	/** The device's downloads of this deploy's files; a file without one is present once the list is whole. */
	private async read(): Promise<ModelJob[]> {
		const { input } = this;
		input.signal.throwIfAborted();
		input.guard();
		const wanted = new Set(this.rows.map((row) => row.key));
		const read = await readJobsOf(input.call, this.features(), wanted);
		if (read.kind !== "ok")
			throw new DeployRunFailure("agent_feature", "model_store");
		const { jobs, complete } = read.data;
		const byKey = new Map(jobs.map((job) => [keyOf(job.digest), job]));
		this.update(
			() => true,
			(row) => {
				const job = byKey.get(row.key);
				if (job) return fromJob(row, job);
				return complete
					? { ...row, state: "present", bytes: row.size }
					: { ...row, state: row.state === "present" ? "present" : "checking" };
			},
		);
		return jobs;
	}

	/**
	 * The agent's flags. A lock clears them until the device is read again, and
	 * this step may ask first: the Ensure the device answered by then shows a
	 * model store.
	 */
	private features(): AgentFeatures {
		return this.input.features() ?? { model_store: 1 };
	}

	/** The first file the device can't get by itself, in bundle order. */
	private stuck(jobs: readonly ModelJob[]) {
		for (const asset of this.input.models.assets) {
			const key = keyOf(asset.descriptor.digest);
			const job = jobs.find((entry) => keyOf(entry.digest) === key);
			if (job?.state === "failed" && job.reason === "disk_budget")
				throw new DeployRunFailure("model_disk", asset.descriptor.file_name);
			if (job?.state === "failed" || job?.state === "awaiting_push")
				return { asset, job };
		}
		return undefined;
	}

	private settled(): boolean {
		return this.rows.every((row) => !ACTIVE.has(row.state));
	}

	private async sendFromHere(asset: PreparedModelAsset, job: ModelJob) {
		const { input } = this;
		const { file_name: fileName } = asset.descriptor;
		if (!input.send) throw new DeployRunFailure("model_unreachable", fileName);
		const key = keyOf(asset.descriptor.digest);
		const mine = (row: ModelFileRow) => row.key === key;
		input.guard();
		this.update(mine, (row) => ({
			...row,
			state: "sending",
			error: undefined,
		}));
		const started: { at: number; bytes: number; from?: SendSource } = {
			at: Date.now(),
			bytes: -1,
		};
		const progress = (bytes: number, from: SendSource) => {
			const switched = started.from !== from;
			if (switched) Object.assign(started, { at: Date.now(), bytes, from });
			const seconds = (Date.now() - started.at) / 1000;
			const sent = bytes - started.bytes;
			const rate =
				seconds > 0 && sent > 0 ? Math.round(sent / seconds) : undefined;
			const all = from !== "local_download" && bytes >= asset.descriptor.size;
			this.update(mine, (row) => ({
				...row,
				state: all ? "verifying" : "sending",
				bytes,
				from,
				bytesPerSecond: all
					? undefined
					: (rate ?? (switched ? undefined : row.bytesPerSecond)),
			}));
		};
		try {
			await input.send(asset, job.job_id, progress, input.signal);
		} catch (error) {
			const message = error instanceof Error ? error.message : String(error);
			this.update(mine, (row) => ({ ...row, state: "failed", error: message }));
			if (error instanceof DeployRunFailure || input.signal.aborted)
				throw error;
			throw new DeployRunFailure("model_push", fileName);
		}
		this.update(mine, (row) => ({ ...row, state: "present", bytes: row.size }));
	}
}

/** Resolves once the device holds every model file of the version it committed. */
export function ensureModels(input: EnsureModelsInput): Promise<void> {
	return new ModelsEnsure(input).run();
}
