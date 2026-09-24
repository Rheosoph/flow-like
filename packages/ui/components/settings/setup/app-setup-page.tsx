"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlertTriangleIcon,
	ArrowRightIcon,
	CalendarClockIcon,
	CheckCircle2Icon,
	LoaderCircleIcon,
	LockIcon,
	MonitorSmartphoneIcon,
	SparklesIcon,
	TrashIcon,
	UsersIcon,
} from "lucide-react";
import { useSearchParams } from "next/navigation";
import { useCallback, useMemo, useState } from "react";
import { useInvoke } from "../../../hooks";
import { RolePermissions } from "../../../lib/permission/role-permission";
import { isRuntimeVariableConfigured } from "../../../lib/runtime-vars-utils";
import {
	IValueType,
	type IVariable,
	IVariableType,
} from "../../../lib/schema/flow/variable";
import { parseUint8ArrayToJson } from "../../../lib/uint8";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import {
	type StoredRuntimeVariable,
	useRuntimeVariables,
} from "../../../state/runtime-variables-context";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import { Progress } from "../../ui/progress";
import { Skeleton } from "../../ui/skeleton";
import { RequirementRow, SharedParameterRow } from "./setup-rows";
import { type DeviceRequirement, useAppSetup } from "./use-app-setup";

type Mode = "setup" | "shared";

/**
 * The app's Setup surface: one place for both the values this person supplies
 * and the app-wide defaults everybody shares.
 *
 * The organising question is "what is stopping this app from running", so the
 * device lane leads and the shared lane sits behind the second segment. The two
 * never share a save path — see `setup-rows.tsx`.
 */
