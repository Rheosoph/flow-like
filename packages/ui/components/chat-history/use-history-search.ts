"use client";

import { useDebounce } from "@uidotdev/usehooks";
import { useMemo } from "react";
import { useSearch } from "../../hooks/use-search-index";
import type { IMessage } from "../interfaces/chat-default/chat-db";
import type { IHistoryEntry } from "./chat-history-types";

/** Below this the query is treated as empty — one or two characters match nearly everything. */
export const MIN_BODY_SEARCH_LENGTH = 2;

/** Streaming rewrites the same assistant row about once a second; debounce so the index is not. */
const SEARCH_DEBOUNCE_MS = 200;

/** Bounds on the indexed corpus, so a long conversation cannot blow up the in-memory index. */
const MAX_MESSAGES_PER_CONVERSATION = 40;
const MAX_CHARS_PER_CONVERSATION = 2000;

/** Flatten one message's content — it is either a plain string or an array of typed parts. */
function messageText(message: IMessage): string {
	const content = message.inner?.content;
	if (typeof content === "string") return content;
	if (!Array.isArray(content)) return "";
	return content
		.map((part) => (typeof part?.text === "string" ? part.text : ""))
		.filter(Boolean)
		.join(" ");
}

/**
 * Fold a flat list of messages from many conversations into one bounded text blob per
 * conversation, newest message first.
 *
 * Callers fetch those messages with a single indexed `anyOf(ids)` scan — never one query per
 * conversation. On desktop every IndexedDB op runs through the SQLite shim at 12–26× native cost,
 * so an N+1 here would be felt directly.
 */
export function buildSearchCorpus(
	messages: readonly IMessage[] | undefined,
): Map<string, string> {
	const corpus = new Map<string, string>();
	if (!messages) return corpus;

	const bySession = new Map<string, IMessage[]>();
	for (const message of messages) {
		const bucket = bySession.get(message.sessionId);
		if (bucket) bucket.push(message);
		else bySession.set(message.sessionId, [message]);
	}

	for (const [sessionId, bucket] of bySession) {
		const text = bucket
			.sort((a, b) => b.timestamp - a.timestamp)
			.slice(0, MAX_MESSAGES_PER_CONVERSATION)
			.map(messageText)
			.filter(Boolean)
			.join(" · ")
			.slice(0, MAX_CHARS_PER_CONVERSATION);
		corpus.set(sessionId, text);
	}

	return corpus;
}

export interface HistorySearchResult {
	/** Entries matching the query, or all entries when the query is empty. */
	results: IHistoryEntry[];
	/** The query the results actually reflect — use this to drive match highlighting. */
	appliedQuery: string;
	/** True once the query is long enough that message bodies should be loaded. */
	bodySearchEnabled: boolean;
}

/**
 * Rank history entries against a query with MiniSearch (prefix + fuzzy, titles boosted over
 * bodies). An empty query returns the entries untouched and in order.
 */
export function useHistorySearch(
	entries: readonly IHistoryEntry[] | undefined,
	query: string,
): HistorySearchResult {
	const trimmed = query.trim();
	const debounced = useDebounce(trimmed, SEARCH_DEBOUNCE_MS);
	const bodySearchEnabled = trimmed.length >= MIN_BODY_SEARCH_LENGTH;

	// `useSearchIndex` only rebuilds on the item list and field config — an `extract` closure over
	// the corpus would be captured in a ref and silently go stale. So body text has to be part of
	// the items themselves, which the caller does by filling `searchBody`.
	const results = useSearch(entries, debounced, {
		fields: ["title", "searchBody"],
		boost: { title: 4 },
	});

	return useMemo(
		() => ({
			results: entries ? results : [],
			appliedQuery: debounced,
			bodySearchEnabled,
		}),
		[entries, results, debounced, bodySearchEnabled],
	);
}
