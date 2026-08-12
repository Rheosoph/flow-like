import { get, set } from "idb-keyval";
import type { IBackendState } from "../../state/backend-state";
import type { Nullable } from "../schema/auto-import";
import { type IBit, IBitTypes, type IMetadata } from "../schema/bit/bit";
import type { IEmbeddingModelParameters } from "../schema/bit/bit/embedding-model-parameters";
import type { IImageEmbeddingModelParameters } from "../schema/bit/bit/image-embedding-model-parameters";
import type { ILlmParameters } from "../schema/bit/bit/llm-parameters";
import type { IVlmParameters } from "../schema/bit/bit/vlm-parameters";
import { BitPack } from "./bit-pack";

export interface IDownloadProgress {
	hash: string;
	max: number;
	downloaded: number;
	path: string;
}

export class Download {
	private readonly _progress = new Map<string, IDownloadProgress>();
	private readonly _bits: Map<string, IBit>;
	private readonly _parent: IBit;
	speed = {
		lastMeasured: 0,
		lastPoints: new Map<string, number>(),
	};

	constructor(parent: IBit, bits: IBit[]) {
		this._parent = parent;
		const map = new Map();
		bits.forEach((bit) => {
			map.set(bit.hash, bit);
		});
		this._bits = map;
	}

	push(progress: IDownloadProgress) {
		this._progress.set(progress.hash, progress);
	}

	parent() {
		return this._parent;
	}

	bits() {
		return this._bits;
	}

	total() {
		const max = Array.from(this._progress.values()).reduce(
			(acc, progress) => acc + progress.max,
			0,
		);
		const downloaded = Array.from(this._progress.values()).reduce(
			(acc, progress) => acc + progress.downloaded,
			0,
		);
		return { max, downloaded };
	}

	files() {
		return this._progress;
	}

	measureSpeed() {
		const now = Date.now();
		const last = this.speed.lastMeasured;
		const time = now - last;

		const { max, downloaded } = this.total();
		const lastDownloaded = Array.from(this.speed.lastPoints.values()).reduce(
			(acc, val) => acc + val,
			0,
		);

		const bytesPerSecond =
			time > 0 ? ((downloaded - lastDownloaded) / time) * 1000 : 0;

		this.speed.lastMeasured = now;
		this.speed.lastPoints.clear();

		this._progress.forEach((progress) => {
			this.speed.lastPoints.set(progress.hash, progress.downloaded);
		});

		return {
			bytesPerSecond,
			total: downloaded,
			progress: (downloaded / max) * 100,
			max,
		};
	}

	progress() {
		const { max, downloaded } = this.total();
		if (max === 0) return 0;
		return downloaded / max;
	}

	clear() {
		this._progress.clear();
	}
}

export class Bit implements IBit {
	authors: string[] = [];
	created = "";
	dependencies: Array<string> = [];
	dependency_tree_hash = "";
	download_link?: Nullable<string>;
	file_name?: Nullable<string>;
	hash = "";
	hub = "";
	icon = "";
	id = "";
	license = "";
	meta: { [key: string]: IMetadata } = {};
	parameters:
		| IImageEmbeddingModelParameters
		| IEmbeddingModelParameters
		| ILlmParameters
		| IVlmParameters
		| object = {};
	repository?: Nullable<string>;
	size?: Nullable<number>;
	type: IBitTypes = IBitTypes.Other;
	updated = "";
	version = "";
	backend: IBackendState | undefined;

	public setBackend(backend?: IBackendState) {
		this.backend = backend;
		return this;
	}

	public toJSON(): string {
		const object = this.toObject();
		return JSON.stringify(object);
	}

	public static fromJson(json: string): Bit {
		const object = JSON.parse(json);
		return Bit.fromObject(object);
	}

	public toObject(): IBit {
		const obj: Record<string, any> = {};
		Object.keys(this).forEach((key) => {
			if (key !== "backend" && typeof (this as any)[key] !== "function") {
				obj[key] = (this as any)[key];
			}
		});
		return obj as IBit;
	}

	public static fromObject(obj: IBit): Bit {
		const bit = new Bit();

		for (const key of Object.keys(obj)) {
			(bit as any)[key] = obj[key];
		}

		return bit;
	}

	public async fetchDependencies(): Promise<BitPack> {
		// An empty hash is not a usable cache key — every bit lacking one would
		// otherwise share a single entry and read back foreign dependencies.
		const cacheKey = this.dependency_tree_hash || undefined;
		const cachedDependencies: unknown = cacheKey
			? await get(cacheKey)
			: undefined;

		if (Array.isArray(cachedDependencies) && cachedDependencies.length > 0) {
			const pack = new BitPack();
			pack.bits = cachedDependencies as IBit[];

			console.groupCollapsed("Cached Dependencies for", this.hash);
			console.dir(pack, { depth: null });
			console.groupEnd();

			return pack;
		}

		if (!this.backend) {
			throw new Error("Backend is not set");
		}

		// Backends disagree on the shape: Tauri returns a `BitPack` envelope,
		// the hub API returns a bare `Bit[]`.
		const response: undefined | IBit[] | { bits?: IBit[] } =
			await this.backend?.bitState?.getPackFromBit(this.toObject());
		const bits = Array.isArray(response) ? response : response?.bits;

		if (!Array.isArray(bits)) {
			throw new Error(`No dependencies found for bit ${this.id || this.hash}`);
		}

		if (cacheKey && bits.length > 0) await set(cacheKey, bits);
		const pack = new BitPack();
		pack.bits = bits;

		console.groupCollapsed("Fetched Dependencies for", this.hash);
		console.dir(pack, { depth: null });
		console.groupEnd();

		return pack;
	}

	public async fetchSize(): Promise<number> {
		if (this.backend?.bitState?.getBitSize) {
			return this.backend.bitState.getBitSize(this.toObject());
		}

		const pack = await this.fetchDependencies();
		const bitsConsidered = new Set<string>([this.hash]);
		return pack.bits.reduce((acc, bit) => {
			if (bitsConsidered.has(bit.hash)) return acc;
			bitsConsidered.add(bit.hash);
			return acc + (bit.size ?? 0);
		}, this.size ?? 0);
	}

	async download(cb?: (progress: Download) => void): Promise<IBit[]> {
		try {
			const dependencies = await this.fetchDependencies();
			console.groupCollapsed("Dependencies for", this.hash);
			console.dir(dependencies, { depth: null });
			console.groupEnd();
			const totalProgress = new Download(this.toObject(), dependencies.bits);

			const download: undefined | IBit[] =
				await this.backend?.bitState?.downloadBit(
					this.toObject(),
					dependencies.toObject(),
					(progress) => {
						const lastElement = progress.pop();
						if (lastElement) totalProgress.push(lastElement);
						if (cb) cb(totalProgress);
					},
				);

			if (!download) {
				throw new Error("No dependencies found");
			}

			return download;
		} catch (error) {
			console.error(error);
			throw error;
		}
	}
}
