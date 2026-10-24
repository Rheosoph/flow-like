"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { IBoardDiff } from "../../../lib/board-diff";
import {
	REMOTE_BOARD_APPLIED_EVENT,
	type RemoteBoardAppliedDetail,
	readRemoteBoardApplied,
} from "../../../lib/flow-history";
import type { IBoard } from "../../../lib/schema/flow/board";
import {
	type BoardLastSeenService,
	type LastSeenIds,
	boardLastSeenService,
} from "./board-last-seen-service";

/**
 * A hub merge that lands this soon after opening is the catch-up of what happened
 * elsewhere (the desktop shows its local copy first and merges the hub's in the
 * background), not a live edit the user watched arrive.
 */
const CATCH_UP_WINDOW_MS = 60_000;
/** While tracking, a changed board is also stored this often, in case the app dies. */
const SAFETY_SAVE_MS = 120_000;
/** A pending notice follows edits with this delay, so typing never re-diffs per keystroke. */
const PENDING_REFRESH_MS = 400;

export interface IBoardChangeNotice {
	/** The board as the user last had it on screen. */
	base: IBoard;
	seenAt: number;
	diff: IBoardDiff;
}

type Phase =
	| { kind: "idle"; key: string }
	| { kind: "opening"; key: string }
	| ({
			kind: "pending";
			key: string;
			awaitingBoard?: IBoard;
	  } & IBoardChangeNotice)
	| { kind: "tracking"; key: string };

type CatchUpMerge = NonNullable<RemoteBoardAppliedDetail["merge"]> & {
	observed: IBoard | undefined;
};

interface Opened {
	key: string;
	at: number;
	seenAt?: number;
}

/**
 * Remembers, per user and board, the last state the user had on screen, and reports
 * what changed since then when the board is opened again.
 *
 * The main thread only serializes the board in short slices and stores a compressed
 * string; parsing, diffing, fingerprinting and compression run in the snapshot worker.
 * The decision waits for a `fresh` board, because both apps restore persisted query data
 * first. While tracking, the snapshot is written when the user leaves (and every two
 * minutes if the board changed); an unchanged board is never rewritten.
 */
