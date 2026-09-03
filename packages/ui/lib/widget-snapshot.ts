/**
 * Captures rendered chat-widget DOM subtrees as image data URLs so the model
 * can see the widget state the user is looking at. Same html2canvas-pro
 * pattern as the WidgetBuilder screenshot (proven against a2ui component
 * trees).
 *
 * Captures are cached per instance under the renderer's content signature and
 * pre-captured once a widget settles, so the send path normally pays for a
 * cache lookup instead of a rasterization.
 */

const WIDGET_ATTRIBUTE = "data-chat-widget-instance";
const MAX_EDGE_PX = 1024;
const WEBP_QUALITY = 0.8;
const CACHE_CAPACITY = 16;
const SETTLE_DEBOUNCE_MS = 800;
const IDLE_TIMEOUT_MS = 2_000;

export function widgetSnapshotAttribute(instanceId: string): {
	[WIDGET_ATTRIBUTE]: string;
} {
	return { [WIDGET_ATTRIBUTE]: instanceId };
}

function findWidgetElement(instanceId: string): HTMLElement | null {
	return document.querySelector<HTMLElement>(
		`[${WIDGET_ATTRIBUTE}="${CSS.escape(instanceId)}"]`,
	);
}

function captureScale(element: HTMLElement): number {
	const { width, height } = element.getBoundingClientRect();
	const longEdge = Math.max(width, height);
	return longEdge > MAX_EDGE_PX ? MAX_EDGE_PX / longEdge : 1;
}

async function blobToDataUrl(blob: Blob): Promise<string> {
	const bytes = new Uint8Array(await blob.arrayBuffer());
	const chunks: string[] = [];
	const chunkSize = 0x8000;
	for (let offset = 0; offset < bytes.length; offset += chunkSize) {
		chunks.push(
			String.fromCharCode(...bytes.subarray(offset, offset + chunkSize)),
		);
	}
	return `data:${blob.type};base64,${btoa(chunks.join(""))}`;
}

type SnapshotCanvas = Pick<HTMLCanvasElement, "toBlob" | "toDataURL">;

/** WebP where the engine can encode it, PNG otherwise; always a data URL. */
export function encodeSnapshotCanvas(canvas: SnapshotCanvas): Promise<string> {
	const png = () => canvas.toDataURL("image/png");
	if (typeof canvas.toBlob !== "function") return Promise.resolve(png());
	return new Promise((resolve) => {
		canvas.toBlob(
			(blob) => {
				if (!blob || blob.type !== "image/webp") {
					resolve(png());
					return;
				}
				blobToDataUrl(blob).then(resolve, () => resolve(png()));
			},
			"image/webp",
			WEBP_QUALITY,
		);
	});
}

export async function captureWidgetSnapshot(
	element: HTMLElement,
): Promise<string | null> {
	try {
		const { default: html2canvas } = await import("html2canvas-pro");
		const canvas = await html2canvas(element, {
			backgroundColor: null,
			scale: captureScale(element),
			logging: false,
			useCORS: true,
		});
		return await encodeSnapshotCanvas(canvas);
	} catch (error) {
		console.warn("[WidgetSnapshot] capture failed:", error);
		return null;
	}
}

type Cancel = () => void;
type GetElement = () => HTMLElement | null;

export interface WidgetSnapshotStoreDeps {
	capture: (element: HTMLElement) => Promise<string | null>;
	setTimer: (run: () => void, delayMs: number) => Cancel;
	whenIdle: (run: () => void) => Cancel;
	onVisible: (run: () => void) => Cancel;
	isHidden: () => boolean;
	capacity?: number;
	debounceMs?: number;
}

export interface WidgetSnapshotStore {
	register: (instanceId: string, signature: string) => void;
	unregister: (instanceId: string) => void;
	schedule: (
		instanceId: string,
		signature: string,
		getElement: GetElement,
	) => void;
	snapshot: (
		instanceId: string,
		getElement: GetElement,
	) => Promise<string | null>;
}

interface CachedSnapshot {
	signature: string;
	dataUrl: string;
}

interface InFlightCapture {
	signature: string | undefined;
	promise: Promise<string | null>;
}

interface PendingCapture {
	signature: string;
	getElement: GetElement;
	cancel: Cancel;
}

/**
 * Pure snapshot bookkeeping: an LRU of `instanceId → { signature, dataUrl }`,
 * the renderer-maintained registry of current signatures, and the debounced
 * idle pre-capture pipeline. Every browser touchpoint is injected so the logic
 * runs without a DOM.
 */
