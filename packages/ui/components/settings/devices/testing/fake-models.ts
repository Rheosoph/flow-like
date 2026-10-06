import {
	type ModelHostSample,
	emptyModels,
	overviewOf,
	rankRecommendations,
	statsOf,
} from "../../../../lib/device-management/model/__fixtures__/sample-models";
import type { AgentFeature } from "../../../../lib/device-management/model/types";
import {
	type HostedModel,
	MODELS_PAGE_MAX,
	MODEL_STATS_MAX_POINTS,
	type ModelAssetDigest,
	type ModelAssetStatus,
	type ModelJob,
	type ModelSettings,
	type Residency,
	type RuntimeInfo,
	STATS_STEPS,
	type StatsStep,
} from "../../../../lib/device-management/models";
import { backendOf } from "../models/models-view";

/*
 * The model host of a fake agent (plan §3.4, §3.5): every `models` request is
 * answered from a `ModelHostSample`, and writes change it the way the agent's
 * management handler and supervisor do (`apps/standalone/src/management/models.rs`,
 * `models/supervisor.rs`). A load answers `loading` and finishes by the next
 * read; downloads stay where they are until the test moves them on:
 *
 *   const api = fakeDeviceApi({ modelHosts: { [id]: gpuBoxModels() } });
 *   api.agent(id).models.finishJobs();                     // every download that hasn't failed is done
 *   api.agent(id).models.finishLoads();                    // loads under way are done without a read
 *   api.agent(id).models.refuse("remove", "busy", "…");    // one request kind refused until restored
 */

export interface ModelsReply {
	state: "completed" | "rejected";
	result: Record<string, unknown>;
}

const done = (result: object): ModelsReply => ({
	state: "completed",
	result: result as Record<string, unknown>,
});

const refused = (code: string, error: string): ModelsReply => ({
	state: "rejected",
	result: { code, error, retryable: false },
});

const READ_KINDS = new Set([
	"overview",
	"models",
	"stats",
	"jobs",
	"recommendations",
	"probe",
]);
/** Acquisition exists from `model_store` on, before an agent hosts models. */
const STORE_KINDS = new Set(["jobs", "ensure", "cancel_job"]);
const DEFAULT_PAGE: Record<string, number> = {
	models: 8,
	jobs: 16,
	recommendations: 8,
};
const PACK_HOST = "cdn.flow-like.com";
const DOWNLOAD_RATE = 26_214_400;
const GIB = 1024 ** 3;

type Request = Record<string, unknown>;

const requestOf = (command: Request): Request =>
	command.request && typeof command.request === "object"
		? (command.request as Request)
		: {};

const kindOf = (command: Request): string => {
	const kind = requestOf(command).kind;
	return typeof kind === "string" ? kind : "";
};

/** The flag a `models` command needs on the agent. */
export function modelsFeature(command: Request): AgentFeature {
	return STORE_KINDS.has(kindOf(command)) ? "model_store" : "model_host";
}

/** Reads are answered from the current state and never journaled. */
export function isModelsRead(command: Request): boolean {
	return READ_KINDS.has(kindOf(command));
}

const text = (value: unknown) => (typeof value === "string" ? value : "");

function pageLimit(request: Request, kind: string): number | ModelsReply {
	const limit = request.limit ?? DEFAULT_PAGE[kind];
	return typeof limit === "number" &&
		Number.isInteger(limit) &&
		limit >= 1 &&
		limit <= MODELS_PAGE_MAX
		? limit
		: refused(
				"invalid",
				`Page limit must be 1 to ${MODELS_PAGE_MAX}, got ${String(limit)}.`,
			);
}

/** Rows sorted by key: the page after `after`, and the cursor of the next one. */
function pageSorted<T>(
	rows: readonly T[],
	keyOf: (row: T) => string,
	after: unknown,
	limit: number,
) {
	const sorted = [...rows].sort((a, b) => (keyOf(a) < keyOf(b) ? -1 : 1));
	const start =
		typeof after === "string"
			? sorted.findIndex((row) => keyOf(row) > after)
			: 0;
	return slicePage(sorted, keyOf, start, limit);
}

