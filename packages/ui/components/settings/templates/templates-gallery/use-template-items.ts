"use client";

import { useQueries } from "@tanstack/react-query";
import { useMemo } from "react";
import { shareBoardSnapshot } from "../../../../lib/board-sync/query";
import type { IBoard } from "../../../../lib/schema/flow/board";
import { useBackend } from "../../../../state/backend-state";
import {
	type ITemplateEntry,
	type ITemplateItem,
	toTemplateItem,
} from "./templates-gallery-model";

type IBoardResult = { data?: IBoard; isError: boolean; isPending: boolean };

const NO_ENTRIES: readonly ITemplateEntry[] = [];

const pickBoards = (results: readonly IBoardResult[]) =>
	results.map(({ data, isError, isPending }) => ({ data, isError, isPending }));

/**
 * Every listed template with its board. The query key matches
 * `useInvoke(templateState.getTemplate, …, [appId, templateId])`, so the
 * template detail page and the editor's template picker reuse what this loads.
 */
export function useTemplateItems(
	entries: readonly ITemplateEntry[] | undefined,
	enabled: boolean,
): ITemplateItem[] {
	const backend = useBackend();
	const list = entries ?? NO_ENTRIES;
	const boards = useQueries({
		queries: list.map(([appId, templateId]) => ({
			queryKey: ["getTemplate", appId, templateId],
			queryFn: () => backend.templateState.getTemplate(appId, templateId),
			structuralSharing: shareBoardSnapshot,
			enabled,
		})),
		combine: pickBoards,
	});

	return useMemo(
		() =>
			list.map((entry, index) => {
				const board = boards[index];
				const state = board?.data
					? "ready"
					: board?.isError
						? "error"
						: "loading";
				return toTemplateItem(entry, board?.data, state);
			}),
		[list, boards],
	);
}
