"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import {
	ArrowLeftIcon,
	ArrowLeftRightIcon,
	CheckIcon,
	EyeIcon,
	GitCompareArrowsIcon,
	LoaderCircleIcon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useInvoke } from "../../../hooks/use-invoke";
import { diffBoards } from "../../../lib/board-diff";
import type { IBoard } from "../../../lib/schema/flow/board";
import type { BoardVersion } from "../../../lib/schema/flow/board-version";
import { useBackend } from "../../../state/backend-state";
import { Button } from "../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { BoardDiffWorkspace } from "./board-diff-workspace";
import {
	BoardVersionTimeline,
	type CompareRef,
	type TimelineStop,
	buildTimeline,
	compareRefKey,
	useStopLabel,
} from "./board-version-timeline";
import type { FlowScriptSource } from "./flowscript-diff";
import { useBoardVersionInfos } from "./use-board-version-infos";

export interface BoardCompareSelection {
	/** Omitted: the newest published version, else the last visit. */
	base?: CompareRef;
	head: CompareRef;
	/** Change to open first, e.g. the one picked in the notice. */
	changeKey?: string;
}

function systemTimeMs(value: IBoard["updated_at"] | undefined) {
	if (!value) return undefined;
	return (
		value.secs_since_epoch * 1000 +
		Math.floor((value.nanos_since_epoch ?? 0) / 1e6)
	);
}

function useRefBoard(
	appId: string,
	boardId: string,
	ref: CompareRef,
	draft: IBoard | undefined,
	lastSeen: IBoard | undefined,
	enabled: boolean,
) {
	const backend = useBackend();
	const version = ref.kind === "version" ? ref.version : undefined;
	const query = useInvoke(
		backend.boardState.getBoard,
		backend.boardState,
		[appId, boardId, version],
		enabled && (ref.kind === "version" || (ref.kind === "draft" && !draft)),
	);
	if (ref.kind === "draft") {
		return draft
			? { board: draft, loading: false, error: undefined }
			: {
					board: query.data,
					loading: query.isLoading,
					error: query.error?.message,
				};
	}
	if (ref.kind === "last-visit")
		return { board: lastSeen, loading: false, error: undefined };
	return {
		board: query.data,
		loading: query.isLoading,
		error: query.error?.message,
	};
}

function useRefScript(
	appId: string,
	boardId: string,
	ref: CompareRef,
	board: IBoard | undefined,
	enabled: boolean,
): FlowScriptSource {
	const { t } = useTranslation("flow");
	const backend = useBackend();
	const draftStamp =
		ref.kind === "draft" ? systemTimeMs(board?.updated_at) : undefined;
	const query = useQuery<string>({
		queryKey: [
			"board-compare-flowscript",
			appId,
			boardId,
			compareRefKey(ref),
			draftStamp,
		],
		enabled: enabled && (ref.kind !== "last-visit" || Boolean(board)),
		staleTime: ref.kind === "version" ? Number.POSITIVE_INFINITY : 0,
		queryFn: async () => {
			const state = backend.boardState;
			if (ref.kind === "last-visit") {
				if (!state.renderFlowScript || !board) {
					throw new Error(
						t(
							"boardDiffScriptUnavailable",
							"FlowScript for your last visit can't be rendered in this app.",
						),
					);
				}
				return state.renderFlowScript(appId, boardId, board, true);
			}
			return state.getFlowScript(
				appId,
				boardId,
				ref.kind === "version" ? ref.version : undefined,
				true,
			);
		},
	});
	return {
		text: query.data,
		loading: query.isLoading,
		error: query.error?.message,
	};
}