/**
 * Rows in their own order. Like the agent's recommendations, computed per read:
 * the cursor counts the rows read before.
 */
function pageCounted<T>(rows: readonly T[], after: unknown, limit: number) {
	const start = typeof after === "string" ? Number(after) : 0;
	if (!Number.isInteger(start) || start < 0) return undefined;
	const page = slicePage(rows, () => "", start, limit);
	const read = start + page.rows.length;
	return { rows: page.rows, next: read < rows.length ? String(read) : null };
}

/** Like the agent's pages: at most `limit` rows, and only as many as fit one encrypted reply. */
const PAGE_BYTES = 16 * 1024 - 1024;

function slicePage<T>(
	rows: readonly T[],
	keyOf: (row: T) => string,
	start: number,
	limit: number,
) {
	if (start < 0) return { rows: [], next: null };
	const page: T[] = [];
	let bytes = 0;
	for (const row of rows.slice(start, start + limit)) {
		bytes += new TextEncoder().encode(JSON.stringify(row)).length + 1;
		if (bytes > PAGE_BYTES) break;
		page.push(row);
	}
	const last = page.at(-1);
	const more = start + page.length < rows.length;
	return { rows: page, next: more && last ? keyOf(last) : null };
}

const sameDigest = (a: ModelAssetDigest, b: ModelAssetDigest) =>
	a.algorithm === b.algorithm && a.hex === b.hex;

/** Canonical JSON: the agent compares settings and specs as structs, whatever the key order. */
function canonical(value: unknown): string {
	if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
	if (value && typeof value === "object")
		return `{${Object.entries(value)
			.filter(([, entry]) => entry !== undefined)
			.sort(([a], [b]) => (a < b ? -1 : 1))
			.map(([key, entry]) => `${JSON.stringify(key)}:${canonical(entry)}`)
			.join(",")}}`;
	return JSON.stringify(value);
}

/** What a hosted model shows of its spec: a seeded model has no other record of it. */
const shownSpec = (
	model: Pick<HostedModel, "display_name" | "kind" | "engine" | "assets">,
) =>
	canonical({
		display_name: model.display_name,
		kind: model.kind,
		engine: model.engine,
		assets: model.assets,
	});

/** A model without its state fields, so another state can be spread over it. */
function stateless(model: HostedModel) {
	const {
		state: _state,
		ram_bytes: _ram,
		vram_bytes: _vram,
		slots: _slots,
		slots_busy: _busy,
		reason: _reason,
		...rest
	} = model as HostedModel & Record<string, unknown>;
	return rest as Omit<HostedModel, "state">;
}

function packDigest(runtime: string, backend: string): ModelAssetDigest {
	const seed = `${runtime}-${backend}`;
	let hex = "";
	for (let index = 0; hex.length < 64; index++)
		hex += (seed.charCodeAt(index % seed.length) % 16).toString(16);
	return { algorithm: "sha256", hex };
}

/** A started download as `install` lists it under `pending`. */
const pendingOf = (job: ModelJob): ModelAssetStatus => ({
	digest: job.digest,
	job_id: job.job_id,
	state: "fetching",
	source_index: 0,
	bytes: 0,
});

function sourceHost(sources: unknown): string {
	const first = Array.isArray(sources) ? sources[0] : undefined;
	if (typeof first !== "string") return PACK_HOST;
	try {
		return new URL(first).host;
	} catch {
		return PACK_HOST;
	}
}

interface AssetInput {
	digest: ModelAssetDigest;
	size: number;
	file_name: string;
	sources?: string[];
}

export class FakeModelHost {
	state: ModelHostSample;
	/** Requests every write sent, oldest first: `kind` plus the request as received. */
	readonly writes: Request[] = [];
	private readonly refusals = new Map<string, ModelsReply>();
	/** Runtime pack downloads by job id. */
	private readonly packJobs = new Map<
		string,
		Pick<RuntimeInfo, "runtime" | "backend">
	>();
	/** Loads under way, as the agent runs them in the background: they finish by the next read. */
	private readonly loads = new Set<string>();
	/** The spec each model was installed with here, canonical: a repeated install compares against it. */
	private readonly specs = new Map<string, string>();

