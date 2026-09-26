"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import type {
	ExplorePlacement,
	ExplorePlacementInput,
} from "../../../store/explore/explore-types";
import {
	type PlacementIssue,
	errorMessage,
	placementInput,
	sameInput,
	validatePlacement,
} from "./explore-admin-model";
import { isDraftConflict } from "./use-admin-explore";

export const AUTOSAVE_DELAY_MS = 600;

export type DraftSaveState =
	| "idle"
	| "pending"
	| "saving"
	| "saved"
	| "invalid"
	| "error"
	| "conflict";

export type DraftFailure =
	| { reason: "invalid" }
	| { reason: "conflict" }
	| { reason: "error"; message: string };

interface Draft {
	id: string;
	/** Bumped by every reset, so a save that finishes after the admin left this draft is told apart from it. */
	generation: number;
	input: ExplorePlacementInput;
}

/**
 * The inspector's local copy of the selected placement. Edits are saved with a debounced PUT; the local copy
 * stays the source of truth while it has unsaved edits, so a conflict or a failed save never loses them.
 * Edits that fail after the admin moved on are reported through `onBackgroundFailure`.
 */
export function usePlacementDraft({
	placement,
	collectionIds,
	save,
	fallbackError,
	onBackgroundFailure,
}: {
	placement: ExplorePlacement | undefined;
	collectionIds: ReadonlySet<string>;
	/** Resolves with the server's copy of the saved placement, when the response carries it. */
	save: (
		id: string,
		input: ExplorePlacementInput,
	) => Promise<ExplorePlacementInput | undefined>;
	fallbackError: string;
	onBackgroundFailure: (name: string, failure: DraftFailure) => void;
}) {
	const [draft, setDraft] = useState<Draft | null>(null);
	const [saveState, setSaveState] = useState<DraftSaveState>("idle");
	const [serverError, setServerError] = useState<string | null>(null);
	const draftRef = useRef<Draft | null>(null);
	const generation = useRef(0);
	const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
	const inflight = useRef<{
		generation: number;
		request: Promise<boolean>;
	} | null>(null);
	const failure = useRef<DraftFailure | null>(null);
	const version = useRef(0);
	const acknowledged = useRef(0);
	const lastSent = useRef<ExplorePlacementInput | null>(null);
	const paused = useRef(false);
	const mounted = useRef(true);
	const saveRef = useRef(save);
	saveRef.current = save;
	const collectionRef = useRef(collectionIds);
	collectionRef.current = collectionIds;
	const fallbackRef = useRef(fallbackError);
	fallbackRef.current = fallbackError;
	const failureRef = useRef(onBackgroundFailure);
	failureRef.current = onBackgroundFailure;

	const replace = useCallback((next: Draft | null) => {
		draftRef.current = next;
		setDraft(next);
	}, []);

	const fail = useCallback(
		(target: Draft, reason: DraftFailure, current: boolean) => {
			if (!current) {
				failureRef.current(target.input.name, reason);
				return;
			}
			failure.current = reason;
			if (reason.reason === "conflict") {
				paused.current = true;
				setSaveState("conflict");
				return;
			}
			if (reason.reason === "error") setServerError(reason.message);
			setSaveState(reason.reason);
		},
		[],
	);

	/** Resolves true once the edits are stored, false when they stay unsaved. */
	const send = useCallback(
		(target: Draft): Promise<boolean> => {
			clearTimeout(timer.current);
			timer.current = undefined;
			const isCurrent = () =>
				mounted.current && draftRef.current?.generation === target.generation;
			if (validatePlacement(target.input, collectionRef.current).length) {
				fail(target, { reason: "invalid" }, isCurrent());
				return Promise.resolve(false);
			}
			const sentVersion = version.current;
			if (isCurrent()) setSaveState("saving");
			const request = (async () => {
				try {
					const echo = await saveRef.current(target.id, target.input);
					if (!isCurrent()) return true;
					lastSent.current = echo ?? target.input;
					acknowledged.current = Math.max(acknowledged.current, sentVersion);
					failure.current = null;
					setServerError(null);
					setSaveState(version.current === sentVersion ? "saved" : "pending");
					return true;
				} catch (error) {
					fail(
						target,
						isDraftConflict(error)
							? { reason: "conflict" }
							: {
									reason: "error",
									message: errorMessage(error, fallbackRef.current),
								},
						isCurrent(),
					);
					return false;
				}
			})();
			inflight.current = { generation: target.generation, request };
			void request.finally(() => {
				if (inflight.current?.request === request) inflight.current = null;
			});
			return request;
		},
		[fail],
	);

	/** Saves pending edits now. Resolves false while edits stay unsaved (invalid, failed or in conflict). */
	const flush = useCallback((): Promise<boolean> => {
		const current = draftRef.current;
		if (!current) return Promise.resolve(true);
		if (paused.current) return Promise.resolve(false);
		if (timer.current !== undefined) return send(current);
		if (inflight.current?.generation === current.generation) {
			return inflight.current.request;
		}
		return Promise.resolve(failure.current === null);
	}, [send]);

	const reset = useCallback(
		(source: ExplorePlacement | undefined) => {
			clearTimeout(timer.current);
			timer.current = undefined;
			paused.current = false;
			failure.current = null;
			generation.current++;
			version.current++;
			acknowledged.current = version.current;
			const input = source ? placementInput(source) : null;
			lastSent.current = input;
			replace(
				source && input
					? { id: source.id, generation: generation.current, input }
					: null,
			);
			setServerError(null);
			setSaveState("idle");
		},
		[replace],
	);

	const placementId = placement?.id;
	// biome-ignore lint/correctness/useExhaustiveDependencies: Only a new selection re-seeds the draft; content changes are merged below.
	useEffect(() => {
		const previous = draftRef.current;
		if (previous?.id === placementId) return;
		const pending = timer.current !== undefined && !paused.current;
		const lost = failure.current;
		const saving = inflight.current?.generation === previous?.generation;
		reset(placement);
		if (!previous) return;
		if (pending) void send(previous);
		else if (lost && !saving) failureRef.current(previous.input.name, lost);
	}, [placementId]);

	useEffect(() => {
		const current = draftRef.current;
		if (!placement || !current || current.id !== placement.id) return;
		if (paused.current) return;
		const server = placementInput(placement);
		if (lastSent.current && sameInput(lastSent.current, server)) return;
		if (version.current !== acknowledged.current) {
			clearTimeout(timer.current);
			timer.current = undefined;
			fail(current, { reason: "conflict" }, true);
			return;
		}
		if (sameInput(current.input, server)) return;
		lastSent.current = server;
		replace({ ...current, input: server });
	}, [placement, replace, fail]);

	useEffect(() => {
		mounted.current = true;
		return () => {
			mounted.current = false;
			const current = draftRef.current;
			if (current && timer.current !== undefined && !paused.current) {
				void send(current);
			}
		};
	}, [send]);

	const edit = useCallback(
		(update: (input: ExplorePlacementInput) => ExplorePlacementInput) => {
			const current = draftRef.current;
			if (!current) return;
			const next = { ...current, input: update(current.input) };
			version.current++;
			replace(next);
			if (paused.current) return;
			setSaveState("pending");
			clearTimeout(timer.current);
			timer.current = setTimeout(() => {
				const latest = draftRef.current;
				if (latest && latest.generation === next.generation) void send(latest);
			}, AUTOSAVE_DELAY_MS);
		},
		[replace, send],
	);

	/** After a conflict: send the local edits again on top of the other admin's draft. */
	const reapply = useCallback(() => {
		paused.current = false;
		failure.current = null;
		const current = draftRef.current;
		if (current) void send(current);
	}, [send]);

	const issues: PlacementIssue[] = draft
		? validatePlacement(draft.input, collectionIds)
		: [];
	const unsaved =
		saveState === "pending" ||
		saveState === "saving" ||
		saveState === "conflict" ||
		saveState === "invalid" ||
		saveState === "error";

	return {
		draft,
		issues,
		saveState,
		serverError,
		unsaved,
		/** Edits that switching away would drop: they failed, conflict, or cannot pass validation. */
		cannotSave:
			saveState === "conflict" ||
			saveState === "invalid" ||
			saveState === "error" ||
			(saveState === "pending" && issues.length > 0),
		edit,
		flush,
		reapply,
		reset,
	};
}