export function AppSetupPage() {
	const { t } = useTranslation("common");
	const searchParams = useSearchParams();
	const appId = searchParams.get("id") ?? "";
	const backend = useBackend();
	const runtimeVars = useRuntimeVariables();

	const setup = useAppSetup(appId);
	const [modeOverride, setModeOverride] = useState<Mode | null>(null);
	const mode: Mode =
		modeOverride ??
		(searchParams.get("mode") === "shared" ? "shared" : "setup");

	const ownRole = useInvoke(
		backend.roleState.getOwnRole,
		backend.roleState,
		[appId],
		appId.length > 0,
	);

	/**
	 * Whether app defaults are writable. Resolved up front rather than
	 * discovered from a rejected save: on desktop a board command commits
	 * locally and only fails later inside the delivery outbox, so trying is not
	 * a way to find out.
	 *
	 * An unreadable role means no hub rather than no permission — a local app
	 * has no permission model at all — so it degrades to writable and lets the
	 * command itself be the authority.
	 */
	const canWriteShared = useMemo(() => {
		const permissions = ownRole.data?.permissions;
		// A restored cache entry can hold an older, non-role shape.
		if (typeof permissions !== "number" || !Number.isInteger(permissions))
			return true;
		return new RolePermissions(BigInt(permissions)).hasPermission(
			RolePermissions.WriteBoards,
		);
	}, [ownRole.data]);

	const goShared = useCallback(() => setModeOverride("shared"), []);
	const goSetup = useCallback(() => setModeOverride("setup"), []);

	const sweepOrphans = useCallback(async () => {
		if (!runtimeVars) return;
		await runtimeVars.deleteValues(
			appId,
			setup.orphans.map((row) => row.variableId),
		);
	}, [runtimeVars, appId, setup.orphans]);

	const { verdict, requirements, sharedGroups, blocking } = setup;
	const pending = requirements.filter((r) => !r.satisfied && !r.waived);
	const done = requirements.filter((r) => r.satisfied);
	const waived = requirements.filter((r) => !r.satisfied && r.waived);

	if (verdict === "empty") {
		return <NothingToSetUp />;
	}

	const showShared = mode === "shared" && verdict !== "loading";

	return (
		<main className="flex max-h-full w-full flex-1 flex-col overflow-hidden">
			<VerdictHeader
				verdict={verdict}
				blocking={blocking.length}
				satisfied={setup.satisfiedCount}
				required={setup.requiredCount}
				progress={setup.progress}
				sharedCount={setup.sharedCount}
				sharedLocked={!canWriteShared}
				mode={mode}
				onMode={setModeOverride}
				onJumpToFirst={() => {
					const el = document.querySelector("[data-first-requirement]");
					el?.scrollIntoView({ behavior: "smooth", block: "center" });
					el?.querySelector("input")?.focus();
				}}
			/>

			<div className="flex flex-1 flex-col gap-6 overflow-y-auto px-1 pb-8 pt-5">
				{verdict === "loading" && <LoadingRows />}

				{verdict === "denied" && (
					<DegradedSetup
						appId={appId}
						stored={setup.stored}
						error={setup.boards.error}
						onWaive={setup.waive}
					/>
				)}

				{verdict !== "loading" && verdict !== "denied" && !showShared && (
					<>
						{pending.length > 0 && (
							<Section
								title={t("neededToRun", "Needed to run")}
								hint={t("staysOnThisDevice", "stays on this device")}
								lane="device"
							>
								{pending.map((requirement, index) => (
									<RequirementRow
										key={requirement.variable.id}
										appId={appId}
										requirement={requirement}
										first={index === 0}
										onWaive={setup.waive}
										onJumpToShared={goShared}
									/>
								))}
							</Section>
						)}

						{done.length > 0 && (
							<Section
								title={t(
									"alreadySetOnThisDevice",
									"Already set on this device",
								)}
								hint={t("countValues", {
									defaultValue_one: "{{count}} value",
									defaultValue_other: "{{count}} values",
									count: done.length,
								})}
								lane="device"
							>
								{done.map((requirement) => (
									<RequirementRow
										key={requirement.variable.id}
										appId={appId}
										requirement={requirement}
										onWaive={setup.waive}
										onJumpToShared={goShared}
									/>
								))}
							</Section>
						)}

						{waived.length > 0 && (
							<Section
								title={t("providedByTheApp", "Provided by the app")}
								hint={t(
									"notCountedTowardsReadiness",
									"not counted towards readiness",
								)}
								lane="device"
							>
								{waived.map((requirement) => (
									<RequirementRow
										key={requirement.variable.id}
										appId={appId}
										requirement={requirement}
										onWaive={setup.waive}
										onJumpToShared={goShared}
									/>
								))}
							</Section>
						)}

						{setup.orphans.length > 0 && (
							<OrphanStrip
								count={setup.orphans.length}
								onSweep={sweepOrphans}
							/>
						)}

						{requirements.length > 0 && <UnattendedRunsNotice />}
					</>
				)}

				{showShared && (
					<>
						<SharedLaneNotice locked={!canWriteShared} onSetup={goSetup} />
						{sharedGroups.map((group) => (
							<Section
								key={group.boardId}
								title={group.boardName}
								hint={t("countParameters", {
									defaultValue_one: "{{count}} parameter",
									defaultValue_other: "{{count}} parameters",
									count: group.variables.length + group.locked.length,
								})}
								lane="shared"
							>
								{group.variables.map((variable) => (
									<SharedParameterRow
										key={variable.id}
										appId={appId}
										boardId={group.boardId}
										variable={variable}
										refs={group.refs}
										disabled={!canWriteShared}
										overridden={group.overriddenIds.has(variable.id)}
										onJumpToSetup={goSetup}
									/>
								))}
								{group.locked.map((variable) => (
									<SharedParameterRow
										key={variable.id}
										appId={appId}
										boardId={group.boardId}
										variable={variable}
										refs={group.refs}
										locked
										onJumpToSetup={goSetup}
									/>
								))}
							</Section>
						))}
						{sharedGroups.length === 0 && (
							<p className="px-4 text-sm text-muted-foreground">
								{t(
									"thisAppHasNoAppwideParametersYet",
									"This app has no app-wide parameters yet.",
								)}
							</p>
						)}
					</>
				)}
			</div>
		</main>
	);
}

