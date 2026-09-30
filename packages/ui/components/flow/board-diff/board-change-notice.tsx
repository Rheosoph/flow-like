"use client";

import { useTranslation } from "@flow-like/locales";
import { CheckIcon, GitCompareArrowsIcon, HistoryIcon } from "lucide-react";
import { memo, useMemo, useRef } from "react";
import type { IBoardChange } from "../../../lib/board-diff";
import type { IBoard } from "../../../lib/schema/flow/board";
import { Button } from "../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { RelativeTime } from "../../ui/relative-time";
import { UserInlineTag } from "../../ui/user-identity";
import { BoardChangeList, useChangeGroups } from "./board-change-list";
import { StatusGlyph } from "./diff-status";
import type { IBoardChangeNotice } from "./use-board-change-notice";
import { useBoardVersionInfos } from "./use-board-version-infos";

export const BoardChangeNoticeDialog = memo(function BoardChangeNoticeDialog({
	open,
	onOpenChange,
	appId,
	boardId,
	board,
	notice,
	onReview,
	onMarkSeen,
}: Readonly<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	appId: string;
	boardId: string;
	board: IBoard;
	notice: IBoardChangeNotice;
	onReview: (change?: IBoardChange) => void;
	onMarkSeen: () => void;
}>) {
	const { t } = useTranslation("flow");
	const { diff, base, seenAt } = notice;
	const logic = useMemo(
		() => diff.changes.filter((c) => c.status !== "moved"),
		[diff.changes],
	);
	const groups = useChangeGroups(logic, board, base);
	const reviewRef = useRef<HTMLButtonElement>(null);
	const versions = useBoardVersionInfos(appId, boardId, open);
	const publishedSince = useMemo(
		() =>
			(versions.data ?? [])
				.filter((info) => (info.published_at ?? 0) > seenAt)
				.sort((a, b) => (a.published_at ?? 0) - (b.published_at ?? 0)),
		[seenAt, versions.data],
	);

	const pill = (
		status: "added" | "changed" | "removed",
		count: number,
		text: string,
	) =>
		count > 0 && (
			<span className="inline-flex h-6 items-center gap-1.5 rounded-full border bg-card pl-1 pr-2.5 text-xs font-medium tabular-nums">
				<StatusGlyph status={status} className="size-4 text-[10px]" />
				{text}
			</span>
		);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent
				className="flex max-h-[min(88dvh,46rem)] flex-col gap-0 overflow-hidden p-0 sm:max-w-2xl"
				onOpenAutoFocus={(event) => {
					event.preventDefault();
					reviewRef.current?.focus();
				}}
			>
				<DialogHeader className="grid grid-cols-[auto_1fr] gap-x-3.5 gap-y-1 px-5 pb-3 pt-5 text-left">
					<span className="row-span-2 grid size-9 place-items-center rounded-lg bg-primary/10 text-primary">
						<HistoryIcon className="size-4.5" />
					</span>
					<div className="min-w-0">
						<p className="text-xs font-medium text-muted-foreground">
							{board.name}
						</p>
						<DialogTitle className="text-lg leading-snug">
							{t(
								"boardDiffNoticeTitle",
								"This flow changed since you last opened it",
							)}
						</DialogTitle>
					</div>
					<DialogDescription className="col-start-2 text-[13px]">
						{t("boardDiffNoticeLastOpened", "You last had it open")}{" "}
						<RelativeTime
							value={seenAt}
							className="font-medium text-foreground"
						/>
						.{" "}
						{publishedSince.length > 0
							? t("boardDiffNoticePublished", {
									defaultValue_one:
										"Since then, {{count}} version was published.",
									defaultValue_other:
										"Since then, {{count}} versions were published.",
									count: publishedSince.length,
								})
							: t(
									"boardDiffNoticeDraftOnly",
									"Since then, the draft was edited.",
								)}
					</DialogDescription>
				</DialogHeader>

				<div className="flex flex-wrap items-center gap-2 px-5 pb-3 sm:pl-[4.35rem]">
					{pill(
						"added",
						diff.counts.added,
						t("boardDiffCountAdded", {
							defaultValue_one: "{{count}} added",
							defaultValue_other: "{{count}} added",
							count: diff.counts.added,
						}),
					)}
					{pill(
						"changed",
						diff.counts.changed,
						t("boardDiffCountChanged", {
							defaultValue_one: "{{count}} changed",
							defaultValue_other: "{{count}} changed",
							count: diff.counts.changed,
						}),
					)}
					{pill(
						"removed",
						diff.counts.removed,
						t("boardDiffCountRemoved", {
							defaultValue_one: "{{count}} removed",
							defaultValue_other: "{{count}} removed",
							count: diff.counts.removed,
						}),
					)}
					{diff.counts.moved > 0 && (
						<span className="text-xs text-muted-foreground">
							{t("boardDiffCountMoved", {
								defaultValue_one: "· {{count}} layout move not listed",
								defaultValue_other: "· {{count}} layout moves not listed",
								count: diff.counts.moved,
							})}
						</span>
					)}
				</div>

				{publishedSince.length > 0 && (
					<div className="grid gap-1.5 border-t px-5 py-3">
						<p className="text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
							{t("boardDiffPublishedSince", "Published since your visit")}
						</p>
						<ul className="grid gap-1">
							{publishedSince.map((info) => (
								<li
									key={info.version.join(".")}
									className="flex flex-wrap items-center gap-2 text-[13px]"
								>
									<span className="rounded bg-muted px-1.5 font-mono text-xs font-medium">
										v{info.version.join(".")}
									</span>
									{info.published_by && (
										<UserInlineTag userId={info.published_by} />
									)}
									{info.published_at && (
										<RelativeTime
											value={info.published_at}
											className="text-xs text-muted-foreground"
										/>
									)}
								</li>
							))}
						</ul>
					</div>
				)}

				<div className="min-h-0 flex-1 overflow-y-auto border-t px-3 pb-3">
					<BoardChangeList
						groups={groups}
						onSelect={(change) => onReview(change)}
						expandSelected={false}
						emptyText={t("boardDiffNoticeEmpty", "Nothing to review.")}
					/>
				</div>

				<DialogFooter className="flex-row flex-wrap items-center gap-2 border-t bg-muted/30 px-5 py-3 sm:justify-end">
					<Button variant="ghost" onClick={onMarkSeen}>
						<CheckIcon className="size-4" />
						{t("boardDiffMarkSeen", "Mark as seen")}
					</Button>
					<Button ref={reviewRef} onClick={() => onReview()}>
						<GitCompareArrowsIcon className="size-4" />
						{t("boardDiffReview", "Review changes")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
});
