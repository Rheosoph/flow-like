import type React from "react";

const ESCAPE_PATTERN = /[.*+?^${}()|[\]\\]/g;

/**
 * Wrap every occurrence of the query's terms in `text` with a `<mark>`.
 *
 * The query is split on whitespace and each term matched independently, so results found by the
 * (tokenized, prefix-matching) MiniSearch index still highlight when the user typed the words in a
 * different order than they appear in the title.
 *
 * Matching is done positionally against the split result rather than by re-testing each part with
 * a `/g` regex: `RegExp.test` advances `lastIndex` between calls, which makes the obvious
 * implementation alternate between highlighting and skipping identical parts.
 */
export function highlightMatch(text: string, query: string): React.ReactNode {
	const terms = query
		.trim()
		.split(/\s+/)
		.filter(Boolean)
		.map((term) => term.replace(ESCAPE_PATTERN, "\\$&"));
	if (terms.length === 0) return text;

	const splitter = new RegExp(`(${terms.join("|")})`, "gi");
	const parts = text.split(splitter);
	if (parts.length === 1) return text;

	// String.split with one capture group interleaves: [text, match, text, match, …], so the odd
	// indices are exactly the matches — no re-testing needed.
	return parts.map((part, index) =>
		index % 2 === 1 ? (
			<mark
				// biome-ignore lint/suspicious/noArrayIndexKey: split output is positional, index is the identity
				key={index}
				className="rounded-[3px] bg-primary/20 px-0.5 text-primary"
			>
				{part}
			</mark>
		) : (
			part
		),
	);
}