export function useBoardChangeNotice({
	appId,
	boardId,
	userKey,
	board,
	fresh,
	enabled,
	service = boardLastSeenService(),
}: Readonly<{
	appId: string;
	boardId: string;
	userKey: string;
	board: IBoard | undefined;
	/** The board came from the source of truth after mount, not from a restored cache. */
	fresh: boolean;
	enabled: boolean;
	service?: BoardLastSeenService;
}>) {
	const key = `${userKey}:${appId}:${boardId}`;
	const ids = useMemo<LastSeenIds>(
		() => ({ userKey, appId, boardId }),
		[appId, boardId, userKey],
	);
	const [phase, setPhase] = useState<Phase>({ kind: "idle", key });
	const keyRef = useRef(key);
	keyRef.current = key;
	const boardRef = useRef(board);
	boardRef.current = board;
	const openedRef = useRef<Opened | undefined>(undefined);
	const catchUpRef = useRef<CatchUpMerge[]>([]);
	const [catchUp, setCatchUp] = useState<CatchUpMerge>();
	const savedRef = useRef<IBoard | undefined>(undefined);
	const saveChainRef = useRef<Promise<void>>(Promise.resolve());

	const current = phase.key === key ? phase : undefined;
	const isEnabled = enabled && Boolean(boardId);

	const persist = useCallback(
		(target: IBoard | undefined, forIds: LastSeenIds) => {
			if (!target || target === savedRef.current) return saveChainRef.current;
			savedRef.current = target;
			saveChainRef.current = saveChainRef.current.then(() =>
				service.save(forIds, target),
			);
			return saveChainRef.current;
		},
		[service],
	);

	useEffect(() => {
		openedRef.current = undefined;
		catchUpRef.current = [];
		setCatchUp(undefined);
		savedRef.current = undefined;
		setPhase({ kind: "idle", key });
	}, [key]);

	const idle = isEnabled && current?.kind === "idle";
	useEffect(() => {
		if (!idle || !board || !fresh) return;
		setPhase({ kind: "opening", key });
		const opening = service.open(ids, board).catch((error) => {
			console.warn(
				`[board-last-seen] comparing board ${boardId} with its last seen state failed`,
				error,
			);
			return { kind: "first" } as const;
		});
		void opening.then((result) => {
			if (keyRef.current !== key) return;
			openedRef.current = {
				key,
				at: Date.now(),
				seenAt: result.kind === "first" ? undefined : result.seenAt,
			};
			if (result.kind === "news") {
				setPhase({
					kind: "pending",
					key,
					base: result.base,
					seenAt: result.seenAt,
					diff: result.diff,
				});
				return;
			}
			setPhase({ kind: "tracking", key });
			void persist(boardRef.current, ids);
		});
	}, [board, boardId, fresh, idle, ids, key, persist, service]);

	const tracking = isEnabled && current?.kind === "tracking";
	const pending = isEnabled && current?.kind === "pending";

	useEffect(() => {
		if (!isEnabled) return;
		const onRemote = (event: Event) => {
			const detail = readRemoteBoardApplied(event);
			const opened = openedRef.current;
			if (
				detail?.appId !== appId ||
				detail.boardId !== boardId ||
				detail.reason === "reset" ||
				!detail.merge ||
				pending ||
				opened?.key !== key ||
				opened.seenAt === undefined ||
				Date.now() - opened.at > CATCH_UP_WINDOW_MS
			)
				return;
			catchUpRef.current.push({ ...detail.merge, observed: boardRef.current });
			setCatchUp(catchUpRef.current[0]);
		};
		window.addEventListener(REMOTE_BOARD_APPLIED_EVENT, onRemote);
		return () =>
			window.removeEventListener(REMOTE_BOARD_APPLIED_EVENT, onRemote);
	}, [appId, boardId, isEnabled, key, pending]);

	// Compare the merge itself. The board prop may still be waiting for that merge,
	// or already include a later local edit such as a FlowScript apply.
	useEffect(() => {
		const opened = openedRef.current;
		const merge = catchUp;
		if (!tracking || !merge || opened?.key !== key) return;
		if (Date.now() - opened.at > CATCH_UP_WINDOW_MS) {
			catchUpRef.current = [];
			setCatchUp(undefined);
			return;
		}
		let cancelled = false;
		void service
			.compare(ids, merge.after, true, merge.before)
			.then(({ diff, base }) => {
				if (
					cancelled ||
					keyRef.current !== key ||
					catchUpRef.current[0] !== merge
				)
					return;
				catchUpRef.current.shift();
				if (!diff || !base) {
					setCatchUp(catchUpRef.current[0]);
					return;
				}
				// Once a notice is pending, its refresh follows all subsequent merges.
				catchUpRef.current = [];
				setCatchUp(undefined);
				setPhase({
					kind: "pending",
					key,
					base,
					seenAt: opened.seenAt ?? Date.now(),
					diff,
					awaitingBoard: merge.observed,
				});
			});
		return () => {
			cancelled = true;
		};
	}, [catchUp, ids, key, service, tracking]);

	const pendingBase = current?.kind === "pending" ? current.base : undefined;
	const awaitingBoard =
		current?.kind === "pending" ? current.awaitingBoard : undefined;
	useEffect(() => {
		if (!pending || !board || !pendingBase || board === awaitingBoard) return;
		if (awaitingBoard) {
			setPhase((prev) =>
				prev.kind === "pending" && prev.key === key
					? { ...prev, awaitingBoard: undefined }
					: prev,
			);
			return;
		}
		let cancelled = false;
		const timer = setTimeout(() => {
			void service.compare(ids, board, false, pendingBase).then(({ diff }) => {
				if (cancelled || keyRef.current !== key) return;
				setPhase((prev) => {
					if (prev.kind !== "pending" || prev.key !== key) return prev;
					return diff ? { ...prev, diff } : { kind: "tracking", key };
				});
			});
		}, PENDING_REFRESH_MS);
		return () => {
			cancelled = true;
			clearTimeout(timer);
		};
	}, [awaitingBoard, board, ids, key, pending, pendingBase, service]);

	// The board that belongs to the tracked key. `boardRef` already points at the next
	// board when a switch's cleanup runs, so saving it would file one board under another.
	const trackedRef = useRef<IBoard | undefined>(undefined);
	useEffect(() => {
		if (tracking && board) trackedRef.current = board;
	}, [board, tracking]);

	useEffect(() => {
		if (!tracking) return;
		const save = () => void persist(trackedRef.current, ids);
		const interval = setInterval(save, SAFETY_SAVE_MS);
		const onVisibility = () => {
			if (document.visibilityState === "hidden") save();
		};
		const onPageHide = () => {
			const target = trackedRef.current;
			if (!target || target === savedRef.current) return;
			savedRef.current = target;
			service.saveNow(ids, target);
		};
		document.addEventListener("visibilitychange", onVisibility);
		window.addEventListener("pagehide", onPageHide);
		return () => {
			clearInterval(interval);
			document.removeEventListener("visibilitychange", onVisibility);
			window.removeEventListener("pagehide", onPageHide);
			save();
			trackedRef.current = undefined;
		};
	}, [ids, persist, service, tracking]);

	const notice = useMemo<IBoardChangeNotice | undefined>(
		() =>
			pending && current?.kind === "pending"
				? { base: current.base, seenAt: current.seenAt, diff: current.diff }
				: undefined,
		[current, pending],
	);

	// Declared last on purpose: cleanups run in declaration order, so the leave-save above
	// is already queued when the worker is told to drop this board's state.
	useEffect(
		() => () => {
			void saveChainRef.current.then(() => service.forget(ids));
		},
		[ids, service],
	);

	const markSeen = useCallback(async () => {
		if (openedRef.current) openedRef.current.at = Number.NEGATIVE_INFINITY;
		catchUpRef.current = [];
		setCatchUp(undefined);
		setPhase({ kind: "tracking", key });
		await persist(boardRef.current, ids);
	}, [ids, key, persist]);

	return { notice, markSeen };
}
