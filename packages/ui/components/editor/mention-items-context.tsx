"use client";

import { createContext, useContext } from "react";

export interface MentionItem {
	readonly key: string;
	readonly text: string;
	/**
	 * Optional custom selection handler. When provided, this runs instead of
	 * the default behavior of inserting a `mention` element with `value: text`.
	 * Use it to insert a richer element (media, link, …) when the user picks
	 * the item from the combobox.
	 */
	readonly onSelect?: (editor: unknown, search: string) => void;
}

const MentionItemsContext = createContext<ReadonlyArray<MentionItem>>([]);

export const MentionItemsProvider = MentionItemsContext.Provider;

export function useMentionItems(): ReadonlyArray<MentionItem> {
	return useContext(MentionItemsContext);
}
