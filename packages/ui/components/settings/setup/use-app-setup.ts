"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { useInvoke } from "../../../hooks";
import { asArray } from "../../../lib/response-shape";
import {
	isRuntimeConfigured,
	isRuntimeVariableConfigured,
	isSharedConfigurable,
	seedRuntimeVariable,
} from "../../../lib/runtime-vars-utils";
import type { IBoardVariables } from "../../../lib/schema/flow/board-summary";
import type { IVariable } from "../../../lib/schema/flow/variable";
import { useBackend } from "../../../state/backend-state";
import {
	type StoredRuntimeVariable,
	useRuntimeVariables,
} from "../../../state/runtime-variables-context";

/** A variable the user supplies themselves, paired with the flow that declares it. */
export interface DeviceRequirement {
	variable: IVariable;
	boardId: string;
	boardName: string;
	refs?: Record<string, string>;
	/** The board default merged with this device's stored value, if any. */
	seeded: IVariable;
	stored?: StoredRuntimeVariable;
	satisfied: boolean;
	/** Marked by the user as supplied by the app, so it stops blocking. */
	waived: boolean;
	/** Also an app-wide default, so the two lanes both carry this variable. */
	alsoShared: boolean;
}

/** A flow's app-wide parameters, including the ones its author locked. */
export interface SharedGroup {
	boardId: string;
	boardName: string;
	refs?: Record<string, string>;
	variables: IVariable[];
	/** Exposed but not editable — visible so they are not invisible, never writable. */
	locked: IVariable[];
	/** Ids that this device also overrides, so the row can say so. */
	overriddenIds: Set<string>;
}

export type SetupVerdict = "loading" | "denied" | "blocked" | "ready" | "empty";

const WAIVER_PREFIX = "flow-like:setup-waived:";

/**
 * Which unverifiable secrets this user has said the app already provides.
 *
 * Deliberately `localStorage` and not a new field on the stored value: the
 * device store is at version 1 with no upgrade function, and adding a field
 * without one costs every user every value they have configured.
 */
function readWaivers(appId: string): Set<string> {
	if (typeof window === "undefined" || !appId) return new Set();
	try {
		const raw = window.localStorage.getItem(WAIVER_PREFIX + appId);
		if (!raw) return new Set();
		const parsed = JSON.parse(raw);
		return Array.isArray(parsed) ? new Set(parsed as string[]) : new Set();
	} catch {
		return new Set();
	}
}

function writeWaivers(appId: string, ids: Set<string>) {
	if (typeof window === "undefined" || !appId) return;
	try {
		window.localStorage.setItem(
			WAIVER_PREFIX + appId,
			JSON.stringify([...ids]),
		);
	} catch {
		/* a browser that refuses storage still gets a working, if forgetful, page */
	}
}

/**
 * Everything the Setup surface renders, derived from one board read and the
 * device store.
 *
 * The two lanes are kept apart here rather than in the components: which lane a
 * variable belongs to is a property of its flags, so the surface never has to
 * decide, and no component is ever handed a save target it could route wrongly.
 */
export function useAppSetup(appId: string) {
	const backend = useBackend();
	const runtimeVars = useRuntimeVariables();

	const boards = useInvoke(
		backend.boardState.getBoardVariables,
		backend.boardState,
		[appId],
		appId.length > 0,
	);

	const [stored, setStored] = useState<StoredRuntimeVariable[]>([]);
	const [waivers, setWaivers] = useState<Set<string>>(() => new Set());

	useEffect(() => {
		setWaivers(readWaivers(appId));
	}, [appId]);

	useEffect(() => {
		if (!runtimeVars || !appId) return;
		return runtimeVars.subscribe(appId, setStored);
	}, [runtimeVars, appId]);

	const derived = useMemo(
		() => deriveSetup(boards.data, stored, waivers),
		[boards.data, stored, waivers],
	);

	const verdict: SetupVerdict = boards.isLoading
		? "loading"
		: boards.isError
			? "denied"
			: derived.requirements.length === 0 && derived.sharedGroups.length === 0
				? "empty"
				: derived.blocking.length > 0
					? "blocked"
					: "ready";

	const waive = useCallback(
		(variableId: string, waived: boolean) => {
			setWaivers((prev) => {
				const next = new Set(prev);
				if (waived) next.add(variableId);
				else next.delete(variableId);
				writeWaivers(appId, next);
				return next;
			});
		},
		[appId],
	);

	return { boards, stored, verdict, waive, ...derived };
}

