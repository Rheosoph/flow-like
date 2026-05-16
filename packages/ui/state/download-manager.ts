import { type StoreApi, type UseBoundStore, create } from "zustand";
import { Bit, Download, type IBit } from "../lib";
import type { IBackendState } from "./backend-state";

declare global {
	var __FL_DL_MANAGER__: DownloadManager | undefined;
	var __FL_DL_STORE__: UseBoundStore<StoreApi<IDownloadManager>> | undefined;
}

function getManagerSingleton(): DownloadManager {
	globalThis.__FL_DL_MANAGER__ ??= new DownloadManager();
	return globalThis.__FL_DL_MANAGER__!;
}

type ProgressListener = (dl: Download) => void;
export type DownloadCompleteListener = (
	bit: IBit,
	downloadedBits: IBit[],
) => void;

export class DownloadManager {
	private readonly downloads = new Map<string, Download>();
	private readonly listeners = new Map<string, Set<ProgressListener>>();
	private readonly completionListeners = new Set<DownloadCompleteListener>();
	private readonly pending = new Map<string, Download>();
	private readonly lastEmit = new Map<string, number>();
	private readonly timers = new Map<string, number>();
	private readonly minIntervalMs = 200;
	private readonly queued = new Set<string>();
	private readonly inFlight = new Map<string, Promise<IBit[]>>();
	private readonly lastMeasure = new Map<
		string,
		{ at: number; downloaded: number }
	>();
	private beforeUnloadBound?: () => void;

	constructor() {
		if (typeof window !== "undefined") {
			this.beforeUnloadBound = () => this.cleanupAll();
			window.addEventListener("beforeunload", this.beforeUnloadBound, {
				once: true,
			});
		}
	}

	public onComplete(listener: DownloadCompleteListener): () => void {
		this.completionListeners.add(listener);
		return () => {
			this.completionListeners.delete(listener);
		};
	}

	private notifyComplete(bit: IBit, downloadedBits: IBit[]) {
		for (const listener of this.completionListeners) {
			try {
				listener(bit, downloadedBits);
			} catch {}
		}
	}

	public onProgress(hash: string, listener: ProgressListener): () => void {
		const set = this.listeners.get(hash) ?? new Set<ProgressListener>();
		set.add(listener);
		this.listeners.set(hash, set);
		const latest = this.pending.get(hash) ?? this.downloads.get(hash);
		if (latest) {
			try {
				listener(latest);
			} catch {}
		}
		return () => {
			const s = this.listeners.get(hash);
			if (!s) return;
			s.delete(listener);
			if (s.size === 0) this.listeners.delete(hash);
		};
	}

	public isQueued(hash: string): boolean {
		return this.queued.has(hash);
	}

	public getLatestPct(hash: string): number | undefined {
		if (this.queued.has(hash)) return 0;
		const dl = this.pending.get(hash) ?? this.downloads.get(hash);
		return dl ? Math.round(dl.progress() * 100) : undefined;
	}

	private notify(hash: string, dl: Download) {
		const set = this.listeners.get(hash);
		if (!set?.size) return;
		for (const l of set) {
			try {
				l(dl);
			} catch {}
		}
	}

	private scheduleNotify(hash: string) {
		if (this.timers.has(hash)) return;
		const now = Date.now();
		const last = this.lastEmit.get(hash) ?? 0;
		const wait = Math.max(0, this.minIntervalMs - (now - last));
		const id = setTimeout(() => {
			this.timers.delete(hash);
			const latest = this.pending.get(hash);
			if (!latest) return;
			this.lastEmit.set(hash, Date.now());
			this.notify(hash, latest);
		}, wait) as unknown as number;
		this.timers.set(hash, id);
	}

	public async download(
		backend: IBackendState,
		bit: IBit,
		cb?: (dl: Download) => void,
	): Promise<IBit[]> {
		const key = bit.hash;
		const hasDependencies = (bit.dependencies?.length ?? 0) > 0;

		// Virtual / hosted bit with no dependency artifacts -> immediately resolve.
		if ((!bit.download_link || bit.size === 0) && !hasDependencies) {
			// Clear any leftover queued state just in case.
			this.cleanupForKey(key);
			if (cb) {
				try {
					// Use a real Download instance so downstream logic relying on class methods stays intact.
					const virtualDownload = new Download(bit, [bit]);
					// Mark as instantly complete (1/1) so progress() => 1.
					virtualDownload.push({
						hash: bit.hash,
						max: 1,
						downloaded: 1,
						path: bit.file_name ?? bit.hash,
					});
					cb(virtualDownload);
				} catch {}
			}
			// Virtual bits don't need completion notification as they weren't actually downloaded
			return [bit];
		}

		const existing = this.inFlight.get(key);
		if (existing) {
			const off = cb ? this.onProgress(key, cb) : undefined;
			existing.finally(() => off?.());
			return existing;
		}

		const pack = Bit.fromObject(bit);
		pack.setBackend(backend);

		const off = cb ? this.onProgress(key, cb) : undefined;
		this.queued.add(key);

		const wrappedCb = (dl: Download) => {
			if (this.queued.has(key)) this.queued.delete(key);
			this.downloads.set(key, dl);
			this.pending.set(key, dl);

			const now = Date.now();
			const last = this.lastEmit.get(key) ?? 0;
			if (now - last >= this.minIntervalMs) {
				this.lastEmit.set(key, now);
				this.notify(key, dl);
			} else {
				this.scheduleNotify(key);
			}
		};

		const promise = pack
			.download(wrappedCb)
			.then((bits) => {
				this.notifyComplete(bit, bits);
				return bits;
			})
			.finally(() => {
				off?.();
				this.inFlight.delete(key);
				this.cleanupForKey(key);
			});

		this.inFlight.set(key, promise);
		return promise;
	}

