"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import type { IBoardChange } from "../../../lib/board-diff";
import type { IBoard } from "../../../lib/schema/flow/board";
import type { BoardVersion } from "../../../lib/schema/flow/board-version";
import { BoardChangeNoticeDialog } from "./board-change-notice";
import {
	BoardCompareDialog,
	type BoardCompareSelection,
} from "./board-compare-dialog";
import {
	type IBoardChangeNotice,
	useBoardChangeNotice,
} from "./use-board-change-notice";

export interface BoardDiffController {
	notice: IBoardChangeNotice | undefined;
	noticeOpen: boolean;
	setNoticeOpen: (open: boolean) => void;
	compare: BoardCompareSelection | undefined;
	closeCompare: () => void;
	openReview: (change?: IBoardChange) => void;
	openCompare: () => void;
	markSeen: () => void;
}

/**
 * Owns the "changed since your last visit" notice and the compare dialog for one board.
 * The notice opens by itself once per board when there is something new to see.
 */
export function useBoardDiffController({
	appId,
	boardId,
	userKey,
	board,
	boardFresh,
	version,
}: Readonly<{
	appId: string;
	boardId: string;
	userKey: string;
	board: IBoard | undefined;
	/** The board query has data from its source, not only a restored cache entry. */
	boardFresh: boolean;
	version: BoardVersion | undefined;
}>): BoardDiffController {
	const { t } = useTranslation("flow");
	const { notice, markSeen: persistSeen } = useBoardChangeNotice({
		appId,
		boardId,
		userKey,
		board,
		fresh: boardFresh,
		enabled: version === undefined,
	});
	const [noticeOpen, setNoticeOpen] = useState(false);
	const [compare, setCompare] = useState<BoardCompareSelection | undefined>();
	const announcedRef = useRef<string | undefined>(undefined);
	const boardKey = `${appId}:${boardId}`;
	const hasNotice = Boolean(notice);

	useEffect(() => {
		if (!hasNotice || announcedRef.current === boardKey) return;
		announcedRef.current = boardKey;
		setNoticeOpen(true);
	}, [boardKey, hasNotice]);

	useEffect(() => {
		if (!hasNotice) setNoticeOpen(false);
	}, [hasNotice]);

	const markSeen = useCallback(() => {
		setNoticeOpen(false);
		void persistSeen();
		toast.success(
			t(
				"boardDiffMarkedSeen",
				"Marked as seen. You'll only hear about changes made from now on.",
			),
		);
	}, [persistSeen, t]);

	const openReview = useCallback((change?: IBoardChange) => {
		setNoticeOpen(false);
		setCompare({
			base: { kind: "last-visit" },
			head: { kind: "draft" },
			changeKey: change?.key,
		});
	}, []);

	const openCompare = useCallback(() => {
		setCompare({
			base: version ? { kind: "version", version } : undefined,
			head: { kind: "draft" },
		});
	}, [version]);

	const closeCompare = useCallback(() => setCompare(undefined), []);

	return {
		notice,
		noticeOpen,
		setNoticeOpen,
		compare,
		closeCompare,
		openReview,
		openCompare,
		markSeen,
	};
}

export function BoardDiffDialogs({
	controller,
	appId,
	boardId,
	board,
	onOpenVersion,
}: Readonly<{
	controller: BoardDiffController;
	appId: string;
	boardId: string;
	board: IBoard | undefined;
	onOpenVersion: (version: BoardVersion) => void;
}>) {
	const {
		notice,
		noticeOpen,
		setNoticeOpen,
		compare,
		closeCompare,
		openReview,
		markSeen,
	} = controller;
	return (
		<>
			{notice && board && (
				<BoardChangeNoticeDialog
					open={noticeOpen}
					onOpenChange={setNoticeOpen}
					appId={appId}
					boardId={boardId}
					board={board}
					notice={notice}
					onReview={openReview}
					onMarkSeen={markSeen}
				/>
			)}
			<BoardCompareDialog
				open={Boolean(compare)}
				onOpenChange={(open) => {
					if (!open) closeCompare();
				}}
				appId={appId}
				boardId={boardId}
				draft={board}
				lastSeen={
					notice ? { board: notice.base, seenAt: notice.seenAt } : undefined
				}
				initial={compare ?? { head: { kind: "draft" } }}
				onMarkSeen={notice ? markSeen : undefined}
				onOpenVersion={onOpenVersion}
			/>
		</>
	);
}