export function createSnapshotStore(
	deps: WidgetSnapshotStoreDeps,
): WidgetSnapshotStore {
	const capacity = deps.capacity ?? CACHE_CAPACITY;
	const debounceMs = deps.debounceMs ?? SETTLE_DEBOUNCE_MS;
	const cache = new Map<string, CachedSnapshot>();
	const sources = new Map<string, string>();
	const pending = new Map<string, PendingCapture>();
	const inFlight = new Map<string, InFlightCapture>();

	const remember = (instanceId: string, signature: string, dataUrl: string) => {
		cache.delete(instanceId);
		cache.set(instanceId, { signature, dataUrl });
		for (const key of cache.keys()) {
			if (cache.size <= capacity) break;
			cache.delete(key);
		}
	};

	const cached = (instanceId: string): string | null => {
		const entry = cache.get(instanceId);
		if (!entry || entry.signature !== sources.get(instanceId)) return null;
		cache.delete(instanceId);
		cache.set(instanceId, entry);
		return entry.dataUrl;
	};

	const cancelPending = (instanceId: string) => {
		pending.get(instanceId)?.cancel();
		pending.delete(instanceId);
	};

	const runCapture = (
		instanceId: string,
		signature: string | undefined,
		getElement: GetElement,
	): Promise<string | null> => {
		const element = getElement();
		if (!element?.isConnected) return Promise.resolve(null);
		const flight: InFlightCapture = {
			signature,
			promise: Promise.resolve(null),
		};
		flight.promise = deps
			.capture(element)
			.then(
				(dataUrl) => {
					if (dataUrl && signature !== undefined) {
						remember(instanceId, signature, dataUrl);
					}
					return dataUrl;
				},
				() => null,
			)
			.finally(() => {
				if (inFlight.get(instanceId) === flight) inFlight.delete(instanceId);
			});
		inFlight.set(instanceId, flight);
		return flight.promise;
	};

	const runWhenIdle = (instanceId: string, job: PendingCapture) => {
		job.cancel = deps.whenIdle(() => {
			if (pending.get(instanceId) !== job) return;
			if (deps.isHidden()) {
				job.cancel = deps.onVisible(() => {
					if (pending.get(instanceId) === job) runWhenIdle(instanceId, job);
				});
				return;
			}
			pending.delete(instanceId);
			void runCapture(instanceId, job.signature, job.getElement);
		});
	};

	const register = (instanceId: string, signature: string) => {
		sources.set(instanceId, signature);
		const job = pending.get(instanceId);
		if (job && job.signature !== signature) cancelPending(instanceId);
	};

	const unregister = (instanceId: string) => {
		sources.delete(instanceId);
		cancelPending(instanceId);
	};

	const schedule = (
		instanceId: string,
		signature: string,
		getElement: GetElement,
	) => {
		sources.set(instanceId, signature);
		cancelPending(instanceId);
		if (cache.get(instanceId)?.signature === signature) return;
		if (inFlight.get(instanceId)?.signature === signature) return;
		const job: PendingCapture = {
			signature,
			getElement,
			cancel: () => undefined,
		};
		pending.set(instanceId, job);
		job.cancel = deps.setTimer(() => {
			if (pending.get(instanceId) === job) runWhenIdle(instanceId, job);
		}, debounceMs);
	};

	const snapshot = (
		instanceId: string,
		getElement: GetElement,
	): Promise<string | null> => {
		const hit = cached(instanceId);
		if (hit) return Promise.resolve(hit);
		const signature = sources.get(instanceId);
		const flight = inFlight.get(instanceId);
		if (flight && signature !== undefined && flight.signature === signature) {
			return flight.promise;
		}
		if (pending.get(instanceId)?.signature === signature) {
			cancelPending(instanceId);
		}
		return runCapture(instanceId, signature, getElement);
	};

	return { register, unregister, schedule, snapshot };
}

function setTimer(run: () => void, delayMs: number): Cancel {
	const handle = setTimeout(run, delayMs);
	return () => clearTimeout(handle);
}

function whenIdle(run: () => void): Cancel {
	if (typeof requestIdleCallback !== "function") return setTimer(run, 0);
	const handle = requestIdleCallback(run, { timeout: IDLE_TIMEOUT_MS });
	return () => cancelIdleCallback(handle);
}

function onVisible(run: () => void): Cancel {
	const listener = () => {
		if (document.visibilityState === "hidden") return;
		document.removeEventListener("visibilitychange", listener);
		run();
	};
	document.addEventListener("visibilitychange", listener);
	return () => document.removeEventListener("visibilitychange", listener);
}

const store = createSnapshotStore({
	capture: captureWidgetSnapshot,
	setTimer,
	whenIdle,
	onVisible,
	isHidden: () =>
		typeof document !== "undefined" && document.visibilityState === "hidden",
});

/** Publish the signature of what the renderer currently shows for an instance. */
export function registerWidgetSnapshotSource(
	instanceId: string,
	signature: string,
): void {
	store.register(instanceId, signature);
}

export function unregisterWidgetSnapshotSource(instanceId: string): void {
	store.unregister(instanceId);
}

/**
 * Pre-capture an instance once it has settled: debounced per instance, run
 * when the browser is idle and the document is visible, skipped when the
 * signature is already cached.
 */
export function scheduleWidgetSnapshot(
	instanceId: string,
	signature: string,
	getElement: GetElement,
): void {
	store.schedule(instanceId, signature, getElement);
}

/**
 * Capture snapshots for the given widget instance ids, in order. Cached
 * captures are reused while their signature matches the registered one.
 * Instances that are not currently rendered (or fail to rasterize) are
 * skipped.
 */
export async function captureWidgetSnapshots(
	instanceIds: string[],
): Promise<string[]> {
	const snapshots = await Promise.all(
		instanceIds.map((instanceId) =>
			store.snapshot(instanceId, () => findWidgetElement(instanceId)),
		),
	);
	return snapshots.filter((snapshot): snapshot is string => snapshot !== null);
}
