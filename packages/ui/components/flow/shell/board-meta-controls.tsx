"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import {
	CloudIcon,
	MonitorIcon,
	ShieldAlertIcon,
	ShieldCheckIcon,
	ShuffleIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import { memo, useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { useInvalidateInvoke, useInvoke } from "../../../hooks";
import {
	type IBoard,
	IExecutionMode,
	IExecutionStage,
	ILogLevel,
	IVersionType,
} from "../../../lib";
import { useBackend } from "../../../state/backend-state";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { Textarea } from "../../ui/textarea";

interface BoardMetaValues {
	name: string;
	description: string;
	stage: IExecutionStage;
	logLevel: ILogLevel;
	executionMode: IExecutionMode;
}

function valuesOf(board: IBoard): BoardMetaValues {
	return {
		name: board.name,
		description: board.description,
		stage: board.stage,
		logLevel: board.log_level,
		executionMode: board.execution_mode ?? IExecutionMode.Hybrid,
	};
}

/**
 * `upsertBoard` replaces the whole meta record, so every control has to send the
 * fields it does not own. One patch function reads them off the board, which is
 * also what makes the settings safe to scatter across the status bar.
 */
function useBoardMetaPatch(appId: string, boardId: string, board: IBoard) {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();

	return useCallback(
		async (partial: Partial<BoardMetaValues>) => {
			const next = { ...valuesOf(board), ...partial };
			await backend.boardState.upsertBoard(
				appId,
				boardId,
				next.name,
				next.description,
				next.logLevel,
				next.stage,
				next.executionMode,
			);
			await invalidate(backend.boardState.getBoard, [appId, boardId]);
		},
		[appId, boardId, board, backend, invalidate],
	);
}

const Field = memo(function Field({
	label,
	hint,
	children,
}: Readonly<{ label: string; hint?: string; children: ReactNode }>) {
	return (
		<div className="grid w-full gap-1.5">
			<Label className="text-xs">{label}</Label>
			{children}
			{hint && <p className="text-[11px] text-muted-foreground">{hint}</p>}
		</div>
	);
});

/** Name and description, edited where the name already shows. */
export const BoardIdentityForm = memo(function BoardIdentityForm({
	appId,
	boardId,
	board,
}: Readonly<{ appId: string; boardId: string; board: IBoard }>) {
	const { t } = useTranslation("flow");
	const patch = useBoardMetaPatch(appId, boardId, board);
	const [draft, setDraft] = useState(() => ({
		name: board.name,
		description: board.description,
	}));
	const [saving, setSaving] = useState(false);

	useEffect(() => {
		setDraft({ name: board.name, description: board.description });
	}, [board.name, board.description]);

	const dirty =
		draft.name !== board.name || draft.description !== board.description;

	const save = useCallback(async () => {
		setSaving(true);
		try {
			await patch(draft);
		} finally {
			setSaving(false);
		}
	}, [patch, draft]);

	return (
		<div className="flex flex-col gap-3">
			<Field label={t("name", "Name")}>
				<Input
					value={draft.name}
					className="h-8"
					onChange={(event) =>
						setDraft((old) => ({ ...old, name: event.target.value }))
					}
				/>
			</Field>
			<Field label={t("description", "Description")}>
				<Textarea
					value={draft.description}
					rows={4}
					className="resize-none text-xs"
					onChange={(event) =>
						setDraft((old) => ({ ...old, description: event.target.value }))
					}
				/>
			</Field>
			<Button size="sm" disabled={!dirty || saving} onClick={() => void save()}>
				{t("save", "Save")}
			</Button>
		</div>
	);
});

const EXECUTION_MODE_ICON: Record<IExecutionMode, ReactNode> = {
	[IExecutionMode.Local]: <MonitorIcon className="size-3.5" />,
	[IExecutionMode.Remote]: <CloudIcon className="size-3.5" />,
	[IExecutionMode.Hybrid]: <ShuffleIcon className="size-3.5" />,
};

export function executionModeIcon(mode: IExecutionMode): ReactNode {
	return (
		EXECUTION_MODE_ICON[mode] ?? EXECUTION_MODE_ICON[IExecutionMode.Hybrid]
	);
}

/** Where a run happens and how much it records — both only matter when running. */
export const BoardRuntimeForm = memo(function BoardRuntimeForm({
	appId,
	boardId,
	board,
	isOffline,
}: Readonly<{
	appId: string;
	boardId: string;
	board: IBoard;
	isOffline?: boolean;
}>) {
	const { t } = useTranslation("flow");
	const patch = useBoardMetaPatch(appId, boardId, board);
	const values = valuesOf(board);

	// What each mode actually does. Choosing between them is consequential —
	// Remote is what a board with secrets needs — so the copy stays with the control.
	const MODE_HINT: Record<IExecutionMode, string> = {
		[IExecutionMode.Hybrid]: t(
			"runsLocallyWhenPossibleFallsBackToRemoteExecution",
			"Runs locally when possible, falls back to remote execution.",
		),
		[IExecutionMode.Remote]: t(
			"alwaysRunsOnRemoteServersRequiredForBoardsWithSecrets",
			"Always runs on remote servers. Required for boards with secrets.",
		),
		[IExecutionMode.Local]: t(
			"alwaysRunsLocallyBestForHighperformanceWorkloadsLikeEmbeddings",
			"Always runs locally. Best for high-performance workloads like embeddings.",
		),
	};

	return (
		<div className="flex flex-col gap-3">
			<Field
				label={t("executionMode", "Execution Mode")}
				hint={
					isOffline
						? t(
								"offlineProjectsOnlySupportLocalExecution",
								"Offline projects only support local execution.",
							)
						: MODE_HINT[values.executionMode]
				}
			>
				<Select
					value={values.executionMode}
					disabled={isOffline}
					onValueChange={(value) =>
						void patch({ executionMode: value as IExecutionMode })
					}
				>
					<SelectTrigger className="h-8">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{Object.values(IExecutionMode).map((mode) => (
							<SelectItem key={mode} value={mode}>
								<span className="flex items-center gap-2">
									{executionModeIcon(mode)}
									{mode}
								</span>
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</Field>
			<Field
				label={t("logLevel", "Log Level")}
				hint={t(
					"theLowestLevelThisBoardRecordsWhileItRuns",
					"The lowest level this board records while it runs.",
				)}
			>
				<Select
					value={values.logLevel}
					onValueChange={(value) =>
						void patch({ logLevel: value as ILogLevel })
					}
				>
					<SelectTrigger className="h-8">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{Object.values(ILogLevel).map((level) => (
							<SelectItem key={level} value={level}>
								{level}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</Field>
		</div>
	);
});

interface IBoardGateVerdict {
	verdict: "pass" | "fail";
	/** Dotted board version the suite run graded. */
	boardVersion: string;
	regressed: number;
	completedAt?: string | null;
}

/**
 * The newest completed regression-suite run touching this board, folded into
 * a pass/fail verdict — the read-only gate display. Derived client-side from
 * the event-keyed runs listing (there is no board-keyed gate route); draft
 * runs never carry a gate verdict and are skipped.
 */
function useBoardGateVerdict(appId: string, boardId: string) {
	const backend = useBackend();
	const supported = typeof backend.eventState.listRegressionRuns === "function";
	return useQuery<IBoardGateVerdict | null>({
		queryKey: ["boardGateVerdict", appId, boardId],
		enabled: Boolean(appId && boardId && supported),
		staleTime: 30_000,
		queryFn: async () => {
			const listRegressionRuns = backend.eventState.listRegressionRuns;
			if (!listRegressionRuns) return null;
			const events = await backend.eventState.getEvents(appId);
			const candidates = events.filter(
				(event) => event.board_id === boardId && !event.default_page_id,
			);
			const lists = await Promise.all(
				candidates.map(async (event) => {
					try {
						return await listRegressionRuns.call(
							backend.eventState,
							appId,
							event.id,
						);
					} catch {
						// 404 — the event has no regression suite.
						return [];
					}
				}),
			);
			const completed = lists
				.flat()
				.filter(
					(run) => run.status === "completed" && run.board_version !== "draft",
				)
				.sort((a, b) => (a.created_at < b.created_at ? 1 : -1));
			const newest = completed[0];
			if (!newest) return null;
			return {
				verdict: newest.regressed > 0 ? "fail" : "pass",
				boardVersion: newest.board_version,
				regressed: newest.regressed,
				completedAt: newest.completed_at ?? newest.created_at,
			};
		},
	});
}

/** Which version is open, which stage it ships at, and cutting a new one. */
export const BoardReleaseForm = memo(function BoardReleaseForm({
	appId,
	boardId,
	board,
	version,
	selectVersion,
}: Readonly<{
	appId: string;
	boardId: string;
	board: IBoard;
	version?: [number, number, number];
	selectVersion: (version?: [number, number, number]) => void;
}>) {
	const { t } = useTranslation("flow");
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const patch = useBoardMetaPatch(appId, boardId, board);
	const [creating, setCreating] = useState(false);
	const versions = useInvoke(
		backend.boardState.getBoardVersions,
		backend.boardState,
		[appId, boardId],
	);

	// Publishing bumps the board's own version, so the draft the editor holds is
	// stale the moment the snapshot lands — without this the status bar and the
	// "Latest" entry keep naming the version that was just superseded.
	const createVersion = useCallback(
		async (type: IVersionType) => {
			setCreating(true);
			try {
				await backend.boardState.createBoardVersion(appId, boardId, type);
				await Promise.all([
					versions.refetch(),
					invalidate(backend.boardState.getBoard, [appId, boardId]),
				]);
			} catch (error) {
				console.error("Failed to create board version", error);
				toast.error(t("failedToCreateVersion", "Failed to create version"));
			} finally {
				setCreating(false);
			}
		},
		[appId, boardId, backend, versions, invalidate, t],
	);

	// While a version is pinned, `board` is that immutable snapshot and its
	// version field is the pinned number — naming it "Latest" would hide the
	// draft entirely. Read the draft separately, and only while pinned; the
	// unpinned key is the one the editor already holds, so this hits cache.
	const draft = useInvoke(
		backend.boardState.getBoard,
		backend.boardState,
		[appId, boardId],
		typeof version !== "undefined",
	);
	const latest = version ? draft.data?.version : board.version;
	const gate = useBoardGateVerdict(appId, boardId);

	return (
		<div className="flex flex-col gap-3">
			<Field label={t("version", "Version")}>
				<Select
					value={version ? version.join(".") : "Latest"}
					onValueChange={(value) => {
						if (value === "Latest") {
							selectVersion(undefined);
							return;
						}
						selectVersion(
							value.split(".").map(Number) as [number, number, number],
						);
					}}
				>
					<SelectTrigger className="h-8">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="Latest">
							{latest
								? t("latestVersion", "Latest ({{version}})", {
										version: latest.join("."),
									})
								: t("latest", "Latest")}
						</SelectItem>
						{(versions.data ?? []).map((entry) => (
							<SelectItem key={entry.join(".")} value={entry.join(".")}>
								{entry.join(".")}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
				{gate.data && (
					<p
						className="flex items-center gap-1.5 text-[11px] text-muted-foreground"
						title={t(
							"regressionGateChipTitle",
							"Newest completed regression-suite run touching this flow. Configure suites on the event's Quality section.",
						)}
					>
						{gate.data.verdict === "pass" ? (
							<ShieldCheckIcon className="h-3.5 w-3.5 shrink-0 text-emerald-500" />
						) : (
							<ShieldAlertIcon className="h-3.5 w-3.5 shrink-0 text-destructive" />
						)}
						{gate.data.verdict === "pass"
							? t(
									"regressionGatePass",
									"Regression gate: pass (v{{version}})",
									{
										version: gate.data.boardVersion,
									},
								)
							: t(
									"regressionGateFail",
									"Regression gate: {{count}} regressed (v{{version}})",
									{
										count: gate.data.regressed,
										version: gate.data.boardVersion,
									},
								)}
					</p>
				)}
			</Field>

			<Field
				label={t("stage", "Stage")}
				hint={t(
					"whichEnvironmentThisBoardIsConsideredReadyFor",
					"Which environment this board is considered ready for.",
				)}
			>
				<Select
					value={board.stage}
					onValueChange={(value) =>
						void patch({ stage: value as IExecutionStage })
					}
				>
					<SelectTrigger className="h-8">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{Object.values(IExecutionStage).map((stage) => (
							<SelectItem key={stage} value={stage}>
								{stage}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</Field>

			<div className="grid gap-1.5">
				<Label className="text-xs">
					{t("createVersion", "Create Version")}
				</Label>
				<div className="grid grid-cols-3 gap-1.5">
					{[IVersionType.Major, IVersionType.Minor, IVersionType.Patch].map(
						(type) => (
							<Button
								key={type}
								size="sm"
								variant="outline"
								disabled={creating}
								onClick={() => void createVersion(type)}
							>
								{type}
							</Button>
						),
					)}
				</div>
			</div>
		</div>
	);
});