	/** `undefined`: a host that holds nothing yet (`emptyModels()`). */
	constructor(
		sample: ModelHostSample | undefined,
		private readonly now: () => number,
	) {
		this.state = structuredClone(sample ?? emptyModels());
	}

	reset(sample: ModelHostSample): void {
		this.state = structuredClone(sample);
		this.packJobs.clear();
		this.loads.clear();
		this.specs.clear();
	}

	/** Loads under way finish: their models are loaded. */
	finishLoads(): void {
		const pending = [...this.loads];
		this.loads.clear();
		for (const id of pending) {
			const model = this.model(id);
			if (model?.state === "loading") this.loaded(model);
		}
	}

	/** Refuse requests of `kind` with a coded rejection until the returned function is called. */
	refuse(kind: string, code: string, error: string): () => void {
		const reply = refused(code, error);
		this.refusals.set(kind, reply);
		return () => {
			if (this.refusals.get(kind) === reply) this.refusals.delete(kind);
		};
	}

	/** Downloads that haven't failed finish: their files are present, models stop waiting and runtimes install. */
	finishJobs(): void {
		const at = this.now();
		this.state.jobs = this.state.jobs.map((job) => {
			if (job.state === "failed" || job.state === "present") return job;
			const pack = this.packJobs.get(job.job_id);
			if (pack) this.runtime(pack.runtime, pack.backend, true);
			this.state.store.bytes += job.size;
			const {
				state: _state,
				bytes: _bytes,
				source_index: _source,
				bytes_per_second: _rate,
				...rest
			} = job as ModelJob & Record<string, unknown>;
			return { ...(rest as ModelJob), updated_at: at, state: "present" };
		});
		this.state.models = this.state.models.map((model) =>
			model.state === "acquiring" && this.allPresent(model.assets)
				? { ...stateless(model), state: "stopped" }
				: model,
		);
	}

	answer(command: Request): ModelsReply {
		const request = requestOf(command);
		const kind = kindOf(command);
		const refusal = this.refusals.get(kind);
		if (refusal) return refusal;
		if (READ_KINDS.has(kind)) this.finishLoads();
		else this.writes.push({ ...request });
		const handlers: Record<string, (request: Request) => ModelsReply> = {
			overview: () => done(overviewOf(this.state, this.now())),
			models: (input) => this.modelPage(input),
			jobs: (input) => this.jobPage(input),
			recommendations: (input) => this.recommendationPage(input),
			probe: () => done(this.state.system),
			stats: (input) => this.stats(input),
			install: (input) => this.install(input),
			configure: (input) => this.configure(input),
			load: (input) => this.load(input),
			unload: (input) => this.unload(input),
			remove: (input) => this.remove(input),
			ensure: (input) => this.ensure(input),
			install_runtime: (input) => this.installRuntime(input),
			remove_runtime: (input) => this.removeRuntime(input),
			cancel_job: (input) => this.cancelJob(input),
		};
		const handler = handlers[kind];
		return handler
			? handler(request)
			: refused("invalid", `Unknown models request "${kind}".`);
	}

	/* Reads. */

	private modelPage(request: Request): ModelsReply {
		const limit = pageLimit(request, "models");
		if (typeof limit !== "number") return limit;
		const page = pageSorted(
			this.state.models,
			(model) => model.id,
			request.after,
			limit,
		);
		return done({ models: page.rows, next: page.next });
	}

	private jobPage(request: Request): ModelsReply {
		const limit = pageLimit(request, "jobs");
		if (typeof limit !== "number") return limit;
		const page = pageSorted(
			this.state.jobs,
			(job) => job.job_id,
			request.after,
			limit,
		);
		return done({ jobs: page.rows, next: page.next });
	}

	private recommendationPage(request: Request): ModelsReply {
		const limit = pageLimit(request, "recommendations");
		if (typeof limit !== "number") return limit;
		const page = pageCounted(
			rankRecommendations(this.state.recommendations),
			request.after,
			limit,
		);
		if (!page)
			return refused(
				"invalid",
				`Recommendation cursor ${text(request.after)} is not a count of recommendations.`,
			);
		return done({ recommendations: page.rows, next: page.next });
	}

