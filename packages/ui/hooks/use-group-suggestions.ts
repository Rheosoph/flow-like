"use client";

import type { InternalNode, Node } from "@xyflow/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
	type GroupSuggestion,
	evaluateGroupCandidates,
	findGroupSuggestions,
	groupMembershipKey,
} from "../lib/flow-grouping";
import { buildGroupSuggestionCommand } from "../lib/flow-grouping-command";
import type { IBoard } from "../lib/schema/flow/board";
import type { IGenericCommand } from "../lib/schema/flow/board/commands/generic-command";

interface Options {
	board?: IBoard;
	currentLayer?: string;
	readOnly: boolean;
	selectedNodeIds: readonly string[];
	getNodes: () => Node[];
	getInternalNode: (id: string) => InternalNode | undefined;
	/** The visible flow-space region, searched first on boards too large to search whole. */
	getFocusBounds?: () => GroupSuggestion["bounds"] | undefined;
	executeCommands: (commands: IGenericCommand[]) => Promise<unknown>;
	onStale: () => void;
}

export type GroupReviewScope = "selection" | "layer";

export interface GroupReviewSummary {
	collapsed: number;
	skipped: number;
}

const EMPTY_SUMMARY: GroupReviewSummary = { collapsed: 0, skipped: 0 };

const membership = (suggestion: GroupSuggestion) =>
	groupMembershipKey(suggestion.memberIds);

/** A retired entry hands its slot to the one after it, wrapping to the start. */
function successor<T>(list: readonly T[], index: number): T | undefined {
	return list[index] ?? list[0];
}

