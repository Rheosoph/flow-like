"use client";

import { useEffect, useState } from "react";
import type { DiffSegment } from "./types";

export interface HighlightToken {
	content: string;
	color?: string;
}

export interface LinePiece {
	text: string;
	color?: string;
	changed: boolean;
}

const NON_HIGHLIGHT_LANGS = new Set(["", "text", "plaintext", "txt", "plain"]);
const MAX_HIGHLIGHT_CHARS = 200_000;

/**
 * Tokenizes `code` into per-line tokens with the active theme's colors using
 * shiki. Returns null (plain rendering) when highlighting is disabled, the
 * language is plaintext, the input is too large, or the grammar fails to load.
 */
export function useHighlightedLines(
	code: string,
	lang: string | undefined,
	theme: "light" | "dark",
	enabled: boolean,
): HighlightToken[][] | null {
	const [grid, setGrid] = useState<HighlightToken[][] | null>(null);

	useEffect(() => {
		const normalized = (lang ?? "").toLowerCase();
		if (
			!enabled ||
			NON_HIGHLIGHT_LANGS.has(normalized) ||
			code.length > MAX_HIGHLIGHT_CHARS
		) {
			setGrid(null);
			return;
		}

		let cancelled = false;
		import("shiki")
			.then(({ codeToTokens }) =>
				codeToTokens(code, {
					lang: normalized,
					theme: theme === "dark" ? "github-dark" : "github-light",
				}),
			)
			.then((result) => {
				if (cancelled) return;
				setGrid(
					result.tokens.map((line) =>
						line.map((token) => ({
							content: token.content,
							color: token.color,
						})),
					),
				);
			})
			.catch(() => {
				if (!cancelled) setGrid(null);
			});

		return () => {
			cancelled = true;
		};
	}, [code, lang, theme, enabled]);

	return grid;
}

function buildChangedFlags(
	text: string,
	segments: DiffSegment[] | undefined,
): boolean[] | null {
	if (!segments || segments.length === 0) return null;
	const flags = new Array<boolean>(text.length).fill(false);
	let pos = 0;
	for (const segment of segments) {
		const len = segment.text.length;
		if (segment.kind !== "common") {
			for (let k = 0; k < len && pos + k < text.length; k++) {
				flags[pos + k] = true;
			}
		}
		pos += len;
	}
	return pos === text.length ? flags : null;
}

/**
 * Merges shiki syntax tokens with the cell's word-level diff segments so each
 * rendered span carries both its syntax color and an add/remove "changed" flag.
 */
export function buildLinePieces(
	text: string,
	tokens: HighlightToken[] | undefined,
	segments: DiffSegment[] | undefined,
): LinePiece[] {
	const changed = buildChangedFlags(text, segments);
	const tokenLength =
		tokens?.reduce((sum, t) => sum + t.content.length, 0) ?? 0;
	const base: HighlightToken[] =
		tokens && tokenLength === text.length ? tokens : [{ content: text }];

	const pieces: LinePiece[] = [];
	let pos = 0;
	for (const token of base) {
		const { content } = token;
		let local = 0;
		while (local < content.length) {
			const isChanged = changed ? changed[pos + local] : false;
			let end = local + 1;
			while (
				end < content.length &&
				(changed ? changed[pos + end] : false) === isChanged
			) {
				end++;
			}
			pieces.push({
				text: content.slice(local, end),
				color: token.color,
				changed: isChanged,
			});
			local = end;
		}
		pos += content.length;
	}

	const merged: LinePiece[] = [];
	for (const piece of pieces) {
		const last = merged[merged.length - 1];
		if (last && last.color === piece.color && last.changed === piece.changed) {
			last.text += piece.text;
		} else {
			merged.push({ ...piece });
		}
	}
	return merged;
}
