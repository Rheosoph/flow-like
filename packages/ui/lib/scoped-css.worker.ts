import { type SafeScopedCssOptions, safeScopedCss } from "./css-utils";

interface ScopedCssWorkerRequest {
	id: number;
	css: string;
	scopeSelector: string;
	options?: SafeScopedCssOptions;
}

self.onmessage = (event: MessageEvent<ScopedCssWorkerRequest>) => {
	const { id, css, scopeSelector, options } = event.data;
	try {
		self.postMessage({
			id,
			ok: true,
			css: safeScopedCss(css, scopeSelector, options),
		});
	} catch (error) {
		self.postMessage({ id, ok: false, error: String(error) });
	}
};