export function BoardCompareDialog({
	open,
	onOpenChange,
	appId,
	boardId,
	draft,
	lastSeen,
	initial,
	onMarkSeen,
	onOpenVersion,
}: Readonly<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	appId: string;
	boardId: string;
	draft: IBoard | undefined;
	/** The state the user last saw, while changes since then are unread. */
	lastSeen?: { board: IBoard; seenAt: number };
	initial: BoardCompareSelection;
	onMarkSeen?: () => void;
	onOpenVersion?: (version: BoardVersion) => void;
}>) {
	const { t } = useTranslation("flow");
	const label = useStopLabel();
	const [base, setBase] = useState<CompareRef>(
		initial.base ?? { kind: "draft" },
	);
	const [head, setHead] = useState<CompareRef>(initial.head);
	const [autoBase, setAutoBase] = useState(!initial.base);
	const [nextRole, setNextRole] = useState<"base" | "head">("base");
	const [wide, setWide] = useState<[CompareRef, CompareRef] | undefined>();

	const initialKey = `${initial.base ? compareRefKey(initial.base) : "auto"}|${compareRefKey(initial.head)}|${initial.changeKey ?? ""}`;
	// biome-ignore lint/correctness/useExhaustiveDependencies: reset only when the dialog is (re)opened with a new selection
	useEffect(() => {
		if (!open) return;
		setBase(initial.base ?? { kind: "draft" });
		setHead(initial.head);
		setAutoBase(!initial.base);
		setNextRole("base");
		setWide(undefined);
	}, [open, initialKey]);

	const versions = useBoardVersionInfos(appId, boardId, open);
	const stops = useMemo(
		() =>
			buildTimeline(
				versions.data ?? [],
				lastSeen?.seenAt,
				systemTimeMs(draft?.updated_at),
			),
		[draft?.updated_at, lastSeen?.seenAt, versions.data],
	);
	const stopByKey = useMemo(
		() => new Map(stops.map((s) => [s.key, s])),
		[stops],
	);

	useEffect(() => {
		if (!open || !autoBase || versions.isLoading) return;
		const fallback =
			[...stops].reverse().find((stop) => stop.ref.kind === "version") ??
			stops.find((stop) => stop.ref.kind === "last-visit");
		if (fallback) setBase(fallback.ref);
		setAutoBase(false);
	}, [autoBase, open, stops, versions.isLoading]);
	const nothingToCompare = !versions.isLoading && stops.length < 2;

	const baseBoard = useRefBoard(
		appId,
		boardId,
		base,
		draft,
		lastSeen?.board,
		open,
	);
	const headBoard = useRefBoard(
		appId,
		boardId,
		head,
		draft,
		lastSeen?.board,
		open,
	);
	const baseScript = useRefScript(appId, boardId, base, baseBoard.board, open);
	const headScript = useRefScript(appId, boardId, head, headBoard.board, open);

	const diff = useMemo(
		() =>
			baseBoard.board && headBoard.board
				? diffBoards(baseBoard.board, headBoard.board)
				: undefined,
		[baseBoard.board, headBoard.board],
	);

	const baseKey = compareRefKey(base);
	const headKey = compareRefKey(head);
	const baseStop = stopByKey.get(baseKey) ?? { key: baseKey, ref: base };
	const headStop = stopByKey.get(headKey) ?? { key: headKey, ref: head };

	const pick = useCallback(
		(stop: TimelineStop) => {
			setWide(undefined);
			if (nextRole === "base") {
				if (stop.key === headKey) setHead(base);
				setBase(stop.ref);
				setNextRole("head");
			} else {
				if (stop.key === baseKey) setBase(head);
				setHead(stop.ref);
				setNextRole("base");
			}
		},
		[base, baseKey, head, headKey, nextRole],
	);

	const selectRef = (role: "base" | "head") => (key: string) => {
		const stop = stopByKey.get(key);
		if (!stop) return;
		setWide(undefined);
		const other = role === "base" ? head : base;
		if (compareRefKey(other) === key) {
			if (role === "base") setHead(base);
			else setBase(head);
		}
		if (role === "base") setBase(stop.ref);
		else setHead(stop.ref);
	};

	const baseIndex = stops.findIndex((s) => s.key === baseKey);
	const headIndex = stops.findIndex((s) => s.key === headKey);
	const lo = Math.min(baseIndex, headIndex);
	const hi = Math.max(baseIndex, headIndex);
	const steps =
		baseIndex >= 0 && headIndex >= 0 && hi - lo > 1
			? stops.slice(lo, hi + 1)
			: [];
	const reversed = baseIndex > headIndex && headIndex >= 0;

	const sinceLastVisit = base.kind === "last-visit" && head.kind === "draft";
	const openable =
		base.kind === "version"
			? base.version
			: head.kind === "version"
				? head.version
				: undefined;
	const loading = baseBoard.loading || headBoard.loading;
	const error = baseBoard.error ?? headBoard.error;

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent
				className="flex h-[min(92dvh,62rem)] w-[min(96vw,110rem)] max-w-none flex-col gap-0 overflow-hidden p-0 outline-none sm:max-w-none"
				onOpenAutoFocus={(event) => {
					event.preventDefault();
					(event.currentTarget as HTMLElement | null)?.focus();
				}}
			>
				<DialogHeader className="gap-1 border-b px-5 pb-3 pt-4 text-left">
					<div className="flex flex-wrap items-start justify-between gap-3 pr-8">
						<div className="min-w-0">
							<DialogTitle className="flex items-center gap-2 text-lg">
								<GitCompareArrowsIcon className="size-4 text-primary" />
								{sinceLastVisit
									? t("boardDiffSinceTitle", "Changes since your last visit")
									: t("boardDiffCompareTitle", "Compare versions")}
							</DialogTitle>
							<DialogDescription className="mt-1">
								{sinceLastVisit && lastSeen
									? t(
											"boardDiffSinceDescription",
											"Compared with what you had on screen on {{date}}.",
											{
												date: new Date(lastSeen.seenAt).toLocaleString(
													undefined,
													{ dateStyle: "medium", timeStyle: "short" },
												),
											},
										)
									: t(
											"boardDiffCompareDescription",
											"Pick any two points in this flow's history, including the unpublished draft.",
										)}
							</DialogDescription>
						</div>
						<div className="flex flex-wrap items-center gap-2">
							{openable && onOpenVersion && (
								<Button
									variant="outline"
									size="sm"
									onClick={() => {
										onOpenVersion(openable);
										onOpenChange(false);
									}}
								>
									<EyeIcon className="size-3.5" />
									{t("boardDiffOpenVersion", "Open v{{version}}", {
										version: openable.join("."),
									})}
								</Button>
							)}
							{onMarkSeen && base.kind === "last-visit" && (
								<Button
									size="sm"
									onClick={() => {
										onMarkSeen();
										onOpenChange(false);
									}}
								>
									<CheckIcon className="size-3.5" />
									{t("boardDiffMarkSeen", "Mark as seen")}
								</Button>
							)}
						</div>
					</div>
				</DialogHeader>

				<div className="grid gap-2.5 border-b px-5 py-3">
					{versions.isLoading ? (
						<div className="flex h-24 items-center justify-center text-sm text-muted-foreground">
							<LoaderCircleIcon className="mr-2 size-4 animate-spin" />
							{t("boardDiffLoadingVersions", "Loading versions…")}
						</div>
					) : (
						<BoardVersionTimeline
							stops={stops}
							baseKey={baseKey}
							headKey={headKey}
							onPick={pick}
						/>
					)}
					<div className="flex flex-wrap items-center gap-x-3 gap-y-2 text-xs">
						<RefSelect
							label={t("boardDiffBase", "Base")}
							value={baseKey}
							stops={stops}
							onChange={selectRef("base")}
						/>
						<Button
							variant="outline"
							size="icon"
							className="size-7"
							aria-label={t("boardDiffSwap", "Swap base and compare")}
							title={t("boardDiffSwap", "Swap base and compare")}
							onClick={() => {
								setBase(head);
								setHead(base);
							}}
						>
							<ArrowLeftRightIcon className="size-3.5" />
						</Button>
						<RefSelect
							label={t("boardDiffCompare", "Compare")}
							value={headKey}
							stops={stops}
							onChange={selectRef("head")}
						/>
						<span className="text-muted-foreground">
							{nextRole === "base"
								? t("boardDiffPickBase", "Click the timeline to set the base")
								: t(
										"boardDiffPickCompare",
										"Click the timeline to set the compare version",
									)}
						</span>
						{reversed && (
							<span className="rounded-full bg-tertiary/15 px-2 py-0.5 text-foreground">
								{t(
									"boardDiffReversed",
									"The base is newer, so this shows what rolling back would change",
								)}
							</span>
						)}
					</div>
					{(steps.length > 0 || wide) && (
						<div className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
							{wide && (
								<Button
									variant="outline"
									size="sm"
									className="h-6 rounded-full border-dashed px-2 text-xs"
									onClick={() => {
										setBase(wide[0]);
										setHead(wide[1]);
										setWide(undefined);
									}}
								>
									<ArrowLeftIcon className="size-3" />
									{t("boardDiffBackToRange", "Back to {{from}} → {{to}}", {
										from: label(
											stopByKey.get(compareRefKey(wide[0])) ?? {
												key: "",
												ref: wide[0],
											},
										),
										to: label(
											stopByKey.get(compareRefKey(wide[1])) ?? {
												key: "",
												ref: wide[1],
											},
										),
									})}
								</Button>
							)}
							{steps.length > 0 && (
								<>
									<span>
										{t("boardDiffSteps", {
											defaultValue_one:
												"{{count}} step in this range. Step through one:",
											defaultValue_other:
												"{{count}} steps in this range. Step through one:",
											count: steps.length - 1,
										})}
									</span>
									{steps.slice(1).map((stop, index) => {
										const previous = steps[index];
										return (
											<Button
												key={stop.key}
												variant="outline"
												size="sm"
												className="h-6 rounded-full px-2 font-mono text-[11px]"
												onClick={() => {
													setWide([base, head]);
													setBase(previous.ref);
													setHead(stop.ref);
												}}
											>
												{label(previous)} → {label(stop)}
											</Button>
										);
									})}
								</>
							)}
						</div>
					)}
				</div>

				<div className="min-h-0 flex-1">
					{nothingToCompare ? (
						<p className="flex h-full items-center justify-center p-6 text-center text-sm text-muted-foreground">
							{t(
								"boardDiffNothingToCompare",
								"Publish a version of this flow to compare it with the draft.",
							)}
						</p>
					) : error ? (
						<p className="p-6 text-sm text-destructive">{error}</p>
					) : autoBase ||
						loading ||
						!diff ||
						!baseBoard.board ||
						!headBoard.board ? (
						<div className="flex h-full items-center justify-center text-sm text-muted-foreground">
							<LoaderCircleIcon className="mr-2 size-4 animate-spin" />
							{t("boardDiffLoadingBoards", "Loading both versions…")}
						</div>
					) : (
						<BoardDiffWorkspace
							key={`${baseKey}|${headKey}`}
							base={baseBoard.board}
							head={headBoard.board}
							diff={diff}
							baseLabel={label(baseStop)}
							headLabel={label(headStop)}
							baseScript={baseScript}
							headScript={headScript}
							initialKey={initial.changeKey}
						/>
					)}
				</div>
			</DialogContent>
		</Dialog>
	);
}

function RefSelect({
	label,
	value,
	stops,
	onChange,
}: Readonly<{
	label: string;
	value: string;
	stops: TimelineStop[];
	onChange: (key: string) => void;
}>) {
	const stopLabel = useStopLabel();
	return (
		<div className="inline-flex items-center gap-2">
			<span
				aria-hidden
				className="text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground"
			>
				{label}
			</span>
			<Select value={value} onValueChange={onChange}>
				<SelectTrigger aria-label={label} className="h-7 min-w-36 text-xs">
					<SelectValue />
				</SelectTrigger>
				<SelectContent>
					{[...stops].reverse().map((stop) => (
						<SelectItem key={stop.key} value={stop.key} className="text-xs">
							{stopLabel(stop)}
						</SelectItem>
					))}
				</SelectContent>
			</Select>
		</div>
	);
}