function VerdictHeader({
	verdict,
	blocking,
	satisfied,
	required,
	progress,
	sharedCount,
	sharedLocked,
	mode,
	onMode,
	onJumpToFirst,
}: Readonly<{
	verdict: ReturnType<typeof useAppSetup>["verdict"];
	blocking: number;
	satisfied: number;
	required: number;
	progress: number;
	sharedCount: number;
	sharedLocked: boolean;
	mode: Mode;
	onMode: (mode: Mode) => void;
	onJumpToFirst: () => void;
}>) {
	const { t } = useTranslation("common");

	// A green verdict is never rendered while the read is in flight or after it
	// failed. Both tabs this screen replaces showed a confident success panel on
	// a permission error, to the exact role that cannot read boards.
	const copy = {
		loading: {
			title: t("checkingSetup", "Checking setup…"),
			sub: t(
				"readingThisAppsFlowsAndWhatYouHaveAlreadyProvided",
				"Reading this app's flows and what you have already provided.",
			),
		},
		denied: {
			title: t("cannotCheckThisAppsSetup", "Cannot check this app's setup"),
			sub: t(
				"yourRoleCannotReadItsFlowsSoTheListBelowIsIncomplete",
				"Your role cannot read its flows, so the list below is incomplete.",
			),
		},
		blocked: {
			title: t("countThingsNeededBeforeThisAppCanRun", {
				defaultValue_one: "{{count}} thing needed before this app can run",
				defaultValue_other: "{{count}} things needed before this app can run",
				count: blocking,
			}),
			sub: t(
				"theyStayOnThisDeviceNobodyElseSeesThemAndTheyAreNeverUploaded",
				"They stay on this device. Nobody else sees them, and they are never uploaded.",
			),
		},
		ready: {
			title: t("readyToRun", "Ready to run"),
			sub: t("countValuesSetOnThisDevice", {
				defaultValue_one: "{{count}} value set on this device",
				defaultValue_other: "{{count}} values set on this device",
				count: satisfied,
			}),
		},
		empty: { title: "", sub: "" },
	}[verdict];

	const Icon = {
		loading: LoaderCircleIcon,
		denied: LockIcon,
		blocked: AlertTriangleIcon,
		ready: CheckCircle2Icon,
		empty: SparklesIcon,
	}[verdict];

	return (
		<header className="sticky top-0 z-10 flex flex-col gap-3 border-b bg-background/95 pt-5 backdrop-blur supports-backdrop-filter:bg-background/60">
			<div className="flex flex-wrap items-start gap-3">
				<div
					className={cn(
						"grid h-10 w-10 shrink-0 place-items-center rounded-xl border",
						verdict === "blocked" &&
							"border-amber-500/60 bg-amber-500/10 text-amber-600 dark:text-amber-400",
						verdict === "ready" &&
							"border-emerald-500/60 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400",
						verdict === "denied" &&
							"border-destructive/60 bg-destructive/10 text-destructive",
						verdict === "loading" &&
							"border-dashed bg-muted text-muted-foreground",
					)}
				>
					<Icon
						className={cn("h-5 w-5", verdict === "loading" && "animate-spin")}
					/>
				</div>
				<div className="flex min-w-60 flex-1 flex-col">
					<h1 className="text-lg font-semibold tracking-tight">{copy.title}</h1>
					<p className="text-sm text-muted-foreground">{copy.sub}</p>
				</div>
				{verdict === "blocked" && (
					<Button onClick={onJumpToFirst} className="shrink-0">
						{t("setUpNext", "Set up next")}
						<ArrowRightIcon className="ml-1.5 h-4 w-4" />
					</Button>
				)}
			</div>

			{(verdict === "blocked" || verdict === "ready") && required > 0 && (
				<div className="space-y-1">
					<Progress value={progress} className="h-1.5" />
					<p className="text-xs text-muted-foreground">
						{t(
							"satisfiedOfRequiredReady",
							"{{satisfied}} of {{required}} ready",
							{
								satisfied,
								required,
							},
						)}
					</p>
				</div>
			)}

			<div className="-mb-px flex gap-1">
				<button
					type="button"
					role="tab"
					aria-selected={mode === "setup"}
					onClick={() => onMode("setup")}
					className={cn(
						"flex items-center gap-2 border-b-2 px-3 pb-2.5 pt-1 text-sm font-medium transition-colors",
						mode === "setup"
							? "border-primary text-foreground"
							: "border-transparent text-muted-foreground hover:text-foreground",
					)}
				>
					<MonitorSmartphoneIcon className="h-4 w-4" />
					{t("setup", "Setup")}
				</button>
				{sharedCount > 0 && (
					<button
						type="button"
						role="tab"
						aria-selected={mode === "shared"}
						onClick={() => onMode("shared")}
						className={cn(
							"flex items-center gap-2 border-b-2 px-3 pb-2.5 pt-1 text-sm font-medium transition-colors",
							mode === "shared"
								? "border-primary text-foreground"
								: "border-transparent text-muted-foreground hover:text-foreground",
						)}
					>
						<UsersIcon className="h-4 w-4" />
						{t("appDefaults", "App defaults")}
						<Badge variant="secondary" className="px-1.5 font-mono text-[10px]">
							{sharedCount}
						</Badge>
						{sharedLocked && (
							<LockIcon className="h-3 w-3 text-muted-foreground" />
						)}
					</button>
				)}
			</div>
		</header>
	);
}