	private stats(request: Request): ModelsReply {
		const { from, to, step } = request;
		const width = STATS_STEPS[step as StatsStep];
		const points =
			typeof from === "number" && typeof to === "number" && width
				? (to - from) / width
				: 0;
		if (
			typeof from !== "number" ||
			typeof to !== "number" ||
			!width ||
			from <= 0 ||
			from % width !== 0 ||
			to % width !== 0 ||
			points < 1 ||
			points > MODEL_STATS_MAX_POINTS
		)
			return refused("invalid", "Model stats range.");
		const modelId =
			typeof request.model_id === "string" ? request.model_id : null;
		return done(
			statsOf(this.state, { modelId, from, to, step: step as StatsStep }),
		);
	}

	/* Writes. */

	private model(id: unknown): HostedModel | undefined {
		return this.state.models.find((model) => model.id === id);
	}

	private replace(next: HostedModel): HostedModel {
		this.state.models = this.state.models.map((model) =>
			model.id === next.id ? next : model,
		);
		return next;
	}

	/** The agent reads a model a write names that is gone as a stale read. */
	private unknownModel(id: unknown): ModelsReply {
		return refused(
			"revision_conflict",
			`Model ${text(id)} is not hosted on this device.`,
		);
	}

	private changed(id: unknown): ModelsReply {
		return refused(
			"revision_conflict",
			`Model ${text(id)} changed on the device. Read it again.`,
		);
	}

	private present(digest: ModelAssetDigest): boolean {
		const job = this.state.jobs.find((row) => sameDigest(row.digest, digest));
		if (job) return job.state === "present";
		return this.state.models.some(
			(model) =>
				model.state !== "acquiring" &&
				model.assets.some((asset) => sameDigest(asset, digest)),
		);
	}

	private allPresent(digests: readonly ModelAssetDigest[]): boolean {
		return digests.every((digest) => this.present(digest));
	}

	/** One job per file, like the agent's: a failed one starts again under its own id. */
	private startJob(asset: AssetInput): ModelJob {
		const known = this.state.jobs.find((job) =>
			sameDigest(job.digest, asset.digest),
		);
		if (known && known.state !== "failed") return known;
		const job: ModelJob = {
			job_id: known?.job_id ?? crypto.randomUUID(),
			digest: asset.digest,
			size: asset.size,
			file_name: asset.file_name,
			source_host: sourceHost(asset.sources),
			bytes_per_second: DOWNLOAD_RATE,
			updated_at: this.now(),
			state: "fetching",
			source_index: 0,
			bytes: 0,
		};
		this.state.jobs = known
			? this.state.jobs.map((row) => (row === known ? job : row))
			: [...this.state.jobs, job];
		return job;
	}

	/** The same spec again answers the model and restarts its failed downloads; another spec under its id is refused. */
	private reinstall(model: HostedModel, spec: Request): ModelsReply {
		const known = this.specs.get(model.id) ?? shownSpec(model);
		const asked = this.specs.has(model.id)
			? canonical(spec)
			: shownSpec({
					display_name: text(spec.display_name),
					kind: spec.kind as HostedModel["kind"],
					engine: spec.engine as HostedModel["engine"],
					assets: (Array.isArray(spec.assets) ? spec.assets : []).map(
						(asset: AssetInput) => asset.digest,
					),
				});
		if (asked !== known)
			return refused(
				"revision_conflict",
				`Install model ${model.id}: this id already hosts another model`,
			);
		const assets = (spec.assets ?? []) as AssetInput[];
		const pending = assets
			.filter((asset) => !this.present(asset.digest))
			.map((asset) => this.startJob(asset));
		return done({
			model: this.model(model.id),
			assets: {
				total: assets.length,
				present: assets.length - pending.length,
				pending: pending.map(pendingOf),
			},
		});
	}

