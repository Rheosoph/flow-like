import {
	type SafeScopedCssOptions,
	type SanitizedCSS,
	safeScopedCss,
} from "./css-utils";

interface PendingRequest {
	resolve: (css: SanitizedCSS) => void;
	reject: (error: Error) => void;
}

interface ScopedCssWorkerResponse {
	id: number;
	ok: boolean;
	css?: string;
	error?: string;
}

let worker: Worker | null | undefined;
let requestSequence = 0;
const pendingRequests = new Map<number, PendingRequest>();

function failPendingRequests(reason: string): void {
	for (const request of pendingRequests.values()) {
		request.reject(new Error(reason));
	}
	pendingRequests.clear();
}

function getScopedCssWorker(): Worker | null {
	if (worker !== undefined) return worker;
	if (typeof Worker === "undefined") {
		worker = null;
		return worker;
	}

	try {
		worker = new Worker(new URL("./scoped-css.worker.ts", import.meta.url), {
			type: "module",
		});
		worker.onmessage = (event: MessageEvent<ScopedCssWorkerResponse>) => {
			const { id, ok, css, error } = event.data;
			const request = pendingRequests.get(id);
			if (!request) return;
			pendingRequests.delete(id);

			if (ok && typeof css === "string") {
				request.resolve(css as SanitizedCSS);
			} else {
				request.reject(new Error(error ?? "CSS worker failed"));
			}
		};
		worker.onerror = (event) => {
			console.warn(
				"[scoped-css] Worker crashed; future stylesheets will use the deferred inline fallback.",
				event,
			);
			failPendingRequests("CSS worker crashed");
			worker?.terminate();
			worker = null;
		};
	} catch (error) {
		console.warn(
			"[scoped-css] Failed to start worker; using deferred inline fallback.",
			error,
		);
		worker = null;
	}

	return worker;
}

function scopeCssDeferred(
	css: string,
	scopeSelector: string,
	options: SafeScopedCssOptions,
): Promise<SanitizedCSS> {
	return new Promise((resolve) => {
		setTimeout(() => resolve(safeScopedCss(css, scopeSelector, options)), 0);
	});
}

export async function scopeCssOffThread(
	css: string,
	scopeSelector: string,
	options: SafeScopedCssOptions = {},
): Promise<SanitizedCSS> {
	const activeWorker = getScopedCssWorker();
	if (!activeWorker) {
		return scopeCssDeferred(css, scopeSelector, options);
	}

	try {
		return await new Promise<SanitizedCSS>((resolve, reject) => {
			const id = ++requestSequence;
			pendingRequests.set(id, { resolve, reject });
			try {
				activeWorker.postMessage({ id, css, scopeSelector, options });
			} catch (error) {
				pendingRequests.delete(id);
				reject(error instanceof Error ? error : new Error(String(error)));
			}
		});
	} catch (error) {
		console.warn(
			"[scoped-css] Worker request failed; using deferred inline fallback.",
			error,
		);
		return scopeCssDeferred(css, scopeSelector, options);
	}
}