/**
 * Turn one board read and the device store into everything the surface shows.
 *
 * Pure on purpose: the counts here are the ones that were wrong before — a
 * progress meter fed by raw row counts, orphaned rows inflating the
 * denominator — so they are worth testing without a DOM.
 */
export function deriveSetup(
	boards: IBoardVariables[] | undefined,
	stored: StoredRuntimeVariable[],
	waivers: Set<string>,
) {
	const storedById = new Map<string, StoredRuntimeVariable>();
	for (const row of stored) storedById.set(row.variableId, row);

	const boardList = asArray(boards);
	const requirements: DeviceRequirement[] = [];
	for (const board of boardList) {
		for (const variable of Object.values(board.variables ?? {})) {
			if (!isRuntimeConfigured(variable)) continue;
			const row = storedById.get(variable.id);
			const seeded = seedRuntimeVariable(variable, row?.value);
			requirements.push({
				variable,
				boardId: board.board_id,
				boardName: board.board_name,
				refs: board.refs,
				seeded,
				stored: row,
				satisfied: isRuntimeVariableConfigured(seeded, board.refs),
				waived: waivers.has(variable.id),
				alsoShared: isSharedConfigurable(variable),
			});
		}
	}
	requirements.sort((a, b) => a.variable.name.localeCompare(b.variable.name));

	const overriddenIds = new Set(
		requirements.filter((r) => r.stored).map((r) => r.variable.id),
	);

	const sharedGroups: SharedGroup[] = boardList
		.map((board) => {
			const all = Object.values(board.variables ?? {});
			return {
				boardId: board.board_id,
				boardName: board.board_name,
				refs: board.refs,
				variables: all
					.filter(isSharedConfigurable)
					.sort((a, b) => a.name.localeCompare(b.name)),
				locked: all
					.filter((v) => v.exposed && !v.editable)
					.sort((a, b) => a.name.localeCompare(b.name)),
				overriddenIds,
			};
		})
		.filter((group) => group.variables.length + group.locked.length > 0)
		.sort((a, b) => a.boardName.localeCompare(b.boardName));

	// Stored values whose variable the app no longer declares. Excluded from
	// every count — counting them is what produced "7 of 4 configured" — and
	// offered for removal, because a forgotten row is a secret that stays on the
	// device forever.
	const known = new Set<string>();
	for (const board of boardList) {
		for (const id of Object.keys(board.variables ?? {})) known.add(id);
	}
	// A read that returned no list must not mark every stored value an orphan.
	const orphans = Array.isArray(boards)
		? stored.filter((row) => !known.has(row.variableId))
		: [];

	const blocking = requirements.filter((r) => !r.satisfied && !r.waived);
	const counted = requirements.filter((r) => !r.waived);
	const satisfiedCount = counted.filter((r) => r.satisfied).length;

	return {
		requirements,
		sharedGroups,
		orphans,
		blocking,
		satisfiedCount,
		requiredCount: counted.length,
		sharedCount: sharedGroups.reduce(
			(sum, group) => sum + group.variables.length,
			0,
		),
		// Clamped: the progress primitive renders a positive translate for values
		// over 100, so an unclamped ratio walks the bar off its own track.
		progress:
			counted.length === 0
				? 100
				: Math.min(100, Math.round((satisfiedCount / counted.length) * 100)),
	};
}