	private install(request: Request): ModelsReply {
		const id = text(request.model_id);
		const spec = (request.model ?? {}) as Request;
		const assets = (
			Array.isArray(spec.assets) ? spec.assets : []
		) as AssetInput[];
		const existing = this.model(id);
		if (existing) return this.reinstall(existing, spec);
		if (!assets.length)
			return refused("invalid", "A model needs one asset at least.");
		const missing = assets.filter((asset) => !this.present(asset.digest));
		const needed = missing.reduce((total, asset) => total + asset.size, 0);
		const left = this.state.store.budget - this.state.store.bytes;
		if (needed > left)
			return refused(
				"limit",
				`The model needs ${needed} bytes but the model disk has ${left} left.`,
			);
		const jobs = missing.map((asset) => this.startJob(asset));
		const model: HostedModel = {
			id,
			display_name: text(spec.display_name) || id,
			kind: spec.kind as HostedModel["kind"],
			engine: spec.engine as HostedModel["engine"],
			assets: assets.map((asset) => asset.digest),
			settings: (request.settings ?? {}) as ModelSettings,
			residency: (request.residency ?? {
				mode: "always_on",
			}) as Residency,
			revision: 1,
			state: jobs.length ? "acquiring" : "stopped",
		};
		this.state.models = [...this.state.models, model];
		this.specs.set(id, canonical(spec));
		return done({
			model,
			assets: {
				total: assets.length,
				present: assets.length - jobs.length,
				pending: jobs.map(pendingOf),
			},
		});
	}

	/**
	 * Like the supervisor: new settings drain and stop a running engine, a model
	 * kept loaded then loads again with them; residency alone restarts nothing,
	 * and "kept off" stops it. The answer is the model right after the stop.
	 */
	private configure(request: Request): ModelsReply {
		const model = this.model(request.model_id);
		if (!model) return this.unknownModel(request.model_id);
		if (request.expected_revision !== model.revision)
			return this.changed(model.id);
		const settings = (request.settings ?? {}) as ModelSettings;
		const residency = (request.residency ?? model.residency) as Residency;
		const restart =
			canonical(settings) !== canonical(model.settings) &&
			model.state === "loaded";
		const keptOff =
			residency.mode === "pinned_off" &&
			(model.state === "loaded" || model.state === "loading");
		const next = {
			...model,
			settings,
			residency,
			revision: model.revision + 1,
		} as HostedModel;
		if (!restart && !keptOff) return done(this.replace(next));
		this.loads.delete(model.id);
		const stopped = this.stop(next);
		if (restart && residency.mode === "always_on") this.startLoad(stopped);
		return done(stopped);
	}

	/** Answers `loading` and loads in the background, like the agent; a model kept off is refused. */
	private load(request: Request): ModelsReply {
		const model = this.model(request.model_id);
		if (!model) return this.unknownModel(request.model_id);
		if (model.residency.mode === "pinned_off")
			return refused(
				"invalid",
				`Model ${model.id} is pinned off; give it another residency to load it`,
			);
		if (model.state === "acquiring")
			return refused(
				"busy",
				`Model ${model.id} is still downloading. Load it once its files are present.`,
			);
		if (model.state === "loaded" || model.state === "loading")
			return done(model);
		return done(this.startLoad(model));
	}

	private startLoad(model: HostedModel): HostedModel {
		this.loads.add(model.id);
		return this.replace({ ...stateless(model), state: "loading" });
	}

	private stop(model: HostedModel): HostedModel {
		return this.replace({ ...stateless(model), state: "stopped" });
	}

	private loaded(model: HostedModel): HostedModel {
		const gpu = this.state.system.gpus.length > 0 && model.engine !== "onnx";
		const slots = model.settings.parallel ?? (gpu ? 4 : 1);
		return this.replace({
			...stateless(model),
			state: "loaded",
			ram_bytes: GIB / 2,
			vram_bytes: gpu ? 4 * GIB : 0,
			slots,
			slots_busy: 0,
		});
	}

	/** Answers `stopped`; like the supervisor, a model kept loaded starts loading again right after. */
	private unload(request: Request): ModelsReply {
		const model = this.model(request.model_id);
		if (!model) return this.unknownModel(request.model_id);
		if (model.state === "acquiring") return done(model);
		this.loads.delete(model.id);
		const stopped = this.stop(model);
		if (model.residency.mode === "always_on") this.startLoad(stopped);
		return done(stopped);
	}