function Section({
	title,
	hint,
	lane,
	children,
}: Readonly<{
	title: string;
	hint?: string;
	lane: "device" | "shared";
	children: React.ReactNode;
}>) {
	const { t } = useTranslation("common");
	return (
		<section className="flex flex-col gap-2">
			<div className="flex flex-wrap items-baseline gap-x-3 gap-y-1 px-1">
				<h2 className="text-xs font-semibold uppercase tracking-wider">
					{title}
				</h2>
				{hint && <span className="text-xs text-muted-foreground">{hint}</span>}
				<span className="ml-auto">
					{lane === "device" ? (
						<Badge
							variant="outline"
							className="gap-1.5 border-violet-500/30 text-xs text-violet-600 dark:text-violet-400"
						>
							<MonitorSmartphoneIcon className="h-3 w-3" />
							{t("onThisDevice", "On this device")}
						</Badge>
					) : (
						<Badge
							variant="outline"
							className="gap-1.5 border-sky-500/30 text-xs text-sky-600 dark:text-sky-400"
						>
							<UsersIcon className="h-3 w-3" />
							{t("sharedWithEveryone", "Shared with everyone")}
						</Badge>
					)}
				</span>
			</div>
			{/* The docs screenshot plan selects rows as `.divide-y > div:nth-child(n)`. */}
			<div className="divide-y overflow-hidden rounded-xl border bg-card">
				{children}
			</div>
		</section>
	);
}

function LoadingRows() {
	return (
		<div className="divide-y overflow-hidden rounded-xl border bg-card">
			{[0, 1, 2].map((row) => (
				<div key={row} className="space-y-2 p-4">
					<Skeleton className="h-4 w-44" />
					<Skeleton className="h-3 w-2/3" />
					<Skeleton className="h-9 w-72" />
				</div>
			))}
		</div>
	);
}

function OrphanStrip({
	count,
	onSweep,
}: Readonly<{ count: number; onSweep: () => void }>) {
	const { t } = useTranslation("common");
	return (
		<div className="flex flex-wrap items-center gap-3 rounded-xl border border-dashed p-3 text-sm text-muted-foreground">
			<TrashIcon className="h-4 w-4 shrink-0" />
			<span>
				{t("countStoredValuesBelongToVariablesThisAppNoLongerHas", {
					defaultValue_one:
						"{{count}} stored value belongs to a variable this app no longer has",
					defaultValue_other:
						"{{count}} stored values belong to variables this app no longer has",
					count,
				})}
			</span>
			<Button variant="outline" size="sm" className="ml-auto" onClick={onSweep}>
				{t("removeThem", "Remove them")}
			</Button>
		</div>
	);
}

