"use client";
import { useTranslation } from "@flow-like/locales";
import {
	CheckCircle2Icon,
	CircleDashedIcon,
	FlaskConicalIcon,
	Loader2Icon,
	PlayIcon,
	ScrollTextIcon,
	Trash2Icon,
	TriangleAlertIcon,
	XCircleIcon,
} from "lucide-react";
import { memo, useCallback, useMemo, useRef } from "react";
import { createPortal } from "react-dom";
import {
	type IBoardTestCase,
	type ILogMetadata,
	type INode,
	type IVariable,
	formatDuration,
} from "../../lib";
import {
	assertionText,
	discoverBoardTests,
	runBoardTest,
} from "../../lib/board-tests";
import { cn } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import {
	type IBoardTestEntry,
	type IBoardTestStatus,
	boardTestSummary,
	useBoardTestsStore,
} from "../../state/board-tests-state";
import { useLogAggregation } from "../../state/log-aggregation-state";
import { Button } from "../ui";
import { usePanelToolbarSlot } from "./shell/board-panes";

export interface IBoardTestRunPreparation {
	ok: boolean;
	runtimeVariables?: Record<string, IVariable>;
}

const TestStatusIcon = ({ status }: { status?: IBoardTestStatus }) => {
	const { t } = useTranslation("flow");
	switch (status) {
		case "running":
			return (
				<Loader2Icon
					aria-label={t("testStatusRunning", "Running")}
					className="size-3.5 shrink-0 animate-spin text-muted-foreground"
				/>
			);
		case "pass":
			return (
				<CheckCircle2Icon
					aria-label={t("testStatusPassed", "Passed")}
					className="size-3.5 shrink-0 text-emerald-500"
				/>
			);
		case "fail":
			return (
				<XCircleIcon
					aria-label={t("testStatusFailed", "Failed")}
					className="size-3.5 shrink-0 text-destructive"
				/>
			);
		case "error":
			return (
				<TriangleAlertIcon
					aria-label={t("testStatusError", "Error")}
					className="size-3.5 shrink-0 text-amber-500"
				/>
			);
		default:
			return (
				<CircleDashedIcon
					aria-label={t("testStatusNotRun", "Not run yet")}
					className="size-3.5 shrink-0 text-muted-foreground/60"
				/>
			);
	}
};

const FlowTestsComponent = ({
	appId,
	boardId,
	nodes,
	onFocusNode,
	onOpenRunLogs,
	prepareRun,
	executeTest,
	variant = "page",
}: {
	appId: string;
	boardId: string;
	nodes: { [key: string]: INode };
	onFocusNode: (nodeId: string) => void;
	/** Show a finished test run's logs (typically in the Traces panel). */
	onOpenRunLogs: (meta: ILogMetadata) => void;
	/** One-time pre-flight for a batch: WASM consent + runtime variables. */
	prepareRun: (representative: INode) => Promise<IBoardTestRunPreparation>;
	/** Raw single-event execution; throws on failure, returns the run's metadata. */
	executeTest: (
		node: INode,
		runtimeVariables?: Record<string, IVariable>,
	) => Promise<ILogMetadata | undefined>;
	/** `panel` drops the page padding — the shell's panel frames it. */
	variant?: "page" | "panel";
}) => {
	const { t } = useTranslation("flow");
	const backend = useBackend();
	const { refetchLogs } = useLogAggregation();
	const entries = useBoardTestsStore((state) => state.entries[boardId]);
	const begin = useBoardTestsStore((state) => state.begin);
	const complete = useBoardTestsStore((state) => state.complete);
	const clear = useBoardTestsStore((state) => state.clear);

	const tests = useMemo(() => discoverBoardTests(nodes), [nodes]);
	const liveNodeIds = useMemo(
		() => new Set(tests.map((test) => test.node.id)),
		[tests],
	);
	const summary = useMemo(
		() => boardTestSummary(entries, liveNodeIds),
		[entries, liveNodeIds],
	);
	const running = summary.running > 0;
	const runInFlightRef = useRef(false);

	const panel = variant === "panel";
	const toolbarSlot = usePanelToolbarSlot();
	const hoisted = panel && toolbarSlot !== null;

	const queryRun = useCallback(
		(meta: ILogMetadata, query: string, offset?: number, limit?: number) =>
			backend.boardState.queryRun(meta, query, offset, limit),
		[backend],
	);

	const runTests = useCallback(
		async (selection: IBoardTestCase[]) => {
			// The store only marks tests running after the async pre-flight, so a
			// synchronous ref has to close the double-click window.
			if (selection.length === 0 || runInFlightRef.current) return;
			runInFlightRef.current = true;
			try {
				const preparation = await prepareRun(selection[0].node);
				if (!preparation.ok) return;
				begin(
					boardId,
					selection.map((test) => test.node.id),
				);
				await Promise.all(
					selection.map(async (test) => {
						const result = await runBoardTest(
							queryRun,
							(node) => executeTest(node, preparation.runtimeVariables),
							test,
						);
						complete(boardId, result);
					}),
				);
				await refetchLogs(backend);
			} finally {
				runInFlightRef.current = false;
			}
		},
		[
			prepareRun,
			begin,
			boardId,
			queryRun,
			executeTest,
			complete,
			refetchLogs,
			backend,
		],
	);

	const toolbar = (
		<div className="flex items-center gap-1">
			{(summary.passed > 0 || summary.failed > 0) && (
				<span className="flex items-center gap-2 px-1 text-[11px] tabular-nums normal-case tracking-normal">
					{summary.passed > 0 && (
						<span className="text-emerald-500">
							{t("testsPassed", {
								defaultValue_one: "{{count}} passed",
								defaultValue_other: "{{count}} passed",
								count: summary.passed,
							})}
						</span>
					)}
					{summary.failed > 0 && (
						<span className="text-destructive">
							{t("testsFailed", {
								defaultValue_one: "{{count}} failed",
								defaultValue_other: "{{count}} failed",
								count: summary.failed,
							})}
						</span>
					)}
				</span>
			)}
			<Button
				size="sm"
				variant="ghost"
				className="h-6 gap-1 px-2 text-[11px]"
				disabled={running || tests.length === 0}
				onClick={() => runTests(tests)}
			>
				{running ? (
					<Loader2Icon className="size-3 animate-spin" />
				) : (
					<PlayIcon className="size-3" />
				)}
				{t("runAllTests", "Run All Tests")}
			</Button>
			<Button
				size="sm"
				variant="ghost"
				className="h-6 px-2 text-[11px]"
				aria-label={t("clearTestResults", "Clear test results")}
				disabled={running || !entries}
				onClick={() => clear(boardId)}
			>
				<Trash2Icon className="size-3" />
			</Button>
		</div>
	);

	return (
		<div
			className={cn("flex h-full min-h-0 flex-col", panel ? "" : "gap-2 p-4")}
		>
			{hoisted ? (
				createPortal(toolbar, toolbarSlot)
			) : (
				<div className="flex shrink-0 items-center justify-end border-b px-2 py-1">
					{toolbar}
				</div>
			)}
			{tests.length === 0 ? (
				<div className="flex h-full flex-col items-center justify-center gap-1 p-4 text-center">
					<FlaskConicalIcon className="size-5 text-muted-foreground/60" />
					<p className="text-sm font-medium">
						{t("noTestsOnBoard", "No tests on this board")}
					</p>
					<p className="max-w-md text-xs text-muted-foreground">
						{t(
							"testConventionHint",
							"A board test is an event whose name starts with “test” and checks outcomes with Assert nodes.",
						)}
					</p>
				</div>
			) : (
				<ul className="flex min-h-0 flex-1 flex-col overflow-auto p-1">
					{tests.map((test) => (
						<FlowTestRow
							key={test.node.id}
							test={test}
							entry={entries?.[test.node.id]}
							running={running}
							onRun={() => runTests([test])}
							onFocusNode={onFocusNode}
							onOpenRunLogs={onOpenRunLogs}
						/>
					))}
				</ul>
			)}
		</div>
	);
};