	public async getParents() {
		const bits = [];
		for (const [, dl] of this.downloads) bits.push(dl.parent());
		return bits;
	}

	public async getBits() {
		const bits = [];
		for (const [, dl] of this.downloads) bits.push(...dl.bits());
		return bits;
	}

	public async getTotal(filter?: Set<string>) {
		let total = 0;
		for (const [key, dl] of this.downloads) {
			if (filter && !filter.has(key)) continue;
			total += dl.total().max;
		}
		return total;
	}

	public async getDownloaded(filter?: Set<string>) {
		let downloaded = 0;
		for (const [key, dl] of this.downloads) {
			if (filter && !filter.has(key)) continue;
			downloaded += dl.total().downloaded;
		}
		return downloaded;
	}

	public async getProgress(filter?: Set<string>) {
		const downloaded = await this.getDownloaded(filter);
		const total = await this.getTotal(filter);
		return total > 0 ? (downloaded / total) * 100 : 0;
	}

	// NEW: Aggregate instantaneous speed, total downloaded, total size and progress.
	public async getSpeed(filter?: Set<string>): Promise<{
		bytesPerSecond: number;
		total: number; // downloaded bytes
		max: number; // total bytes to download
		progress: number; // 0..100
	}> {
		const now = Date.now();
		let bytesPerSecond = 0;
		let downloaded = 0;
		let max = 0;

		// Union of active keys from pending and downloads
		const keys = new Set<string>([
			...this.pending.keys(),
			...this.downloads.keys(),
		]);

		for (const key of keys) {
			if (filter && !filter.has(key)) continue;
			const dl = this.pending.get(key) ?? this.downloads.get(key);
			if (!dl) continue;

			const totals = dl.total(); // { downloaded, max }
			downloaded += totals.downloaded;
			max += totals.max;

			const last = this.lastMeasure.get(key);
			if (last && now > last.at && totals.downloaded >= last.downloaded) {
				const deltaBytes = totals.downloaded - last.downloaded;
				const deltaMs = now - last.at;
				// bytes per second for this key
				bytesPerSecond += deltaMs > 0 ? (deltaBytes * 1000) / deltaMs : 0;
			}

			// Update baseline for next measurement
			this.lastMeasure.set(key, { at: now, downloaded: totals.downloaded });
		}

		const progress = max > 0 ? (downloaded / max) * 100 : 0;

		return {
			bytesPerSecond,
			total: downloaded,
			max,
			progress,
		};
	}

	private cleanupForKey(key: string) {
		const to = this.timers.get(key);
		if (to != null) clearTimeout(to);
		this.timers.delete(key);
		this.pending.delete(key);
		this.lastEmit.delete(key);
		this.downloads.delete(key);
		this.listeners.delete(key);
		this.queued.delete(key);
		this.lastMeasure.delete(key);
	}

	private cleanupAll() {
		for (const key of Array.from(this.timers.keys())) {
			const to = this.timers.get(key);
			if (to != null) clearTimeout(to);
		}
		this.timers.clear();
		this.pending.clear();
		this.lastEmit.clear();
		this.downloads.clear();
		this.listeners.clear();
		this.queued.clear();
		this.inFlight.clear();
		this.lastMeasure.clear();
		if (typeof window !== "undefined" && this.beforeUnloadBound) {
			window.removeEventListener("beforeunload", this.beforeUnloadBound);
		}
	}
}

interface IDownloadManager {
	manager: DownloadManager;
	backend: IBackendState;
	setDownloadBackend: (backend: IBackendState) => void;
	download: (bit: IBit, cb?: (dl: Download) => void) => Promise<IBit[]>;
	onProgress: (hash: string, cb: (dl: Download) => void) => () => void;
	onComplete: (cb: DownloadCompleteListener) => () => void;
	isQueued: (hash: string) => boolean;
	getLatestPct: (hash: string) => number | undefined;
}

const createStore = () =>
	create<IDownloadManager>((set, get) => ({
		manager: getManagerSingleton(),
		backend: {} as IBackendState,
		setDownloadBackend: (backend: IBackendState) => set({ backend }),
		download: async (bit: IBit, cb?: (dl: Download) => void) => {
			const { manager, backend } = get();
			if (!backend.bitState.downloadBit)
				throw new Error("Backend does not support downloading bits.");
			return await manager.download(backend, bit, cb);
		},
		onProgress: (hash: string, cb: (dl: Download) => void) => {
			const { manager } = get();
			return manager.onProgress(hash, cb);
		},
		onComplete: (cb: DownloadCompleteListener) => {
			const { manager } = get();
			return manager.onComplete(cb);
		},
		isQueued: (hash: string) => {
			const { manager } = get();
			return manager.isQueued(hash);
		},
		getLatestPct: (hash: string) => {
			const { manager } = get();
			return manager.getLatestPct(hash);
		},
	}));

export const useDownloadManager = (globalThis.__FL_DL_STORE__ ??=
	createStore());
