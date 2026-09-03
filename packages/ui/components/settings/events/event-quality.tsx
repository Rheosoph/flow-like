"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlertTriangleIcon,
	BanIcon,
	CheckCircle2Icon,
	CircleXIcon,
	ClockIcon,
	CloudIcon,
	DatabaseIcon,
	FlaskConicalIcon,
	LayoutIcon,
	Loader2Icon,
	PlayIcon,
	PlusIcon,
	ScrollTextIcon,
	ShieldAlertIcon,
	Trash2Icon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { useInvalidateInvoke, useInvoke } from "../../../hooks/use-invoke";
import { discoverBoardTests } from "../../../lib/board-tests";
import { formatRelativeTime } from "../../../lib/date";
import { logLevelToNumber } from "../../../lib/log-level";
import type { INode } from "../../../lib/schema/flow/board";
import type { IEvent } from "../../../lib/schema/flow/event";
import type { ILog } from "../../../lib/schema/flow/log";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import type {
	IEventCorpusResult,
	IPutRegressionSuiteRequest,
	IRegressionCaseResult,
	IRegressionCorpusEntry,
	IRegressionFixtureSummary,
	IRegressionGateMode,
	IRegressionSuiteResult,
	IRegressionSuiteRunDetail,
	IRegressionSuiteRunSummary,
} from "../../../state/backend-state/event-state";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Checkbox,
	Dialog,
	DialogBody,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	Input,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Switch,
} from "../../ui";

const CAVEAT_REJECTED = "rejected";
const CAVEAT_TOO_LARGE = "too_large";
const CAVEAT_EMPTY = "empty";
const CAVEAT_GRADING_BLIND = "grading_blind";
const CAVEAT_CALLER_OAUTH = "caller_oauth_tokens";

/** Poll cadence while a suite run is `running` (the POST is fire-and-forget). */
const RUN_POLL_INTERVAL_MS = 2500;

const messageOf = (error: unknown): string =>
	error instanceof Error ? error.message : String(error);

const microsToSystemTime = (micros: number) => ({
	secs_since_epoch: Math.floor(micros / 1_000_000),
	nanos_since_epoch: (micros % 1_000_000) * 1000,
});

const formatIsoTime = (iso?: string | null): string => {
	if (!iso) return "";
	const date = new Date(iso);
	return Number.isNaN(date.getTime()) ? iso : date.toLocaleString();
};

// Never invoked — `enabled` requires the real method; these only satisfy
// useInvoke's non-optional function parameter (same pattern as EventHistory).
async function suiteUnavailable(
	_appId: string,
	_eventId: string,
): Promise<IRegressionSuiteResult | null> {
	throw new Error("Regression suites are not supported on this platform");
}
async function corpusUnavailable(
	_appId: string,
	_eventId: string,
	_limit?: number,
): Promise<IEventCorpusResult> {
	throw new Error("The run corpus is not supported on this platform");
}
async function runsUnavailable(
	_appId: string,
	_eventId: string,
): Promise<IRegressionSuiteRunSummary[]> {
	return [];
}
async function runDetailUnavailable(
	_appId: string,
	_eventId: string,
	_suiteRunId: string,
): Promise<IRegressionSuiteRunDetail> {
	throw new Error("Suite run details are not supported on this platform");
}

export function EventQuality({
	appId,
	event,
	nodes,
	onReload,
}: Readonly<{
	appId: string;
	event: IEvent;
	/** Nodes of the event's current board, for authored tests and names. */
	nodes?: Record<string, INode>;
	onReload?: () => void;
}>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();

	const supported = typeof backend.eventState.getRegressionSuite === "function";
	const isPageEvent = !!event.default_page_id;
	const isOntologyAction = event.event_type === "ontology_action";
	const excluded = isPageEvent || isOntologyAction;
	const enabled = Boolean(appId && event.id && supported && !excluded);

	const suiteQuery = useInvoke<IRegressionSuiteResult | null, [string, string]>(
		backend.eventState.getRegressionSuite ?? suiteUnavailable,
		backend.eventState,
		[appId, event.id],
		enabled,
	);
	const suite = suiteQuery.data ?? null;

	const corpusQuery = useInvoke<IEventCorpusResult, [string, string]>(
		backend.eventState.getEventCorpus ?? corpusUnavailable,
		backend.eventState,
		[appId, event.id],
		enabled && typeof backend.eventState.getEventCorpus === "function",
	);

	const runsQuery = useInvoke<IRegressionSuiteRunSummary[], [string, string]>(
		backend.eventState.listRegressionRuns ?? runsUnavailable,
		backend.eventState,
		[appId, event.id],
		enabled &&
			typeof backend.eventState.listRegressionRuns === "function" &&
			suite !== null,
	);

	const authoredTests = useMemo(() => discoverBoardTests(nodes), [nodes]);

	const nodeName = useCallback(
		(nodeId: string) => nodes?.[nodeId]?.friendly_name ?? nodeId,
		[nodes],
	);

	const refreshSuite = useCallback(async () => {
		if (backend.eventState.getRegressionSuite) {
			await invalidate(backend.eventState.getRegressionSuite, [
				appId,
				event.id,
			]);
		}
		onReload?.();
	}, [appId, event.id, backend.eventState, invalidate, onReload]);

	const refreshRuns = useCallback(async () => {
		if (backend.eventState.listRegressionRuns) {
			await invalidate(backend.eventState.listRegressionRuns, [
				appId,
				event.id,
			]);
		}
	}, [appId, event.id, backend.eventState, invalidate]);

	if (excluded) {
		return (
			<Card>
				<CardContent className="flex flex-col items-center gap-2 py-10 text-center">
					<LayoutIcon className="size-5 text-muted-foreground/60" />
					<p className="text-sm font-medium">
						{t(
							"qualityNotForThisEventType",
							"Regression suites are not available for this event type",
						)}
					</p>
					<p className="max-w-[52ch] text-xs text-muted-foreground">
						{isPageEvent
							? t(
									"qualityNotForPageEventsDetail",
									"Page payloads are sealed to their page session, so recorded inputs cannot be replayed.",
								)
							: t(
									"qualityNotForOntologyActionsDetail",
									"Ontology actions are generated machinery with a governed endpoint of their own.",
								)}
					</p>
				</CardContent>
			</Card>
		);
	}

	if (!supported) {
		return (
			<Card>
				<CardContent className="flex flex-col items-center gap-2 py-10 text-center">
					<CloudIcon className="size-5 text-muted-foreground/60" />
					<p className="text-sm font-medium">
						{t(
							"qualityCloudOnly",
							"Regression suites aren't available on this platform yet",
						)}
					</p>
					<p className="max-w-[52ch] text-xs text-muted-foreground">
						{t(
							"qualityCloudOnlyDetail",
							"Open this event in a cloud-hosted app to build a regression set from its recorded runs.",
						)}
					</p>
				</CardContent>
			</Card>
		);
	}

	return (
		<div className="space-y-6">
			<CorpusCard
				appId={appId}
				eventId={event.id}
				corpus={corpusQuery.data}
				loading={corpusQuery.isLoading}
				error={corpusQuery.isError ? messageOf(corpusQuery.error) : null}
				suiteExists={suite !== null}
				nodeName={nodeName}
				onPromoted={refreshSuite}
			/>

			<FixturesCard
				appId={appId}
				eventId={event.id}
				fixtures={suite?.fixtures ?? []}
				suiteExists={suite !== null}
				nodeName={nodeName}
				onDeleted={refreshSuite}
			/>

			<AuthoredTestsCard tests={authoredTests} />

			<SuiteConfigCard
				appId={appId}
				eventId={event.id}
				suite={suite}
				loading={suiteQuery.isLoading}
				error={suiteQuery.isError ? messageOf(suiteQuery.error) : null}
				onSaved={refreshSuite}
			/>

			<RunPanelCard
				appId={appId}
				event={event}
				suite={suite}
				runs={runsQuery.data ?? []}
				runsLoading={runsQuery.isLoading}
				nodeName={nodeName}
				onRunsChanged={refreshRuns}
			/>
		</div>
	);
}