export function useGroupSuggestions(options: Options) {
	const latest = useRef(options);
	latest.current = options;
	const [open, setOpen] = useState(false);
	const [snapshot, setSnapshot] = useState<GroupSuggestion[]>([]);
	const [selectedKey, setSelectedKey] = useState<string>();
	const [preview, setPreview] = useState(false);
	const [busy, setBusy] = useState(false);
	const [reviewScope, setReviewScope] = useState<GroupReviewScope>("layer");
	const [summary, setSummary] = useState(EMPTY_SUMMARY);
	const [exhausted, setExhausted] = useState(false);
	const pending = useRef(false);
	const scopedReview = useRef(true);
	const [offer, setOffer] = useState<"waiting" | "ready">();
	const [offerCount, setOfferCount] = useState(0);
	const epoch = useRef(0);
	const skipped = useRef(new Map<string, string[][]>());
	const scope = JSON.stringify([
		options.board?.id,
		options.currentLayer,
		options.readOnly,
	]);
	const previousScope = useRef(scope);

	const skippedInScope = useCallback(() => {
		const { board, currentLayer } = latest.current;
		const key = JSON.stringify([board?.id, currentLayer]);
		let entries = skipped.current.get(key);
		if (!entries) {
			entries = [];
			skipped.current.set(key, entries);
		}
		return entries;
	}, []);

	const detect = useCallback(
		(scoped: boolean) => {
			const { board, currentLayer, selectedNodeIds, getNodes, getFocusBounds } =
				latest.current;
			if (!board) return [];
			const nodeSizes = new Map<string, readonly [number, number]>();
			for (const node of getNodes()) {
				const width = node.measured?.width ?? node.width;
				const height = node.measured?.height ?? node.height;
				if (width && height) nodeSizes.set(node.id, [width, height]);
			}
			return findGroupSuggestions({
				board,
				currentLayer,
				nodeSizes,
				skip: skippedInScope(),
				focus: getFocusBounds?.(),
				only:
					scoped && selectedNodeIds.length > 1
						? new Set(selectedNodeIds)
						: undefined,
			});
		},
		[skippedInScope],
	);

	const close = useCallback(() => {
		epoch.current++;
		setOpen(false);
		setPreview(false);
		setSnapshot([]);
		setSelectedKey(undefined);
	}, []);

	useEffect(() => {
		if (previousScope.current === scope) return;
		previousScope.current = scope;
		close();
		setOffer(undefined);
		setOfferCount(0);
	}, [scope, close]);

	const load = useCallback(
		(scoped: boolean) => {
			if (latest.current.readOnly) return;
			scopedReview.current = scoped;
			const candidates = detect(scoped);
			setSnapshot(candidates);
			setSelectedKey(candidates[0] ? membership(candidates[0]) : undefined);
			setExhausted(candidates.length === 0);
			setReviewScope(
				scoped && latest.current.selectedNodeIds.length > 1
					? "selection"
					: "layer",
			);
			setOpen(true);
			setOffer(undefined);
		},
		[detect],
	);

	const start = useCallback(
		(scoped = true) => {
			if (latest.current.readOnly) return;
			setSummary(EMPTY_SUMMARY);
			setPreview(false);
			load(scoped);
		},
		[load],
	);

	const findMore = useCallback(() => {
		if (pending.current) return;
		load(scopedReview.current);
	}, [load]);

	// Revalidate fixed memberships without replacing them with new suggestions mid-review.
	const suggestions = useMemo(() => {
		if (!options.board || !open) return [];
		return evaluateGroupCandidates(
			{ board: options.board, currentLayer: options.currentLayer },
			snapshot.map((candidate) => candidate.memberIds),
		).filter((current): current is GroupSuggestion => current !== undefined);
	}, [snapshot, options.board, options.currentLayer, open]);
	const suggestionsRef = useRef(suggestions);
	suggestionsRef.current = suggestions;
	const lastIndex = useRef(0);
	const foundIndex = suggestions.findIndex(
		(candidate) => membership(candidate) === selectedKey,
	);
	if (foundIndex >= 0) lastIndex.current = foundIndex;
	const selected =
		suggestions[foundIndex] ?? successor(suggestions, lastIndex.current);
	const currentKey = selected ? membership(selected) : undefined;
	useEffect(() => {
		if (currentKey !== selectedKey) setSelectedKey(currentKey);
	}, [currentKey, selectedKey]);

	// Debounced on the board so it counts the laid-out graph, then settled: edits
	// made while the offer is showing never pay for another full search.
	useEffect(() => {
		if (offer !== "waiting" || open || options.readOnly || !options.board)
			return;
		const timer = setTimeout(() => {
			setOfferCount(detect(false).length);
			setOffer("ready");
		}, 180);
		return () => clearTimeout(timer);
	}, [offer, open, options.readOnly, options.board, detect]);

	// A graph refresh may already have dropped the entry and moved selection on.
	const retire = useCallback((key: string) => {
		const list = suggestionsRef.current;
		const index = list.findIndex((item) => membership(item) === key);
		if (index >= 0) {
			const next = successor(
				list.filter((item) => membership(item) !== key),
				index,
			);
			setSelectedKey(next ? membership(next) : undefined);
		}
		setSnapshot((previous) =>
			previous.filter((item) => membership(item) !== key),
		);
	}, []);

	const select = useCallback(
		(id: string) => {
			const candidate = suggestions.find((item) => item.id === id);
			if (candidate) setSelectedKey(membership(candidate));
		},
		[suggestions],
	);
	const step = useCallback(
		(offset: 1 | -1) => {
			if (pending.current || suggestions.length < 2) return;
			const index = suggestions.findIndex(
				(item) => membership(item) === currentKey,
			);
			const next =
				suggestions[
					(Math.max(0, index) + offset + suggestions.length) %
						suggestions.length
				];
			setSelectedKey(membership(next));
		},
		[suggestions, currentKey],
	);
	const skip = useCallback(() => {
		if (!selected || pending.current) return;
		skippedInScope().push([...selected.memberIds]);
		retire(membership(selected));
		setSummary((previous) => ({ ...previous, skipped: previous.skipped + 1 }));
	}, [selected, retire, skippedInScope]);
	const collapse = useCallback(async () => {
		const { board, currentLayer, readOnly, executeCommands, onStale } =
			latest.current;
		if (!board || readOnly || !selected || pending.current) return;
		const command = buildGroupSuggestionCommand({
			board,
			currentLayer,
			suggestion: selected,
		});
		if (!command) {
			onStale();
			load(scopedReview.current);
			return;
		}
		pending.current = true;
		setBusy(true);
		const reviewEpoch = epoch.current;
		try {
			const result = await executeCommands([command]);
			if (result !== undefined && reviewEpoch === epoch.current) {
				retire(membership(selected));
				setSummary((previous) => ({
					...previous,
					collapsed: previous.collapsed + 1,
				}));
			}
		} catch {
			// The command pipeline reports failures. Keep the suggestion available to retry.
		} finally {
			pending.current = false;
			setBusy(false);
		}
	}, [selected, load, retire]);

	const getPinPosition = useCallback((nodeId: string, pinId: string) => {
		const { getInternalNode, currentLayer } = latest.current;
		const ids =
			nodeId === currentLayer
				? [`${nodeId}-input`, `${nodeId}-return`]
				: [nodeId];
		for (const id of ids) {
			const node = getInternalNode(id);
			if (!node) continue;
			const bounds = node.internals.handleBounds;
			const handle = [
				...(bounds?.source ?? []),
				...(bounds?.target ?? []),
			].find((value) => value.id === pinId);
			if (!handle) continue;
			return {
				x: node.internals.positionAbsolute.x + handle.x + handle.width / 2,
				y: node.internals.positionAbsolute.y + handle.y + handle.height / 2,
			};
		}
	}, []);

	const previewMemberIds = useMemo(
		() => new Set(open && preview ? selected?.memberIds : []),
		[open, preview, selected],
	);
	return {
		open,
		suggestions,
		selectedId: selected?.id,
		preview,
		busy,
		scope: reviewScope,
		summary,
		exhausted,
		offerCount: offer === "ready" && !open ? offerCount : 0,
		previewMemberIds,
		start,
		close,
		select,
		next: useCallback(() => step(1), [step]),
		previous: useCallback(() => step(-1), [step]),
		skip,
		collapse,
		findMore,
		getPinPosition,
		togglePreview: useCallback(() => setPreview((value) => !value), []),
		offerAfterLayout: useCallback(() => {
			setOfferCount(0);
			setOffer("waiting");
		}, []),
		dismissOffer: useCallback(() => setOffer(undefined), []),
	};
}