const FlowTestRow = memo(function FlowTestRow({
	test,
	entry,
	running,
	onRun,
	onFocusNode,
	onOpenRunLogs,
}: Readonly<{
	test: IBoardTestCase;
	entry?: IBoardTestEntry;
	running: boolean;
	onRun: () => void;
	onFocusNode: (nodeId: string) => void;
	onOpenRunLogs: (meta: ILogMetadata) => void;
}>) {
	const { t } = useTranslation("flow");
	const result = entry?.result;
	const failed = entry?.status === "fail" || entry?.status === "error";
	return (
		<li className="group rounded-sm hover:bg-accent/50">
			<div className="flex w-full items-center gap-2 px-2 py-1 text-xs">
				<TestStatusIcon status={entry?.status} />
				<button
					type="button"
					onClick={() => onFocusNode(test.node.id)}
					className="min-w-0 truncate text-left font-medium hover:underline"
					title={t("focusNode", "Focus node")}
				>
					{test.alias}
				</button>
				<span className="flex-1" />
				{result && (
					<span className="shrink-0 tabular-nums text-muted-foreground">
						{result.assertOk + result.assertFail > 0 &&
							`${result.assertOk}/${result.assertOk + result.assertFail} · `}
						{formatDuration(result.durationMs * 1000)}
					</span>
				)}
				{result?.metadata && (
					<Button
						size="sm"
						variant="ghost"
						className="h-5 w-5 p-0 opacity-0 focus-visible:opacity-100 group-hover:opacity-100"
						aria-label={t("viewTestLogs", "View logs")}
						onClick={() => {
							if (result.metadata) onOpenRunLogs(result.metadata);
						}}
					>
						<ScrollTextIcon className="size-3" />
					</Button>
				)}
				<Button
					size="sm"
					variant="ghost"
					className="h-5 w-5 p-0 opacity-0 focus-visible:opacity-100 group-hover:opacity-100"
					aria-label={t("runTest", "Run test")}
					disabled={running}
					onClick={onRun}
				>
					<PlayIcon className="size-3" />
				</Button>
			</div>
			{failed && result && (
				<div className="flex flex-col gap-0.5 px-8 pb-1.5 text-[11px]">
					{result.failedAssertions.map((log, index) => (
						<p
							key={`${result.runId}-assert-${index.toString()}`}
							className="wrap-break-word text-destructive"
						>
							{assertionText(log)}
						</p>
					))}
					{result.executionError && (
						<p className="wrap-break-word text-destructive">
							{result.executionError}
						</p>
					)}
					{result.failedAssertions.length === 0 &&
						result.errorLogs.map((log, index) => (
							<p
								key={`${result.runId}-error-${index.toString()}`}
								className={cn(
									"wrap-break-word text-destructive",
									result.executionError && "text-muted-foreground",
								)}
							>
								{log.message}
							</p>
						))}
				</div>
			)}
		</li>
	);
});

export const FlowTests = memo(FlowTestsComponent);