/* ------------------------------------------------------------------ corpus */

function CaveatChip({ caveat }: Readonly<{ caveat: string }>) {
	const { t } = useTranslation("settings");
	const labels: Record<string, { label: string; title: string }> = {
		[CAVEAT_REJECTED]: {
			label: t("corpusCaveatRejected", "rejected"),
			title: t(
				"corpusCaveatRejectedTitle",
				"This trigger was rejected before execution — its payload never reached the flow.",
			),
		},
		[CAVEAT_TOO_LARGE]: {
			label: t("corpusCaveatTooLarge", "too large"),
			title: t(
				"corpusCaveatTooLargeTitle",
				"The redacted payload exceeds the fixture size cap, so this run cannot be promoted.",
			),
		},
		[CAVEAT_EMPTY]: {
			label: t("corpusCaveatEmpty", "no payload"),
			title: t("corpusCaveatEmptyTitle", "The run recorded no input payload."),
		},
		[CAVEAT_GRADING_BLIND]: {
			label: t("fixtureCaveatGradingBlind", "grading blind"),
			title: t(
				"fixtureCaveatGradingBlindTitle",
				"The board's log level discards assertion markers, so a green verdict cannot be justified — only errors are detected.",
			),
		},
		[CAVEAT_CALLER_OAUTH]: {
			label: t("fixtureCaveatCallerOauth", "caller OAuth"),
			title: t(
				"fixtureCaveatCallerOauthTitle",
				"The recorded run carried caller OAuth tokens, which are not part of the fixture — a suite with this fixture cannot be scheduled.",
			),
		},
	};
	const entry = labels[caveat] ?? { label: caveat, title: caveat };
	return (
		<Badge
			variant="outline"
			title={entry.title}
			className="h-5 gap-1 px-1.5 text-[10px] font-medium text-amber-600 dark:text-amber-500"
		>
			<AlertTriangleIcon className="size-2.5" />
			{entry.label}
		</Badge>
	);
}

function RunLevelIcon({ level }: Readonly<{ level: number }>) {
	if (level >= 4) return <BanIcon className="h-3 w-3 text-red-800" />;
	if (level === 3) return <CircleXIcon className="h-3 w-3 text-red-500" />;
	if (level === 2)
		return <AlertTriangleIcon className="h-3 w-3 text-yellow-500" />;
	return <CheckCircle2Icon className="h-3 w-3 text-green-500" />;
}