function UnattendedRunsNotice() {
	const { t } = useTranslation("common");
	return (
		<div className="flex items-start gap-3 rounded-xl border border-amber-500/40 bg-amber-500/10 p-4">
			<CalendarClockIcon className="mt-0.5 h-4 w-4 shrink-0 text-amber-600 dark:text-amber-400" />
			<div className="space-y-1">
				<p className="text-sm font-medium">
					{t(
						"unattendedRunsDoNotUseTheseValues",
						"Unattended runs do not use these values",
					)}
				</p>
				<p className="max-w-prose text-xs text-muted-foreground">
					{t(
						"aScheduleAWebhookOrAnMcpCallHasNobodyToAskSoItReadsTheAppDefaultInsteadGiveThoseFlowsAnAppDefaultOrSetAPerTriggerValueOnTheEventsPage",
						"A schedule, a webhook or an MCP call has nobody to ask, so it reads the app default instead. Give those flows an app default, or set a per-trigger value on the Events page.",
					)}
				</p>
			</div>
		</div>
	);
}

function SharedLaneNotice({
	locked,
	onSetup,
}: Readonly<{ locked: boolean; onSetup: () => void }>) {
	const { t } = useTranslation("common");
	if (locked) {
		return (
			<div className="flex items-start gap-3 rounded-xl border border-amber-500/40 bg-amber-500/10 p-4">
				<LockIcon className="mt-0.5 h-4 w-4 shrink-0 text-amber-600 dark:text-amber-400" />
				<div className="space-y-1">
					<p className="text-sm font-medium">
						{t("readonlyForYou", "Read-only for you")}
					</p>
					<p className="max-w-prose text-xs text-muted-foreground">
						{t(
							"changingAnAppDefaultNeedsPermissionToEditFlowsYouCanStillSetYourOwnValues",
							"Changing an app default needs permission to edit flows. You can still set your own values.",
						)}{" "}
						<button
							type="button"
							onClick={onSetup}
							className="font-medium underline underline-offset-2"
						>
							{t("goToSetup", "Go to Setup")}
						</button>
					</p>
				</div>
			</div>
		);
	}
	return (
		<div className="flex items-start gap-3 rounded-xl border bg-muted/40 p-4">
			<UsersIcon className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
			<div className="space-y-1">
				<p className="text-sm font-medium">
					{t("valuesSavedIntoTheAppItself", "Values saved into the app itself")}
				</p>
				<p className="max-w-prose text-xs text-muted-foreground">
					{t(
						"changingOneChangesItForEveryMemberAndEveryScheduledRunItCountsAsAFlowEditTriggersPinnedToAPublishedVersionKeepTheOldValueUntilYouPublishAgain",
						"Changing one changes it for every member and every scheduled run. It counts as a flow edit. Triggers pinned to a published version keep the old value until you publish again.",
					)}
				</p>
			</div>
		</div>
	);
}

function NothingToSetUp() {
	const { t } = useTranslation("common");
	return (
		<main className="flex w-full flex-1 flex-col items-center justify-center p-8">
			<div className="flex max-w-md flex-col items-center gap-6 text-center">
				<div className="grid h-20 w-20 place-items-center rounded-2xl bg-emerald-500/10">
					<CheckCircle2Icon className="h-10 w-10 text-emerald-500" />
				</div>
				<div className="space-y-2">
					<h1 className="text-2xl font-semibold">
						{t("nothingToSetUp", "Nothing to set up")}
					</h1>
					<p className="text-muted-foreground">
						{t(
							"thisAppDoesNotAskForAnyValuesSoItIsReadyToRunAsItIs",
							"This app does not ask for any values, so it is ready to run as it is.",
						)}
					</p>
				</div>
				<div className="space-y-2 rounded-lg bg-muted/50 p-4 text-left text-sm text-muted-foreground">
					<p className="font-medium text-foreground">
						{t("howToAddSomething", "How to add something here")}
					</p>
					<p>
						{t(
							"inTheFlowEditorMarkAVariableExposedToGiveEveryoneASharedDefaultOrRuntimeConfiguredOrSecretToAskEachPersonForTheirOwnValue",
							'In the flow editor, mark a variable "Exposed" to give everyone a shared default, or "Runtime Configured" or "Secret" to ask each person for their own value.',
						)}
					</p>
				</div>
			</div>
		</main>
	);
}

