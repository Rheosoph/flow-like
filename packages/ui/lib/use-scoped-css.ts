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

/**
 * Scope and sanitize custom CSS without blocking rendering for large inputs.
 * Small stylesheets stay synchronous to avoid worker round-trip latency.
 */
export function useScopedCss(
	css: string | null | undefined,
	scopeSelector: string,
	options: SafeScopedCssOptions = {},
): SanitizedCSS {
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
		void scopeCssOffThread(inputCss, scopeSelector, { scopeRoot }).then(
			(scopedCss) => {
				if (cancelled) return;
				setWorkerResult({
					inputCss,
					scopeSelector,
					scopeRoot,
					css: scopedCss,
				});
			},
			(error) => {
				if (!cancelled) {
					console.warn("[scoped-css] Failed to process stylesheet:", error);
				}
			},
		);

		return () => {
			cancelled = true;
		};
	}, [inputCss, scopeSelector, scopeRoot, useWorker]);

	if (!useWorker) return inlineCss;
	if (
		workerResult?.inputCss === inputCss &&
		workerResult.scopeSelector === scopeSelector &&
		workerResult.scopeRoot === scopeRoot
	) {
		return workerResult.css;
	}
	return EMPTY_SANITIZED_CSS;
}
