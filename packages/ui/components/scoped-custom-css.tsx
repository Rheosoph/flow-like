"use client";

import type { SafeScopedCssOptions } from "../lib/css-utils";
import { createSanitizedStyleProps } from "../lib/css-utils";
import { useScopedCss } from "../lib/use-scoped-css";

export interface ScopedCustomCssProps {
	css: string | null | undefined;
	scopeSelector: string;
	options?: SafeScopedCssOptions;
}

export function ScopedCustomCss({
	css,
	scopeSelector,
	options,
}: ScopedCustomCssProps) {
	const sanitizedCss = useScopedCss(css, scopeSelector, options);
	if (!sanitizedCss) return null;
	return <style {...createSanitizedStyleProps(sanitizedCss)} />;
}
