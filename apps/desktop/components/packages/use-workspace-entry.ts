"use client";

import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import { readManifest } from "./local-projects";
import { findWorkspaceEntry, sameCheckoutPath } from "./mine-model";
import { mineQueryKeys, useMinePackages } from "./use-mine-packages";

const MANIFEST_STALE_MS = 60_000;

export interface WorkspaceEntryInput {
	id?: string | null;
	project?: string | null;
}

/**
 * The Mine entry behind a workspace link, so the workspace header shows the
 * same state and primary action as the Mine card. `project` wins over `id`; a
 * folder that is not in the developer list still resolves from its manifest.
 */
export function useWorkspaceEntry({ id, project }: WorkspaceEntryInput) {
	const mine = useMinePackages({ includeDisabled: true });
	const entry = useMemo(
		() => findWorkspaceEntry(mine.model.entries, { id, project }),
		[mine.model.entries, id, project],
	);
	const checkoutPath = project || entry?.checkouts[0]?.path || null;

	const manifest = useQuery({
		queryKey: mineQueryKeys.manifest(checkoutPath ?? ""),
		queryFn: () => readManifest(checkoutPath ?? ""),
		enabled: !!checkoutPath,
		staleTime: MANIFEST_STALE_MS,
	});

	const packageId = checkoutPath
		? manifest.data?.id?.trim() || entry?.packageId || undefined
		: id || undefined;
	const checkout = checkoutPath
		? entry?.checkouts.find((candidate) =>
				sameCheckoutPath(candidate.path, checkoutPath),
			)
		: undefined;

	return {
		mine,
		entry,
		checkout,
		checkoutPath,
		/** The folder is in the developer project list (Mine shows it). */
		listed: !!checkout,
		manifest: manifest.data ?? null,
		packageId,
		/** The gate waits: which checkout (and so which id) this is is still unknown. */
		pending:
			mine.isLoading ||
			mine.manifestsPending ||
			(!!checkoutPath && manifest.isPending),
	};
}

export type WorkspaceEntry = ReturnType<typeof useWorkspaceEntry>;