	private remove(request: Request): ModelsReply {
		const model = this.model(request.model_id);
		if (!model) return this.unknownModel(request.model_id);
		if (request.expected_revision !== model.revision)
			return this.changed(model.id);
		this.loads.delete(model.id);
		this.specs.delete(model.id);
		this.state.models = this.state.models.filter((row) => row.id !== model.id);
		const kept = (digest: ModelAssetDigest) =>
			this.state.models.some((row) =>
				row.assets.some((asset) => sameDigest(asset, digest)),
			);
		this.state.jobs = this.state.jobs.filter(
			(job) =>
				!model.assets.some((asset) => sameDigest(asset, job.digest)) ||
				kept(job.digest),
		);
		this.state.recommendations = this.state.recommendations.filter(
			(row) => row.model_id !== model.id,
		);
		return done({ model_id: model.id });
	}

	private ensure(request: Request): ModelsReply {
		const pins = Array.isArray(request.pins) ? request.pins : [];
		if (!pins.length) return refused("invalid", "Model ensure pin count.");
		return done({ total: pins.length, present: pins.length, pending: [] });
	}

	private runtime(
		runtime: unknown,
		backend: unknown,
		installed?: boolean,
	): RuntimeInfo | undefined {
		const row = this.state.runtimes.find(
			(entry) => entry.runtime === runtime && entry.backend === backend,
		);
		if (row && installed !== undefined) row.installed = installed;
		return row;
	}

	private installRuntime(request: Request): ModelsReply {
		const { runtime, backend } = request;
		if (runtime === "mlx" && backend !== "metal")
			return refused("invalid", "MLX runs on Metal only.");
		const row = this.runtime(runtime, backend);
		if (!row)
			return refused(
				"invalid",
				`No ${text(runtime)} runtime for ${text(backend)} is available for this device.`,
			);
		const digest = packDigest(row.runtime, row.backend);
		if (row.installed)
			return done({ runtime: row, asset: { digest, state: "present" } });
		const job = this.startJob({
			digest,
			size: row.size,
			file_name: `${row.runtime}-${row.build}-${row.backend}.tar.gz`,
			sources: [`https://${PACK_HOST}/runtimes/${row.build}`],
		});
		this.packJobs.set(job.job_id, {
			runtime: row.runtime,
			backend: row.backend,
		});
		return done({
			runtime: row,
			asset: {
				digest,
				job_id: job.job_id,
				state: "fetching",
				source_index: 0,
				bytes: 0,
			},
		});
	}

	/** Refused while a loaded model runs from exactly this pack, as the agent's `runtime_in_use`. */
	private removeRuntime(request: Request): ModelsReply {
		const row = this.runtime(request.runtime, request.backend);
		if (!row)
			return refused(
				"invalid",
				`No ${text(request.runtime)} runtime for ${text(request.backend)} on this device.`,
			);
		const user = this.state.models.find(
			(model) =>
				model.state === "loaded" &&
				model.engine === row.runtime &&
				backendOf(model.engine, this.state.runtimes) === row.backend,
		);
		if (user)
			return refused(
				"revision_conflict",
				`Remove runtime ${row.runtime}/${row.backend}: loaded model ${user.id} runs from it; unload it first`,
			);
		row.installed = false;
		return done(row);
	}

	private cancelJob(request: Request): ModelsReply {
		const job = this.state.jobs.find((row) => row.job_id === request.job_id);
		if (!job)
			return refused(
				"revision_conflict",
				`Model job ${text(request.job_id)} is not open on this device.`,
			);
		if (job.state === "present")
			return refused("invalid", "This download already finished.");
		const {
			state: _state,
			bytes: _bytes,
			source_index: _source,
			bytes_per_second: _rate,
			reason: _reason,
			http_status: _status,
			...rest
		} = job as ModelJob & Record<string, unknown>;
		const cancelled: ModelJob = {
			...(rest as ModelJob),
			updated_at: this.now(),
			state: "failed",
			reason: "cancelled",
		};
		this.state.jobs = this.state.jobs.map((row) =>
			row.job_id === job.job_id ? cancelled : row,
		);
		return done(cancelled);
	}
}
