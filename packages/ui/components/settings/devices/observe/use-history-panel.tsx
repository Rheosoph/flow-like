"use client";

import { type ReactNode, useMemo, useState } from "react";
import { HistoryReadersSheet, type ReadersEdit } from "./history-readers-sheet";
import {
	DEVICE_SCOPE,
	type HistoryRead,
	useHistoryStreams,
} from "./use-history";
import type { ObserveTarget } from "./use-observe-target";

/** Each listed service costs two reads per cycle; further services set their history on their own page. */
export const MAX_HISTORY_SERVICES = 12;

export interface HistoryPanel {
	history: HistoryRead;
	/** The page's own scope first, then the listed services. */
	scopes: readonly string[];
	/** Services of the device that aren't listed. */
	hiddenServices: number;
	onEdit(edit: ReadersEdit): void;
	/** The readers sheet while it is open; render it once per view. */
	sheet: ReactNode;
}

/**
 * The readers lists a view shows and the one sheet that edits them. With
 * `withServices` the owner's device page also lists each service's history.
 */
export function useHistoryPanel(
	target: ObserveTarget,
	withServices: boolean,
): HistoryPanel {
	const own = target.serviceId ?? DEVICE_SCOPE;
	const services =
		withServices && target.owner && target.serviceId === null
			? (target.services ?? []).map((service) => service.serviceId)
			: [];
	const listed = services.slice(0, MAX_HISTORY_SERVICES).join("|");
	const scopes = useMemo(
		() => [own, ...(listed ? listed.split("|") : [])],
		[own, listed],
	);
	const history = useHistoryStreams(target, scopes);
	const [edit, setEdit] = useState<ReadersEdit | null>(null);
	return {
		history,
		scopes,
		hiddenServices: Math.max(0, services.length - MAX_HISTORY_SERVICES),
		onEdit: setEdit,
		sheet: edit ? (
			<HistoryReadersSheet
				key={`${edit.scope}|${edit.kinds.join("+")}|${edit.mode}`}
				target={target}
				edit={edit}
				onClose={() => setEdit(null)}
			/>
		) : null,
	};
}