function CorpusCard({
	appId,
	eventId,
	corpus,
	loading,
	error,
	suiteExists,
	nodeName,
	onPromoted,
}: Readonly<{
	appId: string;
	eventId: string;
	corpus?: IEventCorpusResult;
	loading: boolean;
	error: string | null;
	suiteExists: boolean;
	nodeName: (nodeId: string) => string;
	onPromoted: () => Promise<void>;
}>) {
	const { t } = useTranslation("settings");
	const [promoteTarget, setPromoteTarget] =
		useState<IRegressionCorpusEntry | null>(null);

	const entries = corpus?.entries ?? [];

	return (
		<Card>
			<CardHeader>
				<CardTitle className="flex items-center gap-2">
					<DatabaseIcon className="h-5 w-5" />
					{t("corpusTitle", "Recorded inputs")}
				</CardTitle>
				<CardDescription>
					{t(
						"corpusDescription",
						"Recent real inputs of this event, deduplicated by payload shape with failing inputs kept. Previews are redacted. Promote the ones worth keeping into the regression set.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-3">
				{loading && (
					<div className="flex items-center gap-2 py-6 justify-center text-sm text-muted-foreground">
						<Loader2Icon className="h-4 w-4 animate-spin" />
						{t("corpusLoading", "Scanning recorded runs…")}
					</div>
				)}
				{error && (
					<div className="flex gap-3 rounded-lg border border-destructive/40 bg-destructive/10 p-3 text-sm">
						<AlertTriangleIcon className="mt-0.5 h-4 w-4 shrink-0 text-destructive" />
						<p>
							{t("corpusLoadFailed", "Could not load the corpus: {{val}}", {
								val: error,
							})}
						</p>
					</div>
				)}
				{!loading && !error && entries.length === 0 && (
					<p className="py-6 text-center text-sm text-muted-foreground">
						{t(
							"corpusEmpty",
							"No recorded inputs yet. Once this event runs, its real inputs appear here as fixture candidates.",
						)}
					</p>
				)}
				{entries.length > 0 && (
					<>
						{!suiteExists && (
							<p className="rounded-md border border-dashed p-2 text-xs text-muted-foreground">
								{t(
									"corpusNeedsSuite",
									"Save the suite configuration below before promoting runs into the regression set.",
								)}
							</p>
						)}
						<ul className="flex flex-col divide-y">
							{entries.map((entry) => {
								const tooLarge = entry.caveats.includes(CAVEAT_TOO_LARGE);
								return (
									<li
										key={entry.run_id}
										className="flex flex-col gap-1.5 py-2.5"
									>
										<div className="flex items-center gap-2">
											<RunLevelIcon level={entry.log_level} />
											<span className="text-xs font-medium">
												{nodeName(entry.node_id)}
											</span>
											<span className="text-[11px] text-muted-foreground">
												{formatRelativeTime(
													microsToSystemTime(entry.start),
													"narrow",
												)}
											</span>
											<span className="text-[11px] text-muted-foreground">
												{entry.board_version}
											</span>
											<span className="text-[11px] tabular-nums text-muted-foreground">
												{(entry.payload_len / 1024).toFixed(1)} KiB
											</span>
											{entry.caveats.map((caveat) => (
												<CaveatChip key={caveat} caveat={caveat} />
											))}
											<span className="flex-1" />
											<Button
												size="sm"
												variant="outline"
												className="h-6 gap-1 px-2 text-[11px]"
												disabled={!suiteExists || tooLarge}
												title={
													tooLarge
														? t(
																"corpusTooLargeToPromote",
																"The redacted payload exceeds the fixture size cap.",
															)
														: undefined
												}
												onClick={() => setPromoteTarget(entry)}
											>
												<PlusIcon className="size-3" />
												{t("addToRegressionSet", "Add to regression set")}
											</Button>
										</div>
										{entry.preview && (
											<pre className="max-h-24 overflow-auto rounded bg-muted/50 p-2 text-[11px] leading-snug text-muted-foreground">
												{entry.preview}
											</pre>
										)}
									</li>
								);
							})}
						</ul>
					</>
				)}
				{promoteTarget && (
					<PromoteFixtureDialog
						appId={appId}
						eventId={eventId}
						entry={promoteTarget}
						onClose={() => setPromoteTarget(null)}
						onPromoted={onPromoted}
					/>
				)}
			</CardContent>
		</Card>
	);
}

function PromoteFixtureDialog({
	appId,
	eventId,
	entry,
	onClose,
	onPromoted,
}: Readonly<{
	appId: string;
	eventId: string;
	entry: IRegressionCorpusEntry;
	onClose: () => void;
	onPromoted: () => Promise<void>;
}>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const [expectation, setExpectation] = useState<"auto" | "pass" | "fail">(
		"auto",
	);
	const [acknowledgeRejected, setAcknowledgeRejected] = useState(false);
	const [busy, setBusy] = useState(false);

	const isRejected = entry.caveats.includes(CAVEAT_REJECTED);

	const promote = useCallback(async () => {
		const promoteRegressionFixture =
			backend.eventState.promoteRegressionFixture;
		if (!promoteRegressionFixture) return;
		setBusy(true);
		try {
			await promoteRegressionFixture.call(
				backend.eventState,
				appId,
				eventId,
				entry.run_id,
				{
					expectation: expectation === "auto" ? undefined : expectation,
					acknowledgeRejected: isRejected ? acknowledgeRejected : undefined,
				},
			);
			toast.success(t("fixturePromoted", "Run added to the regression set"));
			await onPromoted();
			onClose();
		} catch (error) {
			toast.error(
				t("fixturePromoteFailed", "Could not promote the run: {{val}}", {
					val: messageOf(error),
				}),
			);
		} finally {
			setBusy(false);
		}
	}, [
		appId,
		eventId,
		entry.run_id,
		expectation,
		acknowledgeRejected,
		isRejected,
		backend.eventState,
		onPromoted,
		onClose,
		t,
	]);

	return (
		<Dialog open onOpenChange={(open) => !open && onClose()}>
			<DialogContent className="sm:max-w-lg">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<FlaskConicalIcon className="h-4 w-4" />
						{t("promoteFixtureTitle", "Add to regression set")}
					</DialogTitle>
					<DialogDescription>
						{t(
							"promoteFixtureDescription",
							"The run's redacted payload becomes a fixture, and its graded verdict becomes the baseline future replays are compared against — a failing run is a first-class fixture too.",
						)}
					</DialogDescription>
				</DialogHeader>
				<DialogBody className="space-y-4">
					{entry.preview && (
						<pre className="max-h-40 overflow-auto rounded bg-muted/50 p-2 text-[11px] leading-snug text-muted-foreground">
							{entry.preview}
						</pre>
					)}
					<div className="grid gap-1.5">
						<Label className="text-xs">
							{t("fixtureExpectation", "Baseline expectation")}
						</Label>
						<Select
							value={expectation}
							onValueChange={(value) =>
								setExpectation(value as "auto" | "pass" | "fail")
							}
						>
							<SelectTrigger className="h-8">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="auto">
									{t(
										"fixtureExpectationAuto",
										"Graded from the recorded run (recommended)",
									)}
								</SelectItem>
								<SelectItem value="pass">
									{t("fixtureExpectationPass", "Expect pass")}
								</SelectItem>
								<SelectItem value="fail">
									{t("fixtureExpectationFail", "Expect fail")}
								</SelectItem>
							</SelectContent>
						</Select>
					</div>
					{isRejected && (
						<label
							htmlFor="promote-acknowledge-rejected"
							className="flex cursor-pointer items-start gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-2.5 text-xs"
						>
							<Checkbox
								id="promote-acknowledge-rejected"
								checked={acknowledgeRejected}
								onCheckedChange={(checked) =>
									setAcknowledgeRejected(checked === true)
								}
								className="mt-0.5"
							/>
							<span>
								{t(
									"fixtureAcknowledgeRejected",
									"This run was rejected before execution — its payload never reached the flow. Promote it anyway.",
								)}
							</span>
						</label>
					)}
				</DialogBody>
				<DialogFooter>
					<Button variant="outline" onClick={onClose} disabled={busy}>
						{t("cancel", "Cancel")}
					</Button>
					<Button
						onClick={() => void promote()}
						disabled={busy || (isRejected && !acknowledgeRejected)}
					>
						{busy && <Loader2Icon className="mr-1 size-3 animate-spin" />}
						{t("promoteFixture", "Add fixture")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

/* ---------------------------------------------------------------- fixtures */

function FixturesCard({
	appId,
	eventId,
	fixtures,
	suiteExists,
	nodeName,
	onDeleted,
}: Readonly<{
	appId: string;
	eventId: string;
	fixtures: IRegressionFixtureSummary[];
	suiteExists: boolean;
	nodeName: (nodeId: string) => string;
	onDeleted: () => Promise<void>;
}>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const [deleteTarget, setDeleteTarget] =
		useState<IRegressionFixtureSummary | null>(null);
	const [busy, setBusy] = useState(false);

	const remove = useCallback(async () => {
		const deleteRegressionFixture = backend.eventState.deleteRegressionFixture;
		if (!deleteRegressionFixture || !deleteTarget) return;
		setBusy(true);
		try {
			await deleteRegressionFixture.call(
				backend.eventState,
				appId,
				eventId,
				deleteTarget.id,
			);
			toast.success(t("fixtureDeleted", "Fixture deleted"));
			setDeleteTarget(null);
			await onDeleted();
		} catch (error) {
			toast.error(
				t("fixtureDeleteFailed", "Could not delete the fixture: {{val}}", {
					val: messageOf(error),
				}),
			);
		} finally {
			setBusy(false);
		}
	}, [appId, eventId, deleteTarget, backend.eventState, onDeleted, t]);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="flex items-center gap-2">
					<FlaskConicalIcon className="h-5 w-5" />
					{t("fixturesTitle", "Regression set")}
				</CardTitle>
				<CardDescription>
					{t(
						"fixturesDescription",
						"Promoted recorded inputs. Each replay is compared against the baseline verdict captured at promotion — never output against output.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent>
				{fixtures.length === 0 && (
					<p className="py-4 text-center text-sm text-muted-foreground">
						{suiteExists
							? t(
									"fixturesEmpty",
									"No fixtures yet. Promote recorded inputs above to build the set.",
								)
							: t(
									"fixturesNeedSuite",
									"Save the suite configuration below, then promote recorded inputs into the set.",
								)}
					</p>
				)}
				{fixtures.length > 0 && (
					<ul className="flex flex-col divide-y">
						{fixtures.map((fixture) => (
							<li
								key={fixture.id}
								className="flex items-center gap-2 py-2 text-xs"
							>
								<Badge
									variant={
										fixture.baseline.verdict === "pass"
											? "outline"
											: "secondary"
									}
									className={cn(
										"h-5 px-1.5 text-[10px]",
										fixture.baseline.verdict === "pass"
											? "text-emerald-600 dark:text-emerald-500"
											: "text-amber-600 dark:text-amber-500",
									)}
								>
									{fixture.baseline.verdict === "pass"
										? t("fixtureExpectsPass", "expects pass")
										: t("fixtureExpectsFail", "expects fail")}
								</Badge>
								{fixture.baseline.error_class && (
									<span
										className="text-[11px] text-muted-foreground"
										title={t(
											"fixtureErrorClassTitle",
											"Error class of the failing baseline",
										)}
									>
										{fixture.baseline.error_class}
									</span>
								)}
								<span className="font-medium">
									{nodeName(fixture.source_node_id)}
								</span>
								<span className="text-muted-foreground">
									{formatRelativeTime(
										microsToSystemTime(fixture.baseline.recorded_at),
										"narrow",
									)}
								</span>
								{fixture.caveats.map((caveat) => (
									<CaveatChip key={caveat} caveat={caveat} />
								))}
								<span className="flex-1" />
								<Button
									size="sm"
									variant="ghost"
									className="h-6 w-6 p-0 text-muted-foreground hover:text-destructive"
									aria-label={t("deleteFixture", "Delete fixture")}
									onClick={() => setDeleteTarget(fixture)}
								>
									<Trash2Icon className="size-3.5" />
								</Button>
							</li>
						))}
					</ul>
				)}
				{deleteTarget && (
					<AlertDialog
						open
						onOpenChange={(open) => !open && setDeleteTarget(null)}
					>
						<AlertDialogContent>
							<AlertDialogHeader>
								<AlertDialogTitle>
									{t("deleteFixtureTitle", "Delete this fixture?")}
								</AlertDialogTitle>
								<AlertDialogDescription>
									{t(
										"deleteFixtureDescription",
										"The fixture and its stored payload are removed. Future suite runs no longer replay this input.",
									)}
								</AlertDialogDescription>
							</AlertDialogHeader>
							<AlertDialogFooter>
								<AlertDialogCancel disabled={busy}>
									{t("cancel", "Cancel")}
								</AlertDialogCancel>
								<AlertDialogAction
									disabled={busy}
									onClick={() => void remove()}
								>
									{t("delete", "Delete")}
								</AlertDialogAction>
							</AlertDialogFooter>
						</AlertDialogContent>
					</AlertDialog>
				)}
			</CardContent>
		</Card>
	);
}

/* ----------------------------------------------------------- authored tests */

function AuthoredTestsCard({
	tests,
}: Readonly<{ tests: ReturnType<typeof discoverBoardTests> }>) {
	const { t } = useTranslation("settings");
	return (
		<Card>
			<CardHeader>
				<CardTitle className="flex items-center gap-2">
					<CheckCircle2Icon className="h-5 w-5" />
					{t("authoredTestsTitle", "Authored board tests")}
				</CardTitle>
				<CardDescription>
					{t(
						"authoredTestsDescription",
						"Event start nodes on the flow whose name starts with “test”. They carry semantic expectations via Assert nodes and run with every suite run — no traffic history needed.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent>
				{tests.length === 0 && (
					<p className="py-4 text-center text-sm text-muted-foreground">
						{t(
							"authoredTestsEmpty",
							"No authored tests on this flow. Add an event node named “test …” with Assert nodes to check behaviour, not just errors.",
						)}
					</p>
				)}
				{tests.length > 0 && (
					<ul className="flex flex-col divide-y">
						{tests.map((test) => (
							<li
								key={test.node.id}
								className="flex items-center gap-2 py-2 text-xs"
							>
								<FlaskConicalIcon className="size-3.5 text-muted-foreground" />
								<span className="font-medium">{test.alias}</span>
								<span className="flex-1" />
								<span className="text-[11px] text-muted-foreground">
									{t("authoredTestRunsAlways", "runs with every suite run")}
								</span>
							</li>
						))}
					</ul>
				)}
			</CardContent>
		</Card>
	);
}

/* ------------------------------------------------------------ suite config */

function SuiteConfigCard({
	appId,
	eventId,
	suite,
	loading,
	error,
	onSaved,
}: Readonly<{
	appId: string;
	eventId: string;
	suite: IRegressionSuiteResult | null;
	loading: boolean;
	error: string | null;
	onSaved: () => Promise<void>;
}>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const [draft, setDraft] = useState<IPutRegressionSuiteRequest>({
		trigger_on_publish: false,
		schedule: null,
		gate_mode: "Off",
		allow_live_side_effects: false,
	});
	const [busy, setBusy] = useState(false);

	// Re-seed the draft whenever the stored suite changes (save, promote,
	// delete elsewhere) — the card always edits on top of the persisted state.
	const seedKey = suite
		? `${suite.suite.id}:${suite.suite.updated_at}`
		: "none";
	const seededFor = useRef<string | null>(null);
	useEffect(() => {
		if (seededFor.current === seedKey) return;
		seededFor.current = seedKey;
		setDraft({
			trigger_on_publish: suite?.suite.trigger_on_publish ?? false,
			schedule: suite?.suite.schedule ?? null,
			gate_mode: suite?.suite.gate_mode ?? "Off",
			allow_live_side_effects: suite?.suite.allow_live_side_effects ?? false,
		});
	}, [seedKey, suite]);

	const oauthFixtures = useMemo(
		() =>
			(suite?.fixtures ?? []).some((fixture) =>
				fixture.caveats.includes(CAVEAT_CALLER_OAUTH),
			),
		[suite?.fixtures],
	);
	const scheduleDraft = draft.schedule?.trim() ?? "";
	const scheduleConflict = oauthFixtures && scheduleDraft.length > 0;

	const save = useCallback(async () => {
		const putRegressionSuite = backend.eventState.putRegressionSuite;
		if (!putRegressionSuite) return;
		setBusy(true);
		try {
			await putRegressionSuite.call(backend.eventState, appId, eventId, {
				trigger_on_publish: draft.trigger_on_publish,
				schedule: scheduleDraft.length > 0 ? scheduleDraft : null,
				gate_mode: draft.gate_mode,
				allow_live_side_effects: draft.allow_live_side_effects,
			});
			toast.success(t("suiteSaved", "Regression suite saved"));
			await onSaved();
		} catch (err) {
			toast.error(
				t("suiteSaveFailed", "Could not save the suite: {{val}}", {
					val: messageOf(err),
				}),
			);
		} finally {
			setBusy(false);
		}
	}, [appId, eventId, draft, scheduleDraft, backend.eventState, onSaved, t]);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="flex items-center gap-2">
					<ShieldAlertIcon className="h-5 w-5" />
					{t("suiteConfigTitle", "Suite configuration")}
				</CardTitle>
				<CardDescription>
					{suite
						? t(
								"suiteConfigDescription",
								"When the suite runs automatically, and what a failing run does to promotions.",
							)
						: t(
								"suiteConfigCreateDescription",
								"No regression suite yet. Save one to start promoting recorded inputs and running replays.",
							)}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				{loading && (
					<div className="flex items-center gap-2 py-4 justify-center text-sm text-muted-foreground">
						<Loader2Icon className="h-4 w-4 animate-spin" />
						{t("suiteLoading", "Loading the suite…")}
					</div>
				)}
				{error && (
					<div className="flex gap-3 rounded-lg border border-destructive/40 bg-destructive/10 p-3 text-sm">
						<AlertTriangleIcon className="mt-0.5 h-4 w-4 shrink-0 text-destructive" />
						<p>
							{t("suiteLoadFailed", "Could not load the suite: {{val}}", {
								val: error,
							})}
						</p>
					</div>
				)}
				{!loading && (
					<>
						<label
							htmlFor="suite-allow-side-effects"
							className="flex cursor-pointer items-start gap-2.5 rounded-md border p-3 text-xs"
						>
							<Checkbox
								id="suite-allow-side-effects"
								checked={draft.allow_live_side_effects}
								onCheckedChange={(checked) =>
									setDraft((d) => ({
										...d,
										allow_live_side_effects: checked === true,
									}))
								}
								className="mt-0.5"
							/>
							<span className="space-y-0.5">
								<span className="block font-medium">
									{t(
										"suiteAllowSideEffects",
										"I understand replays execute live side effects",
									)}
								</span>
								<span className="block text-muted-foreground">
									{t(
										"suiteAllowSideEffectsDetail",
										"Replays run isolated from your stored data — storage writes and WASM are sandboxed — but outbound HTTP and other external calls made by native nodes still fire for real: emails send, webhooks post, APIs get called. Every run of this suite is refused until this is acknowledged.",
									)}
								</span>
							</span>
						</label>

						<div className="grid gap-4 sm:grid-cols-2">
							<div className="grid gap-1.5">
								<Label className="text-xs">
									{t("suiteGateMode", "Publish gate")}
								</Label>
								<Select
									value={draft.gate_mode ?? "Off"}
									onValueChange={(value) =>
										setDraft((d) => ({
											...d,
											gate_mode: value as IRegressionGateMode,
										}))
									}
								>
									<SelectTrigger className="h-8">
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value="Off">
											{t("suiteGateOff", "Off — never consulted")}
										</SelectItem>
										<SelectItem value="Warn">
											{t("suiteGateWarn", "Warn — surface a failing verdict")}
										</SelectItem>
										<SelectItem value="Block">
											{t("suiteGateBlock", "Block — refuse a failing promote")}
										</SelectItem>
									</SelectContent>
								</Select>
								<p className="text-[11px] text-muted-foreground">
									{t(
										"suiteGateModeHint",
										"Consulted when a canary is promoted to a version the suite has run against.",
									)}
								</p>
							</div>

							<div className="grid gap-1.5">
								<Label className="text-xs">
									{t("suiteSchedule", "Schedule (cron)")}
								</Label>
								<Input
									className="h-8 font-mono text-xs"
									placeholder="0 3 * * *"
									value={draft.schedule ?? ""}
									onChange={(e) =>
										setDraft((d) => ({ ...d, schedule: e.target.value }))
									}
								/>
								<p className="text-[11px] text-muted-foreground">
									{suite?.next_run_at
										? t("suiteNextRunAt", "Next run: {{val}}", {
												val: formatIsoTime(suite.next_run_at),
											})
										: t(
												"suiteScheduleHint",
												"Leave empty for manual and publish-triggered runs only.",
											)}
								</p>
							</div>
						</div>

						<label
							htmlFor="suite-trigger-on-publish"
							className="flex cursor-pointer items-center gap-2.5 text-xs"
						>
							<Switch
								id="suite-trigger-on-publish"
								checked={draft.trigger_on_publish}
								onCheckedChange={(checked) =>
									setDraft((d) => ({ ...d, trigger_on_publish: checked }))
								}
							/>
							<span>
								{t(
									"suiteTriggerOnPublish",
									"Run automatically when a new flow version is published",
								)}
							</span>
						</label>

						{scheduleConflict && (
							<div className="flex gap-3 rounded-lg border border-amber-500/40 bg-amber-500/10 p-3 text-xs">
								<AlertTriangleIcon className="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
								<p>
									{t(
										"suiteScheduleOauthConflict",
										"This suite contains fixtures recorded with caller OAuth tokens. Those tokens are per-caller and not part of the fixture, so scheduled replays would diverge for reasons unrelated to the flow — saving a schedule will be refused until those fixtures are removed.",
									)}
								</p>
							</div>
						)}

						<div className="flex justify-end">
							<Button size="sm" disabled={busy} onClick={() => void save()}>
								{busy && <Loader2Icon className="mr-1 size-3 animate-spin" />}
								{suite
									? t("suiteSave", "Save suite")
									: t("suiteCreate", "Create suite")}
							</Button>
						</div>
					</>
				)}
			</CardContent>
		</Card>
	);
}

/* ---------------------------------------------------------------- run panel */

function OutcomeChip({ outcome }: Readonly<{ outcome: string }>) {
	const { t } = useTranslation("settings");
	const styles: Record<string, { label: string; className: string }> = {
		ok: {
			label: t("caseOutcomeOk", "OK"),
			className: "text-emerald-600 dark:text-emerald-500",
		},
		regressed: {
			label: t("caseOutcomeRegressed", "REGRESSED"),
			className: "text-destructive",
		},
		still_failing: {
			label: t("caseOutcomeStillFailing", "STILL FAILING"),
			className: "text-amber-600 dark:text-amber-500",
		},
		fixed: {
			label: t("caseOutcomeFixed", "FIXED"),
			className: "text-blue-600 dark:text-blue-400",
		},
		skipped: {
			label: t("caseOutcomeSkipped", "SKIPPED"),
			className: "text-muted-foreground",
		},
	};
	const entry = styles[outcome] ?? {
		label: outcome,
		className: "text-muted-foreground",
	};
	return (
		<Badge
			variant="outline"
			className={cn("h-5 px-1.5 text-[10px] font-semibold", entry.className)}
		>
			{entry.label}
		</Badge>
	);
}

/** `queued` (Lambda deployments defer to the maintenance job) still ends in
 * a terminal status, so it polls and spins like `running`. */
function isActiveRunStatus(status: string | undefined) {
	return status === "running" || status === "queued";
}

function RunStatusIcon({ status }: Readonly<{ status: string }>) {
	if (isActiveRunStatus(status))
		return (
			<Loader2Icon className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
		);
	if (status === "errored")
		return <CircleXIcon className="h-3.5 w-3.5 text-red-500" />;
	return <CheckCircle2Icon className="h-3.5 w-3.5 text-green-500" />;
}

function RunPanelCard({
	appId,
	event,
	suite,
	runs,
	runsLoading,
	nodeName,
	onRunsChanged,
}: Readonly<{
	appId: string;
	event: IEvent;
	suite: IRegressionSuiteResult | null;
	runs: IRegressionSuiteRunSummary[];
	runsLoading: boolean;
	nodeName: (nodeId: string) => string;
	onRunsChanged: () => Promise<void>;
}>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();

	const boardId = suite?.suite.board_id ?? event.board_id ?? "";
	const versionsQuery = useInvoke(
		backend.boardState.getBoardVersions,
		backend.boardState,
		[appId, boardId],
		Boolean(appId && boardId && suite !== null),
	);
	const versionOptions = useMemo(() => {
		const versions = [...(versionsQuery.data ?? [])];
		versions.sort((a, b) => b[0] - a[0] || b[1] - a[1] || b[2] - a[2]);
		return versions.map((version) => version.join("."));
	}, [versionsQuery.data]);

	const [candidate, setCandidate] = useState<string>("latest");
	const [starting, setStarting] = useState(false);
	const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
	const [logsRunId, setLogsRunId] = useState<string | null>(null);

	const detailQuery = useInvoke<
		IRegressionSuiteRunDetail,
		[string, string, string]
	>(
		backend.eventState.getRegressionRun ?? runDetailUnavailable,
		backend.eventState,
		[appId, event.id, selectedRunId ?? ""],
		Boolean(
			appId &&
				event.id &&
				selectedRunId &&
				typeof backend.eventState.getRegressionRun === "function",
		),
	);

	// The POST is fire-and-forget (202): poll the detail while it runs, and
	// refresh the history once so tallies land when it finishes.
	const detailStatus = detailQuery.data?.run.status;
	const { refetch: refetchDetail } = detailQuery;
	useEffect(() => {
		if (!isActiveRunStatus(detailStatus)) return;
		const timer = setInterval(() => {
			void refetchDetail();
		}, RUN_POLL_INTERVAL_MS);
		return () => clearInterval(timer);
	}, [detailStatus, refetchDetail]);
	const previousStatus = useRef<string | undefined>(undefined);
	useEffect(() => {
		if (
			isActiveRunStatus(previousStatus.current) &&
			!isActiveRunStatus(detailStatus)
		) {
			void onRunsChanged();
		}
		previousStatus.current = detailStatus;
	}, [detailStatus, onRunsChanged]);

	const canRun = suite?.suite.allow_live_side_effects === true;

	const startRun = useCallback(async () => {
		const runRegressionSuite = backend.eventState.runRegressionSuite;
		if (!runRegressionSuite) return;
		setStarting(true);
		try {
			const options =
				candidate === "latest"
					? {}
					: candidate === "draft"
						? { allowDraft: true }
						: {
								boardVersion: candidate.split(".").map(Number) as [
									number,
									number,
									number,
								],
							};
			const accepted = await runRegressionSuite.call(
				backend.eventState,
				appId,
				event.id,
				options,
			);
			setSelectedRunId(accepted.suite_run_id);
			toast.success(
				t("suiteRunStarted", "Suite run started — grading replays…"),
			);
			await onRunsChanged();
		} catch (error) {
			toast.error(
				t("suiteRunStartFailed", "Could not start the suite run: {{val}}", {
					val: messageOf(error),
				}),
			);
		} finally {
			setStarting(false);
		}
	}, [appId, event.id, candidate, backend.eventState, onRunsChanged, t]);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="flex items-center gap-2">
					<PlayIcon className="h-5 w-5" />
					{t("suiteRunsTitle", "Suite runs")}
				</CardTitle>
				<CardDescription>
					{t(
						"suiteRunsDescription",
						"Replay every fixture and authored test against a candidate flow version, and compare each verdict to its baseline.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="flex flex-wrap items-end gap-2">
					<div className="grid gap-1.5">
						<Label className="text-xs">
							{t("suiteRunCandidate", "Run against")}
						</Label>
						<Select value={candidate} onValueChange={setCandidate}>
							<SelectTrigger className="h-8 min-w-52">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="latest">
									{t("suiteRunLatestPublished", "Latest published version")}
								</SelectItem>
								<SelectItem value="draft">
									{t("suiteRunDraftHead", "Draft head (never feeds the gate)")}
								</SelectItem>
								{versionOptions.map((version) => (
									<SelectItem key={version} value={version}>
										v{version}
									</SelectItem>
								))}
							</SelectContent>
						</Select>
					</div>
					<Button
						size="sm"
						className="h-8 gap-1.5"
						disabled={!canRun || starting}
						title={
							suite === null
								? t("suiteRunNeedsSuite", "Save the suite configuration first.")
								: !suite.suite.allow_live_side_effects
									? t(
											"suiteRunNeedsAck",
											"Acknowledge live side effects in the suite configuration first.",
										)
									: undefined
						}
						onClick={() => void startRun()}
					>
						{starting ? (
							<Loader2Icon className="size-3.5 animate-spin" />
						) : (
							<PlayIcon className="size-3.5" />
						)}
						{t("suiteRunStart", "Run suite")}
					</Button>
				</div>

				{suite !== null && !suite.suite.allow_live_side_effects && (
					<p className="text-xs text-amber-600 dark:text-amber-500">
						{t(
							"suiteRunsRefusedHint",
							"Runs are refused until live side effects are acknowledged in the suite configuration.",
						)}
					</p>
				)}

				{runsLoading && (
					<div className="flex items-center gap-2 py-4 justify-center text-sm text-muted-foreground">
						<Loader2Icon className="h-4 w-4 animate-spin" />
						{t("suiteRunsLoading", "Loading suite runs…")}
					</div>
				)}
				{!runsLoading && runs.length === 0 && suite !== null && (
					<p className="py-4 text-center text-sm text-muted-foreground">
						{t("suiteRunsEmpty", "This suite hasn't run yet.")}
					</p>
				)}
				{runs.length > 0 && (
					<ul className="flex flex-col divide-y rounded-md border">
						{runs.map((run) => (
							<li key={run.id}>
								<button
									type="button"
									className={cn(
										"flex w-full items-center gap-2 px-3 py-2 text-left text-xs hover:bg-muted/50",
										selectedRunId === run.id && "bg-muted/50",
									)}
									onClick={() =>
										setSelectedRunId((current) =>
											current === run.id ? null : run.id,
										)
									}
								>
									<RunStatusIcon status={run.status} />
									<span className="font-medium">
										{run.board_version === "draft"
											? t("suiteRunDraftLabel", "draft")
											: `v${run.board_version}`}
									</span>
									<span className="text-muted-foreground">{run.trigger}</span>
									<span className="flex-1" />
									{run.regressed > 0 && (
										<span className="font-semibold text-destructive">
											{t("suiteRunRegressedCount", "{{count}} regressed", {
												count: run.regressed,
											})}
										</span>
									)}
									{run.fixed > 0 && (
										<span className="text-blue-600 dark:text-blue-400">
											{t("suiteRunFixedCount", "{{count}} fixed", {
												count: run.fixed,
											})}
										</span>
									)}
									{run.still_failing > 0 && (
										<span className="text-amber-600 dark:text-amber-500">
											{t(
												"suiteRunStillFailingCount",
												"{{count}} still failing",
												{ count: run.still_failing },
											)}
										</span>
									)}
									<span className="text-emerald-600 dark:text-emerald-500">
										{t("suiteRunOkCount", "{{count}} ok", { count: run.ok })}
									</span>
									{run.skipped > 0 && (
										<span className="text-muted-foreground">
											{t("suiteRunSkippedCount", "{{count}} skipped", {
												count: run.skipped,
											})}
										</span>
									)}
									<span className="tabular-nums text-muted-foreground">
										{formatIsoTime(run.created_at)}
									</span>
								</button>
								{selectedRunId === run.id && (
									<RunDetail
										detail={detailQuery.data}
										loading={detailQuery.isLoading}
										error={
											detailQuery.isError ? messageOf(detailQuery.error) : null
										}
										nodeName={nodeName}
										onOpenLogs={setLogsRunId}
									/>
								)}
							</li>
						))}
					</ul>
				)}
				{logsRunId && suite !== null && (
					<ReplayLogsDialog
						appId={appId}
						boardId={suite.suite.board_id}
						eventId={event.id}
						runId={logsRunId}
						onClose={() => setLogsRunId(null)}
					/>
				)}
			</CardContent>
		</Card>
	);
}

function caseDisplayName(
	caseResult: IRegressionCaseResult,
	nodeName: (nodeId: string) => string,
): string {
	const alias = caseResult.detail?.alias;
	if (typeof alias === "string" && alias.length > 0) return alias;
	if (caseResult.case_kind === "authored_test")
		return nodeName(caseResult.case_ref);
	return caseResult.case_ref;
}

function RunDetail({
	detail,
	loading,
	error,
	nodeName,
	onOpenLogs,
}: Readonly<{
	detail?: IRegressionSuiteRunDetail;
	loading: boolean;
	error: string | null;
	nodeName: (nodeId: string) => string;
	onOpenLogs: (replayRunId: string) => void;
}>) {
	const { t } = useTranslation("settings");
	return (
		<div className="border-t bg-muted/20 px-3 py-2">
			{loading && (
				<div className="flex items-center gap-2 py-2 text-xs text-muted-foreground">
					<Loader2Icon className="h-3.5 w-3.5 animate-spin" />
					{t("suiteRunDetailLoading", "Loading case results…")}
				</div>
			)}
			{error && (
				<p className="py-2 text-xs text-destructive">
					{t("suiteRunDetailFailed", "Could not load the run: {{val}}", {
						val: error,
					})}
				</p>
			)}
			{detail?.run.error && (
				<p className="py-1.5 text-xs text-destructive">{detail.run.error}</p>
			)}
			{detail && detail.cases.length === 0 && !loading && (
				<p className="py-2 text-xs text-muted-foreground">
					{t("suiteRunNoCases", "No case results recorded for this run yet.")}
				</p>
			)}
			{detail && detail.cases.length > 0 && (
				<ul className="flex flex-col divide-y">
					{detail.cases.map((caseResult) => {
						const failedAssertions = Array.isArray(
							caseResult.detail?.failed_assertions,
						)
							? (caseResult.detail?.failed_assertions as string[])
							: [];
						const errorClass =
							typeof caseResult.detail?.error_class === "string"
								? (caseResult.detail?.error_class as string)
								: null;
						const skipReason =
							typeof caseResult.detail?.reason === "string"
								? (caseResult.detail?.reason as string)
								: null;
						return (
							<li key={caseResult.id} className="flex flex-col gap-1 py-1.5">
								<div className="flex items-center gap-2 text-xs">
									<OutcomeChip outcome={caseResult.outcome} />
									<span className="font-medium">
										{caseDisplayName(caseResult, nodeName)}
									</span>
									<span className="text-[11px] text-muted-foreground">
										{caseResult.case_kind === "authored_test"
											? t("caseKindAuthored", "authored test")
											: t("caseKindRecorded", "recorded input")}
									</span>
									{caseResult.detail?.grading_blind === true && (
										<CaveatChip caveat={CAVEAT_GRADING_BLIND} />
									)}
									<span className="flex-1" />
									{typeof caseResult.duration_ms === "number" && (
										<span className="flex items-center gap-1 tabular-nums text-[11px] text-muted-foreground">
											<ClockIcon className="size-3" />
											{(caseResult.duration_ms / 1000).toFixed(1)}s
										</span>
									)}
									{caseResult.replay_run_id && (
										<Button
											size="sm"
											variant="ghost"
											className="h-5 gap-1 px-1.5 text-[11px]"
											onClick={() =>
												caseResult.replay_run_id &&
												onOpenLogs(caseResult.replay_run_id)
											}
										>
											<ScrollTextIcon className="size-3" />
											{t("caseViewLogs", "Logs")}
										</Button>
									)}
								</div>
								{errorClass && caseResult.outcome !== "ok" && (
									<p className="pl-1 text-[11px] text-muted-foreground">
										{errorClass}
									</p>
								)}
								{failedAssertions.map((assertion, index) => (
									<p
										key={`${caseResult.id}-assert-${index.toString()}`}
										className="pl-1 text-[11px] text-destructive"
									>
										{assertion}
									</p>
								))}
								{skipReason && (
									<p className="pl-1 text-[11px] text-muted-foreground">
										{skipReason}
									</p>
								)}
							</li>
						);
					})}
				</ul>
			)}
		</div>
	);
}

/* ------------------------------------------------------------- replay logs */

function ReplayLogsDialog({
	appId,
	boardId,
	eventId,
	runId,
	onClose,
}: Readonly<{
	appId: string;
	boardId: string;
	eventId: string;
	runId: string;
	onClose: () => void;
}>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const [logs, setLogs] = useState<ILog[] | null>(null);
	const [failed, setFailed] = useState(false);

	useEffect(() => {
		let cancelled = false;
		// queryRun only reads run/app/board off the metadata — the rest of the
		// summary is irrelevant for fetching a replay's logs.
		void backend.boardState
			.queryRun(
				{
					run_id: runId,
					app_id: appId,
					board_id: boardId,
					event_id: eventId,
					start: 0,
					end: 0,
					log_level: 0,
					node_id: "",
					payload: [],
					version: "",
				},
				"",
				0,
				500,
			)
			.then((result) => {
				if (!cancelled) setLogs(result);
			})
			.catch(() => {
				if (!cancelled) setFailed(true);
			});
		return () => {
			cancelled = true;
		};
	}, [appId, boardId, eventId, runId, backend.boardState]);

	return (
		<Dialog open onOpenChange={(open) => !open && onClose()}>
			<DialogContent className="max-h-[80vh] sm:max-w-2xl">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<ScrollTextIcon className="h-4 w-4" />
						{t("replayLogsTitle", "Replay logs")}
					</DialogTitle>
					<DialogDescription className="font-mono text-[11px]">
						{runId}
					</DialogDescription>
				</DialogHeader>
				<DialogBody>
					{logs === null && !failed && (
						<div className="flex items-center gap-2 py-6 justify-center text-sm text-muted-foreground">
							<Loader2Icon className="h-4 w-4 animate-spin" />
							{t("replayLogsLoading", "Loading logs…")}
						</div>
					)}
					{failed && (
						<p className="py-6 text-center text-sm text-muted-foreground">
							{t("replayLogsFailed", "The replay's logs could not be loaded.")}
						</p>
					)}
					{logs !== null && logs.length === 0 && (
						<p className="py-6 text-center text-sm text-muted-foreground">
							{t("replayLogsEmpty", "The replay recorded no logs.")}
						</p>
					)}
					{logs !== null && logs.length > 0 && (
						<ul className="max-h-[55vh] space-y-0.5 overflow-auto font-mono text-[11px] leading-snug">
							{logs.map((log, index) => {
								const level = logLevelToNumber(log.log_level);
								return (
									<li
										key={`${runId}-${index.toString()}`}
										className={cn(
											"whitespace-pre-wrap break-words",
											level >= 3
												? "text-destructive"
												: level === 2
													? "text-amber-600 dark:text-amber-500"
													: "text-muted-foreground",
										)}
									>
										{log.message}
									</li>
								);
							})}
						</ul>
					)}
				</DialogBody>
			</DialogContent>
		</Dialog>
	);
}
