"use client";

import { useEffect, useMemo, useState } from "react";
import {
	type SafeScopedCssOptions,
	type SanitizedCSS,
	safeScopedCss,
} from "./css-utils";
import { scopeCssOffThread } from "./scoped-css-worker-client";

export const LARGE_STYLESHEET_THRESHOLD = 12_000;

const EMPTY_SANITIZED_CSS = "" as SanitizedCSS;

interface WorkerResult {
	inputCss: string;
	scopeSelector: string;
	scopeRoot: boolean;
	css: SanitizedCSS;
}

export interface ScopedCssState {
	readonly css: SanitizedCSS;
	/**
	 * The current input is still being scoped off-thread. `css` holds the previous
	 * finished stylesheet meanwhile, or nothing before the first one.
	 */
	readonly pending: boolean;
}

/**
 * Scope and sanitize custom CSS without blocking rendering for large inputs.
 * Small stylesheets stay synchronous to avoid worker round-trip latency.
 */
export function useScopedCss(
	css: string | null | undefined,
	scopeSelector: string,
	options: SafeScopedCssOptions = {},
): ScopedCssState {
	const inputCss = typeof css === "string" ? css : "";
	const scopeRoot = options.scopeRoot === true;
	const useWorker = inputCss.length >= LARGE_STYLESHEET_THRESHOLD;
	const inlineCss = useMemo(
		() =>
			useWorker
				? EMPTY_SANITIZED_CSS
				: safeScopedCss(inputCss, scopeSelector, { scopeRoot }),
		[inputCss, scopeSelector, scopeRoot, useWorker],
	);
	const [workerResult, setWorkerResult] = useState<WorkerResult>();

	useEffect(() => {
		if (!useWorker || !inputCss) return;

		let cancelled = false;
		const settle = (scopedCss: SanitizedCSS) => {
			if (cancelled) return;
			setWorkerResult({ inputCss, scopeSelector, scopeRoot, css: scopedCss });
		};
		void scopeCssOffThread(inputCss, scopeSelector, { scopeRoot }).then(
			settle,
			(error) => {
				if (cancelled) return;
				console.warn("[scoped-css] Failed to process stylesheet:", error);
				// A surface holding its first paint for this sheet must not wait forever.
				settle(EMPTY_SANITIZED_CSS);
			},
		);

		return () => {
			cancelled = true;
		};
	}, [inputCss, scopeSelector, scopeRoot, useWorker]);

	if (!useWorker) return { css: inlineCss, pending: false };
	const settled =
		workerResult?.inputCss === inputCss &&
		workerResult.scopeSelector === scopeSelector &&
		workerResult.scopeRoot === scopeRoot;
	return {
		css: workerResult?.css ?? EMPTY_SANITIZED_CSS,
		pending: !settled,
	};
}