/**
 * What is still usable when the board read fails.
 *
 * The stored rows already carry the variable's name, its flow and whether it is
 * secret, so someone whose role cannot read flows can still see and edit what
 * this app previously asked them for. The declaration is gone, so the type is
 * inferred from the stored JSON and anything structured stays read-only rather
 * than being re-encoded as a string.
 */
function DegradedSetup({
	appId,
	stored,
	error,
	onWaive,
}: Readonly<{
	appId: string;
	stored: StoredRuntimeVariable[];
	error: Error | null;
	onWaive: (variableId: string, waived: boolean) => void;
}>) {
	const { t } = useTranslation("common");
	const rebuilt = useMemo(
		() =>
			stored
				.map(reconstructRequirement)
				.filter((row): row is DeviceRequirement => row !== null),
		[stored],
	);

	return (
		<>
			<div className="flex items-start gap-3 rounded-xl border border-destructive/40 bg-destructive/10 p-4">
				<LockIcon className="mt-0.5 h-4 w-4 shrink-0 text-destructive" />
				<div className="space-y-1">
					<p className="text-sm font-medium">
						{t(
							"theFullListOfSettingsIsUnavailable",
							"The full list of settings is unavailable",
						)}
					</p>
					<p className="max-w-prose text-xs text-muted-foreground">
						{t(
							"belowIsWhatThisAppHasAskedYouForBeforeReconstructedFromWhatIsStoredOnThisDeviceAskAnOwnerForFlowReadAccessToSeeEverything",
							"Below is what this app has asked you for before, reconstructed from what is stored on this device. Ask an owner for flow read access to see everything.",
						)}
					</p>
					{error?.message && (
						<p className="font-mono text-xs text-muted-foreground">
							{error.message}
						</p>
					)}
				</div>
			</div>

			{rebuilt.length > 0 ? (
				<Section
					title={t(
						"previouslyAskedForOnThisDevice",
						"Previously asked for on this device",
					)}
					hint={t("reconstructed", "reconstructed")}
					lane="device"
				>
					{rebuilt.map((requirement) => (
						<RequirementRow
							key={requirement.variable.id}
							appId={appId}
							requirement={requirement}
							onWaive={onWaive}
							onJumpToShared={() => undefined}
						/>
					))}
				</Section>
			) : (
				<p className="px-1 text-sm text-muted-foreground">
					{t(
						"thisDeviceHoldsNoValuesForThisApp",
						"This device holds no values for this app.",
					)}
				</p>
			)}
		</>
	);
}

function reconstructRequirement(
	row: StoredRuntimeVariable,
): DeviceRequirement | null {
	const decoded = parseUint8ArrayToJson(row.value);
	let dataType: IVariableType;
	if (typeof decoded === "string") dataType = IVariableType.String;
	else if (typeof decoded === "boolean") dataType = IVariableType.Boolean;
	else if (typeof decoded === "number")
		dataType = Number.isInteger(decoded)
			? IVariableType.Integer
			: IVariableType.Float;
	else return null;

	const variable: IVariable = {
		id: row.variableId,
		name: row.variableName,
		data_type: dataType,
		value_type: IValueType.Normal,
		default_value: row.value,
		secret: row.isSecret,
		exposed: false,
		editable: true,
		runtime_configured: true,
	};

	return {
		variable,
		boardId: row.boardId,
		boardName: "",
		refs: undefined,
		seeded: variable,
		stored: row,
		satisfied: isRuntimeVariableConfigured(variable),
		waived: false,
		alsoShared: false,
	};
}
