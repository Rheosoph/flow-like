"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Box,
	Check,
	ChevronDown,
	ChevronRight,
	CircleArrowUp,
	GitCommitHorizontal,
	LayoutGrid,
	Settings,
	Sparkles,
} from "lucide-react";
import { type ReactNode, useMemo, useState } from "react";
import { useInvoke } from "../../../../../hooks/use-invoke";
import type { BotFacts } from "../../../../../lib/device-management/bot-config";
import type { EventEligibility } from "../../../../../lib/device-management/deployment";
import {
	type AppEventInput,
	type AppMode,
	type AppVisibility,
	appEventRule,
	appMode,
	isClaimedKind,
} from "../../../../../lib/device-management/model/app-plan";
import {
	type DeployDraft,
	type PlanTarget,
	wholeAppEvents,
} from "../../../../../lib/device-management/model/deploy-plan";
import { whereOf } from "../../../../../lib/device-management/model/schedule-where";
import {
	nextRuns,
	onceAhead,
} from "../../../../../lib/device-management/schedule";
import { useBackend } from "../../../../../state/backend-state";
import { appCopy } from "../../copy/app-copy";
import {
	eligibilityCopy,
	eligibilityFixLabel,
	eligibilityInput,
	eventRunsCopy,
	eventTypeLabel,
	howItRunsCopy,
} from "../../copy/eligibility-copy";
import {
	scheduleTime,
	scheduleWhereText,
	scheduleZone,
	whereNames,
} from "../../copy/schedule-copy";
import { ModeChip, VisibilityChip } from "../../primitives/app-chips";
import {
	type DevicesT,
	useAreaPrefs,
	useAreaTime,
} from "../../primitives/area-context";
import { Block } from "../../primitives/block";
import { DvButton } from "../../primitives/dv-button";
import { DvSheet } from "../../primitives/dv-sheet";
import {
	CellSub,
	DvTable,
	GroupRow,
	Td,
	Th,
	Tr,
} from "../../primitives/dv-table";
import { LatestTag } from "../../primitives/event-cell";
import {
	CheckField,
	ChoiceCards,
	type ChoiceOption,
} from "../../primitives/form-fields";
import { GateInline } from "../../primitives/gate-notice";
import { ModeExplainer } from "../../primitives/how-runs";
import { IdRef } from "../../primitives/id-ref";
import { Segmented } from "../../primitives/segmented";
import { StateView } from "../../primitives/state-view";
import { cx } from "../../primitives/tone";
import { WizardStepHeader } from "../../primitives/wizard";
import { appEventsHref } from "../../routing/devices-href";
import { useDevicesRoute, useHostLink } from "../../routing/use-devices-route";
import { eventName, issueText, noFlowsText, planNames } from "../deploy-copy";
import {
	EventTile,
	HeadChip,
	HubStamp,
	LocalStamp,
	TargetsStamp,
} from "../deploy-parts";
import { ServicePlan } from "../service-plan";
import type { PlanStepProps } from "../step-props";
import { useUnreadableApps } from "../use-deploy-reads";

/* Step 1 · What (APP §3.5): the app, its version, the events and the services they become. */

interface EventRow {
	event: AppEventInput;
	rule: EventEligibility;
	/** Devices that serve it now, as far as this computer can read. */
	servedOn: number;
}

function initials(name: string): string {
	return name
		.split(/\s+/)
		.map((word) => word.charAt(0))
		.join("")
		.slice(0, 2)
		.toUpperCase();
}

const versionOf = (value: readonly number[] | null | undefined) =>
	value ? value.join(".") : null;

function useEventRows({ state }: PlanStepProps): {
	ok: EventRow[];
	cant: EventRow[];
} {
	const { app, appRead, facts } = state;
	const { hub } = facts;
	return useMemo(() => {
		const served = new Map<string, number>(
			(appRead.view?.events.rows ?? []).map((row) => [
				row.eventId,
				Object.values(row.cells).filter((cell) => cell.state === "served")
					.length,
			]),
		);
		const rows = (app?.events ?? []).map((event) => ({
			event,
			rule: appEventRule(event, hub),
			servedOn: served.get(event.id) ?? 0,
		}));
		return {
			ok: rows.filter((row) => row.rule.eligible),
			cant: rows.filter((row) => !row.rule.eligible),
		};
	}, [app, appRead.view, hub]);
}

function ExplainSheet({
	app,
	mode,
	open,
	onOpenChange,
}: Readonly<{
	app: string;
	mode: AppMode;
	open: boolean;
	onOpenChange(open: boolean): void;
}>) {
	const { t } = useTranslation("devices");
	const copy = appCopy(t);
	return (
		<DvSheet
			open={open}
			onOpenChange={onOpenChange}
			wide
			title={copy.explainerTitle()}
			sub={copy.explainerSub(mode, app)}
		>
			<ModeExplainer app={app} mode={mode} />
		</DvSheet>
	);
}

function AppIdentity(props: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { app, versionLabel, mode } = props.state;
	const [explain, setExplain] = useState(false);
	if (!app || !mode) return null;
	return (
		<div className="flex min-w-0 items-start gap-3">
			<span
				aria-hidden
				className="inline-flex size-9 shrink-0 items-center justify-center rounded-lg border border-border bg-surface-sunken text-ui font-semibold text-ink-2"
			>
				{initials(app.name)}
			</span>
			<div className="flex min-w-0 flex-col gap-1">
				<p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-ui">
					<b className="text-[15px]/5 font-semibold">{app.name}</b>
					<VisibilityChip visibility={app.visibility} />
					<ModeChip mode={mode} app={app.name} />
				</p>
				{versionLabel ? (
					<p className="text-xs text-muted-foreground">
						{t("deploy.what.newestIs", "Newest")}{" "}
						<span className="font-mono text-foreground">{versionLabel}</span>
					</p>
				) : null}
				<p className="flex flex-wrap items-center gap-x-1.5 text-xs text-muted-foreground">
					<IdRef
						id={app.id}
						label={t("deploy.what.appId", "App ID")}
						copyLabel={t("deploy.what.copyAppId", "Copy app ID")}
					/>
					<span aria-hidden>·</span>
					<DvButton variant="link" size="xs" onClick={() => setExplain(true)}>
						{appCopy(t).explainButton()}
					</DvButton>
				</p>
			</div>
			<ExplainSheet
				app={app.name}
				mode={mode}
				open={explain}
				onOpenChange={setExplain}
			/>
		</div>
	);
}

const APP_CAP = 12;

/** Device-first: the apps you can deploy, as choice cards; one your role can't read is listed, not offered. */
function AppPicker({ route, state }: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const backend = useBackend();
	const { navigate } = useDevicesRoute();
	const apps = useInvoke(backend.appState.getApps, backend.appState, []);
	const [all, setAll] = useState(false);
	const copy = appCopy(t);
	const chosen = state.draft.appId ?? undefined;
	const rows = apps.data;
	const count = rows?.length ?? 0;
	// The chosen app is never hidden behind "Show more".
	const beyond =
		(rows?.findIndex(([row]) => row.id === chosen) ?? -1) >= APP_CAP;
	const shown = useMemo(
		() => (all || beyond ? rows : rows?.slice(0, APP_CAP)) ?? [],
		[rows, all, beyond],
	);
	const ids = useMemo(() => shown.map(([row]) => row.id), [shown]);
	const unreadable = useUnreadableApps(ids);
	if (apps.isLoading) return <StateView kind="loading" rows={3} />;
	if (apps.error)
		return (
			<StateView
				kind="error"
				title={t("deploy.what.appsError", "Couldn't load your apps")}
				text={apps.error.message}
			/>
		);
	const options: ChoiceOption<string>[] = shown.map(([row, meta]) => {
		const visibility = row.visibility as AppVisibility;
		const name = meta?.name ?? row.id;
		const locked = unreadable.has(row.id);
		return {
			value: row.id,
			disabled: locked,
			title: (
				<span className="flex flex-wrap items-center gap-x-2 gap-y-1">
					{name}
					<VisibilityChip visibility={visibility} />
					<ModeChip mode={appMode(visibility)} app={name} />
				</span>
			),
			hint: locked
				? noFlowsText(t, name)
				: copy.mode(appMode(visibility), name).sentence,
		};
	});
	if (!options.length)
		return (
			<StateView
				kind="empty"
				title={t("deploy.what.noApps", "You have no apps yet")}
				text={t(
					"deploy.what.noAppsText",
					"Create an app first; then deploy it to this device.",
				)}
			/>
		);
	return (
		<>
			<ChoiceCards
				id="deploy-app"
				legend={
					<span className="sr-only">
						{t("deploy.what.appLegend", "App to deploy")}
					</span>
				}
				value={chosen}
				onValueChange={(appId) =>
					navigate({ ...route, appId }, { replace: true })
				}
				options={options}
			/>
			{count > APP_CAP && !beyond ? (
				<DvButton
					size="sm"
					variant="ghost"
					className="w-fit"
					onClick={() => setAll(!all)}
				>
					{all
						? t("deploy.what.showFewerApps", "Show fewer")
						: t("deploy.what.showMoreApps", {
								count: count - APP_CAP,
								defaultValue_one: "Show {{count, number}} more app",
								defaultValue_other: "Show {{count, number}} more apps",
							})}
				</DvButton>
			) : null}
		</>
	);
}

function AppBlock(props: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { state, scope } = props;
	const fixed = scope.kind === "app" || state.draft.entry === "update";
	const local = state.mode === "offline";
	const [explain, setExplain] = useState(false);
	return (
		<Block
			title={t("deploy.what.app", "App")}
			icon={LayoutGrid}
			stamp={
				local ? (
					<LocalStamp
						text={t("deploy.stamp.onThisComputer", "on this computer")}
					/>
				) : (
					<HubStamp />
				)
			}
			foot={
				fixed ? undefined : (
					<>
						{t(
							"deploy.what.appFoot",
							"How an app runs on a device follows the app: online apps run online, local-only apps get an offline copy.",
						)}
						{state.app && state.mode ? (
							<DvButton
								variant="link"
								size="xs"
								onClick={() => setExplain(true)}
							>
								{appCopy(t).explainButton()}
							</DvButton>
						) : null}
					</>
				)
			}
		>
			{fixed ? <AppIdentity {...props} /> : <AppPicker {...props} />}
			{!fixed && state.app && state.mode ? (
				<ExplainSheet
					app={state.app.name}
					mode={state.mode}
					open={explain}
					onOpenChange={setExplain}
				/>
			) : null}
		</Block>
	);
}

function VersionLine({ state, prepared }: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const local = state.mode === "offline";
	const hash =
		prepared?.approved?.sha256 ?? prepared?.artifact.descriptor.manifest_sha256;
	return (
		<p className="flex flex-wrap items-center gap-x-1.5 gap-y-1 text-ui">
			<Check aria-hidden className="size-3.5 text-good" />
			<span>{t("deploy.what.newest", "Newest:")}</span>
			{state.versionLabel ? (
				<b className="font-mono font-semibold">{state.versionLabel}</b>
			) : null}
			{hash ? (
				<IdRef
					id={hash}
					copyLabel={t("deploy.what.copyHash", "Copy app version hash")}
				/>
			) : null}
			<span className="text-ink-2">
				{state.versionLabel || hash ? <span aria-hidden>· </span> : null}
				{local
					? t(
							"deploy.what.newestLocal",
							"the app as it is on this computer now",
						)
					: t("deploy.what.newestOnline", "what's published now")}
			</span>
		</p>
	);
}

function VersionBlock(props: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { state, update } = props;
	const { draft, mode } = state;
	if (!mode) return null;
	const copy = appCopy(t);
	const updating = draft.entry === "update";
	return (
		<Block
			title={t("deploy.what.version", "Version")}
			icon={GitCommitHorizontal}
			stamp={
				mode === "offline" ? (
					<LocalStamp
						text={t("deploy.stamp.onThisComputer", "on this computer")}
					/>
				) : (
					<HubStamp />
				)
			}
		>
			{updating ? (
				<ChoiceCards<DeployDraft["version"]>
					id="deploy-version"
					legend={
						<span className="sr-only">
							{t("deploy.what.version", "Version")}
						</span>
					}
					value={draft.version}
					onValueChange={(version) => update({ version })}
					className="[&_[role=radiogroup]]:grid-cols-2 @max-[560px]/whatstep:[&_[role=radiogroup]]:grid-cols-1"
					options={[
						{
							value: "newest",
							icon: CircleArrowUp,
							title: state.versionLabel
								? t("deploy.what.newestLabel", "Newest {{version}}", {
										version: state.versionLabel,
									})
								: t("deploy.what.newestPlain", "Newest version"),
							hint:
								mode === "offline"
									? t(
											"deploy.what.newestHintLocal",
											"The app as it is on this computer now.",
										)
									: t("deploy.what.newestHint", "What's published now."),
						},
						{
							value: "keep",
							icon: Settings,
							title: t("deploy.what.keep", "Keep each service's version"),
							hint: t(
								"deploy.what.keepHint",
								"Change settings or events only; nothing is uploaded.",
							),
						},
					]}
				/>
			) : (
				<VersionLine {...props} />
			)}
			<p className="text-xs text-muted-foreground">
				{updating
					? t(
							"deploy.what.olderVersions",
							"Older versions can't be prepared again.",
						)
					: t(
							"deploy.what.versionPins",
							"An app version pins one version of each event and its flow. An event that follows Latest is deployed as the flow is at that moment.",
						)}{" "}
				{copy.publishNote(mode)}
			</p>
		</Block>
	);
}

interface EventRowProps {
	row: EventRow;
	checked: boolean;
	locked?: boolean;
	appId: string;
	state: PlanStepProps["state"];
	/** Third column: versions and how it runs (new) or what it does now (update). */
	now?: ReactNode;
	onToggle(on: boolean): void;
}

function EventNameCell({
	row,
	checked,
	locked = false,
	onToggle,
}: Readonly<EventRowProps>) {
	const { showTechnicalKeys } = useAreaPrefs();
	const { event, rule } = row;
	return (
		<CheckField
			id={`deploy-event-${event.id}`}
			checked={checked}
			disabled={!rule.eligible || locked}
			onCheckedChange={onToggle}
			className="[&_label]:opacity-100!"
		>
			<span className="flex min-w-0 items-start gap-2">
				<EventTile
					eventType={event.event_type}
					hasPage={Boolean(event.default_page_id)}
				/>
				<span className="flex min-w-0 flex-col">
					<b className="font-semibold">{event.name}</b>
					{showTechnicalKeys ? (
						<span className="font-mono text-xs text-muted-foreground">
							{event.id}
						</span>
					) : null}
				</span>
			</span>
		</CheckField>
	);
}

/**
 * The pins of one event. An event that follows Latest has no flow pin: the
 * deploy takes the flow as it is then, so the row says which version that is,
 * or that Preparing creates one.
 */
function VersionsCell({ row }: Readonly<{ row: EventRow }>) {
	const { t } = useTranslation("devices");
	const { rule, event } = row;
	const flow = typeof event.flow === "object" ? event.flow : null;
	return (
		<>
			<span className="font-mono text-xs">
				{rule.followsLatest
					? appCopy(t).pinLatest(versionOf(rule.eventVersion) ?? "–")
					: t("deploy.what.pins", "event {{event}} · flow {{flow}}", {
							event: versionOf(rule.eventVersion) ?? "–",
							flow: versionOf(rule.boardVersion) ?? "–",
						})}
			</span>
			{rule.followsLatest ? (
				<CellSub data-latest={flow?.current ? "version" : "edits"}>
					<LatestTag className="mr-1.5" />
					{flow?.current
						? t("deploy.what.deploysFlow", "Deploys flow {{version}}.", {
								version: versionOf(flow.current),
							})
						: flow
							? t(
									"deploy.what.createsFlow",
									"Preparing creates a flow version from the current edits. It stays in the flow's history.",
								)
							: null}
				</CellSub>
			) : null}
		</>
	);
}

/** A schedule: when it runs, that the device starts it, its next time and where it runs today. */
function ScheduleLines({
	row,
	state,
	sub = false,
}: Readonly<{
	row: EventRow;
	state: PlanStepProps["state"];
	/** Every line under another one, the first included. */
	sub?: boolean;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { schedule } = row.rule;
	if (!schedule) return null;
	const when = eventRunsCopy(t, row.rule);
	const [next] = nextRuns(schedule, time.now, 1);
	const where = whereOf(state.facts.schedules, row.event.id);
	const today =
		state.mode === "offline"
			? t(
					"deploy.what.scheduleLocal",
					"Runs in the desktop app while it is open. On a device it runs without a computer.",
				)
			: where?.fact === "hub"
				? t(
						"deploy.what.scheduleHub",
						"Runs on the hub today. Tick it to run it on the device instead.",
					)
				: where
					? scheduleWhereText(t, where, whereNames(t, deviceNames(state), time))
					: null;
	return (
		<>
			{sub ? <CellSub>{when}</CellSub> : when}
			<CellSub>{howItRunsCopy(t, row.rule)}</CellSub>
			{next === undefined ? null : (
				<CellSub data-schedule-next="">
					{t("deploy.what.scheduleNext", "Next run by its schedule: {{time}}", {
						time: scheduleTime(
							t,
							Math.floor(next / 1000),
							schedule.timezone,
							time,
						),
					})}
				</CellSub>
			)}
			{today ? (
				<CellSub data-schedule-where={where?.fact ?? "local"}>{today}</CellSub>
			) : null}
		</>
	);
}

/** An Endpoint: its method and path, and who may call it on a device. */
function EndpointLines({ row }: Readonly<{ row: EventRow }>) {
	const { t } = useTranslation("devices");
	const { rule, event } = row;
	if (!rule.route) return null;
	return (
		<>
			<span data-route="" className="font-mono text-xs">
				{t("deploy.what.endpoint", "{{method}} {{path}}", {
					method: rule.route.method,
					path: rule.route.path,
				})}
			</span>
			<CellSub>{howItRunsCopy(t, rule)}</CellSub>
			{event.ownToken === undefined ? null : (
				<CellSub data-endpoint-token={event.ownToken ? "own" : "open"}>
					{event.ownToken
						? t(
								"deploy.what.endpointToken",
								"Its own token from Events is not used on a device.",
							)
						: t(
								"deploy.what.endpointOpen",
								"Open on the hub. On a device callers need the service's access token.",
							)}
				</CellSub>
			)}
		</>
	);
}

/** A form or quick action: a person starts it, with its fields; a file field needs a service page. */
function OnDemandLines({ row }: Readonly<{ row: EventRow }>) {
	const { t } = useTranslation("devices");
	const form = row.event.form;
	const count = form?.fields ?? 0;
	return (
		<>
			{count > 0
				? t("deploy.what.onDemandForm", {
						count,
						defaultValue_one: "Started by a person · {{count, number}} field",
						defaultValue_other:
							"Started by a person · {{count, number}} fields",
					})
				: t("deploy.what.onDemand", "Started by a person")}
			{form?.fileFields ? (
				<CellSub data-file-field="">
					{t(
						"deploy.what.onDemandFile",
						"It takes a file: only a service page can send one.",
					)}
				</CellSub>
			) : null}
		</>
	);
}

/** A one-time schedule: when it runs, whether that time has passed, and where it runs today. */
function OnceLines({
	row,
	state,
	sub = false,
}: Readonly<{ row: EventRow; state: PlanStepProps["state"]; sub?: boolean }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { once } = row.rule;
	if (!once) return null;
	const when = t(
		"deploy.what.once",
		"Runs once on {{date}} at {{time}} ({{zone}})",
		{ date: once.date, time: once.time, zone: scheduleZone(t, once) },
	);
	const where = whereOf(state.facts.schedules, row.event.id);
	// A one-time schedule never reads "runs on the hub": only some hubs run it, once.
	const today =
		where && where.fact !== "hub"
			? scheduleWhereText(
					t,
					where,
					whereNames(t, deviceNames(state), time),
					"once",
				)
			: null;
	return (
		<>
			{sub ? <CellSub>{when}</CellSub> : when}
			{onceAhead(once, time.now) ? null : (
				<CellSub data-once-passed="" className="text-critical">
					{t(
						"deploy.what.oncePassed",
						"Its time has passed. Set a new time in Events.",
					)}
				</CellSub>
			)}
			<CellSub>{howItRunsCopy(t, row.rule)}</CellSub>
			{today ? (
				<CellSub data-schedule-where={where?.fact}>{today}</CellSub>
			) : null}
		</>
	);
}

const deviceNames = (state: PlanStepProps["state"]) =>
	new Map(state.devices.map((device) => [device.id, device.name]));

/** A local-only app's bot: a device the viewer sees whose service of the app runs it. */
function localHolder(state: PlanStepProps["state"], eventId: string) {
	const isIt = (event: { event_id: string }) => event.event_id === eventId;
	for (const device of state.devices)
		for (const service of device.services ?? []) {
			const runs = service.events?.some(isIt) ?? false;
			if (runs && service.projectId === state.draft.appId)
				return { device: device.name, service: service.serviceId };
		}
	return null;
}

/** Where a bot runs today: this computer, a device, or nowhere. Unknown while the hub's list loads. */
function botToday(
	t: DevicesT,
	row: EventRow,
	state: PlanStepProps["state"],
	time: ReturnType<typeof useAreaTime>,
): { at: string; text: string } | null {
	const id = row.event.id;
	const nowhere = {
		at: "nowhere",
		text: t(
			"deploy.what.botNowhere",
			"No device runs it. Tick it to keep it connected from the device.",
		),
	};
	if (state.facts.localTriggers?.includes(id))
		return {
			at: "here",
			text: t(
				"deploy.what.botHere",
				"Runs in the desktop app on this computer today. Tick it to keep it connected from the device instead.",
			),
		};
	if (state.mode === "offline") {
		const holder = localHolder(state, id);
		return holder
			? {
					at: "device",
					text: t(
						"deploy.what.botDevice",
						"Connected from {{device}} › {{service}} today.",
						holder,
					),
				}
			: nowhere;
	}
	const where = whereOf(state.facts.schedules, id);
	if (!where) return null;
	return where.fact === "hub"
		? nowhere
		: {
				at: where.fact,
				text: scheduleWhereText(
					t,
					where,
					whereNames(t, deviceNames(state), time),
					"bot",
				),
			};
}

/**
 * What a bot answers in groups and servers (§5.5). A Telegram bot: mentions and
 * replies, its prefix, both or nothing. A Discord bot reads no prefix, like the
 * desktop app's: mentions and replies, or every message.
 */
function botAnswersText(t: DevicesT, bot: BotFacts): string {
	if (bot.provider === "discord" && !bot.mentions)
		return t(
			"deploy.what.botAnswersEvery",
			"In servers it answers every message in its channels, not only mentions and replies.",
		);
	if (bot.mentions)
		return bot.prefix
			? t(
					"deploy.what.botAnswersPrefix",
					"In groups and servers it answers mentions, replies and every message that starts with {{prefix}}.",
					{ prefix: bot.prefix },
				)
			: t(
					"deploy.what.botAnswers",
					"In groups and servers it answers mentions and replies.",
				);
	return bot.prefix
		? t(
				"deploy.what.botAnswersPrefixOnly",
				"In groups and servers it answers every message that starts with {{prefix}}.",
				{ prefix: bot.prefix },
			)
		: t(
				"deploy.what.botAnswersNone",
				"In groups and servers it answers nothing; only private messages start runs.",
			);
}

/** A Telegram or Discord bot: where it runs today, what it answers in groups, and that it stays connected. */
function BotLines({
	row,
	state,
	sub = false,
}: Readonly<{ row: EventRow; state: PlanStepProps["state"]; sub?: boolean }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { bot } = row.rule;
	if (!bot) return null;
	const runs = howItRunsCopy(t, row.rule);
	const today = botToday(t, row, state, time);
	return (
		<>
			{sub ? <CellSub>{runs}</CellSub> : runs}
			{today ? <CellSub data-bot-where={today.at}>{today.text}</CellSub> : null}
			<CellSub data-bot-answers="">{botAnswersText(t, bot)}</CellSub>
		</>
	);
}

/** How the event runs on a device, in the words of its kind. */
function KindLines({
	row,
	state,
	sub = false,
}: Readonly<{ row: EventRow; state: PlanStepProps["state"]; sub?: boolean }>) {
	const { t } = useTranslation("devices");
	const { rule } = row;
	if (rule.route) return <EndpointLines row={row} />;
	if (rule.once) return <OnceLines row={row} state={state} sub={sub} />;
	if (rule.schedule) return <ScheduleLines row={row} state={state} sub={sub} />;
	if (rule.bot) return <BotLines row={row} state={state} sub={sub} />;
	if (rule.kind === "on_demand") return <OnDemandLines row={row} />;
	return <>{howItRunsCopy(t, rule)}</>;
}

function OnDeviceCell({
	row,
	appId,
	state,
}: Readonly<
	Pick<EventRowProps, "row" | "appId"> & { state: PlanStepProps["state"] }
>) {
	const { t } = useTranslation("devices");
	const hostLink = useHostLink();
	const { event, rule, servedOn } = row;
	if (rule.eligible)
		return (
			<>
				<KindLines row={row} state={state} />
				{servedOn > 0 ? (
					<CellSub>
						{t("deploy.what.runsOn", {
							count: servedOn,
							defaultValue_one: "Runs on {{count, number}} device now",
							defaultValue_other: "Runs on {{count, number}} devices now",
						})}
					</CellSub>
				) : null}
			</>
		);
	const reason = eligibilityCopy(
		t,
		eligibilityInput({ ...rule, code: rule.code ?? "type" }, event.event_type),
	);
	return (
		<>
			<span className="text-muted-foreground">{reason.long}</span>
			{reason.fix ? (
				<CellSub>
					<a
						className="underline decoration-border-strong underline-offset-2 hover:decoration-current"
						{...hostLink(appEventsHref(appId, event.id))}
					>
						{eligibilityFixLabel(t, reason.fix)}
					</a>
				</CellSub>
			) : null}
			<CellSub>
				{t("deploy.what.notOffered", "Not offered when you deploy")}
			</CellSub>
		</>
	);
}

function EventTableRow(props: Readonly<EventRowProps>) {
	const { t } = useTranslation("devices");
	const { row, appId } = props;
	const { event, rule } = row;
	return (
		<Tr dim={!rule.eligible} data-event={event.id}>
			<Td label={t("deploy.what.colEvent", "Event")} kind="name">
				<EventNameCell {...props} />
			</Td>
			<Td label={t("deploy.what.colType", "Type")}>
				{eventTypeLabel(t, event.event_type, Boolean(event.default_page_id))}
			</Td>
			<Td label={t("deploy.what.colVersions", "Versions")}>
				<VersionsCell row={row} />
			</Td>
			<Td label={t("deploy.what.colOnDevice", "On a device")}>
				{props.now ?? (
					<OnDeviceCell row={row} appId={appId} state={props.state} />
				)}
			</Td>
		</Tr>
	);
}

function GroupToggle({
	open,
	onToggle,
	children,
}: Readonly<{ open: boolean; onToggle(): void; children: string }>) {
	const Icon = open ? ChevronDown : ChevronRight;
	return (
		<GroupRow colSpan={4}>
			<button
				type="button"
				aria-expanded={open}
				onClick={onToggle}
				className="inline-flex items-center gap-1.5 font-medium text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
			>
				<Icon aria-hidden className="size-3.5" />
				{children}
			</button>
		</GroupRow>
	);
}

function EventsHead({ last }: Readonly<{ last: string }>) {
	const { t } = useTranslation("devices");
	return (
		<tr>
			<Th>{t("deploy.what.colEvent", "Event")}</Th>
			<Th>{t("deploy.what.colType", "Type")}</Th>
			<Th>{t("deploy.what.colVersions", "Versions")}</Th>
			<Th>{last}</Th>
		</tr>
	);
}

const EVENT_COLS = ["34%", "14%", "16%", "36%"] as const;

type Scope = DeployDraft["scope"];

function ScopeSwitch({ state, update, route }: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { draft, plan, facts } = state;
	// Whole app leaves schedules unticked: a deploy of the app's pages must not move its nightly jobs off the hub by itself.
	const all = wholeAppEvents(plan.app, facts.hub);
	const pick = (scope: Scope) => {
		if (scope === "app") update({ scope, events: all });
		else if (scope === "event" && route.eventId)
			update({ scope, events: [route.eventId] });
		else update({ scope });
	};
	return (
		<Segmented<Scope>
			label={t("deploy.what.scope", "What to run")}
			value={draft.scope}
			onChange={pick}
			options={[
				{ value: "app", label: t("deploy.what.scopeApp", "Whole app") },
				{
					value: "events",
					label: t("deploy.what.scopeEvents", "Choose events"),
				},
				...(route.eventId
					? [
							{
								value: "event" as const,
								label: t("deploy.what.scopeEvent", "One event"),
							},
						]
					: []),
			]}
		/>
	);
}

function EventsBlock(props: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { state, update, route, check } = props;
	const { draft, mode, plan, app } = state;
	const { ok, cant } = useEventRows(props);
	const [open, setOpen] = useState({ also: false, cant: false });
	if (!app) return null;
	const picked = new Set(draft.events);
	const toggle = (eventId: string, on: boolean) =>
		update({
			events: on
				? [...draft.events, eventId]
				: draft.events.filter((id) => id !== eventId),
		});
	const single = draft.scope === "event" && !!route.eventId;
	const main = single ? ok.filter((row) => row.event.id === route.eventId) : ok;
	const also = single ? ok.filter((row) => row.event.id !== route.eventId) : [];
	const issue = check.issues.find(
		(row) => row.step === "what" && row.code === "no_events",
	);
	const renderRow = (row: EventRow) => (
		<EventTableRow
			key={row.event.id}
			row={row}
			appId={app.id}
			state={state}
			checked={picked.has(row.event.id)}
			// Whole app fixes every tick but a schedule's or a bot's: those are always the person's own choice.
			locked={draft.scope === "app" && !isClaimedKind(row.rule.kind)}
			onToggle={(on) => toggle(row.event.id, on)}
		/>
	);
	return (
		<Block
			title={t("deploy.what.events", "Events")}
			icon={Sparkles}
			summary={
				<span className="inline-flex flex-wrap items-center gap-x-2 gap-y-1">
					<HeadChip>
						{t(
							"deploy.what.eventsCount",
							"{{count, number}} of {{total, number}}",
							{ count: picked.size, total: app.events.length },
						)}
					</HeadChip>
					{state.versionLabel ? (
						<span>
							{t("deploy.what.eventsIn", "in {{version}}", {
								version: state.versionLabel,
							})}
						</span>
					) : null}
				</span>
			}
			stamp={
				mode === "offline" ? (
					<LocalStamp
						text={t("deploy.stamp.onThisComputer", "on this computer")}
					/>
				) : (
					<HubStamp />
				)
			}
			toolbar={<ScopeSwitch {...props} />}
			flush
			foot={
				<>
					{draft.scope === "app"
						? t(
								"deploy.what.footWholeClaimed",
								"Whole app ticks every event that can run on a device, except schedules and bots: tick one yourself to move it to the device.",
							)
						: t(
								"deploy.what.footPick",
								"Pick the events this deploy runs.",
							)}{" "}
					{mode === "online"
						? t(
								"deploy.what.footOnline",
								"The hub leaves out events that can't run on devices; the reasons above come from the app's own event list.",
							)
						: t(
								"deploy.what.footOffline",
								"The device checks the events again after the copy arrives.",
							)}
				</>
			}
		>
			<DvTable
				label={t("deploy.what.eventsLabel", "Events of {{app}}", {
					app: app.name,
				})}
				cols={EVENT_COLS}
				head={<EventsHead last={t("deploy.what.colOnDevice", "On a device")} />}
			>
				{main.map(renderRow)}
				{also.length ? (
					<GroupToggle
						open={open.also}
						onToggle={() => setOpen({ ...open, also: !open.also })}
					>
						{t(
							"deploy.what.alsoRun",
							"Also run other events of {{app}} · {{count, number}}",
							{ app: app.name, count: also.length },
						)}
					</GroupToggle>
				) : null}
				{open.also ? also.map(renderRow) : null}
				{cant.length ? (
					<GroupToggle
						open={open.cant}
						onToggle={() => setOpen({ ...open, cant: !open.cant })}
					>
						{t(
							"deploy.what.cantRun",
							"Can't run on devices · {{count, number}}",
							{ count: cant.length },
						)}
					</GroupToggle>
				) : null}
				{open.cant ? cant.map(renderRow) : null}
			</DvTable>
			{issue ? (
				<p className="px-4 py-2.5 text-xs text-critical">
					{issueText(t, issue, planNames(t, plan))}
				</p>
			) : null}
		</Block>
	);
}

/** "Stop serving … after the update": the one acknowledgement for events an update drops. */
function RemovedEvents({ state, update }: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { plan, draft } = state;
	const dropped = plan.targets.flatMap((target) =>
		target.services
			.filter((service) => service.removedEvents.length > 0)
			.map((service) => ({ target, service })),
	);
	if (!dropped.length) return null;
	return (
		<section
			aria-label={t(
				"deploy.what.removedLabel",
				"Events that stop being served",
			)}
			className="flex flex-col gap-2 rounded-lg border border-warning-line bg-warning-bg px-3 py-3 text-ui"
		>
			{dropped.map(({ target, service }) => (
				<p key={`${target.deviceId}/${service.serviceId}`} className="text-ui">
					{t("deploy.what.removed", {
						events: service.removedEvents
							.map((eventId) => eventName(plan, eventId))
							.join(", "),
						service: service.serviceId,
						device: target.name,
						count: service.removedEvents.length,
						defaultValue_one:
							"{{events}} stops being served by {{service}} on {{device}} after the update.",
						defaultValue_other:
							"{{events}} stop being served by {{service}} on {{device}} after the update.",
					})}
				</p>
			))}
			<CheckField
				id="deploy-accept-removed"
				checked={draft.acceptRemovedEvents}
				onCheckedChange={(acceptRemovedEvents) =>
					update({ acceptRemovedEvents })
				}
			>
				{t(
					"deploy.what.acceptRemoved",
					"Stop serving these events after the update",
				)}
			</CheckField>
		</section>
	);
}

function servesNow(
	target: PlanTarget | undefined,
	eventId: string,
	state: PlanStepProps["state"],
) {
	const device = state.devices.find((row) => row.id === target?.deviceId);
	const [planned] = target?.services ?? [];
	return (
		device?.services
			?.find((service) => service.serviceId === planned?.serviceId)
			?.events?.some((event) => event.event_id === eventId) ?? false
	);
}

/** One service: its events ticked, the app's other events offered unticked. */
function UpdateEventsTable(props: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { state, update } = props;
	const { draft, app, plan } = state;
	const { ok } = useEventRows(props);
	if (!app) return null;
	const [target] = plan.targets;
	const picked = new Set(draft.events);
	return (
		<DvTable
			label={t("deploy.what.updateEventsLabel", "Events in this update")}
			cols={EVENT_COLS}
			head={<EventsHead last={t("deploy.what.colNow", "Now")} />}
		>
			{ok.map((row) => (
				<EventTableRow
					key={row.event.id}
					row={row}
					appId={app.id}
					state={state}
					checked={picked.has(row.event.id)}
					now={
						servesNow(target, row.event.id, state) ? (
							t("deploy.what.servedNow", "Served now")
						) : isClaimedKind(row.rule.kind) ? (
							// Adding a schedule or a bot moves it: the row says when it runs or what it answers, and where it runs today.
							<>
								{t("deploy.what.notServed", "Not served yet")}
								<KindLines row={row} state={state} sub />
							</>
						) : (
							t("deploy.what.notServed", "Not served yet")
						)
					}
					onToggle={(on) =>
						update({
							scope: "events",
							events: on
								? [...draft.events, row.event.id]
								: draft.events.filter((id) => id !== row.event.id),
						})
					}
				/>
			))}
		</DvTable>
	);
}

/** Several services: each keeps the events it serves now. */
function KeptEvents({ state }: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { plan } = state;
	if (!plan.targets.length)
		return (
			<div className="px-4 py-3">
				<StateView
					kind="empty"
					title={t("deploy.what.noServicePicked", "No service picked yet")}
					text={t(
						"deploy.what.noServicePickedText",
						"Pick the devices in Where; each one's service of this app is listed here.",
					)}
				/>
			</div>
		);
	return (
		<ul className="flex flex-col">
			{plan.targets.map((target) => {
				const [service] = target.services;
				const known = service && service.kind !== "new";
				return (
					<li
						key={target.deviceId}
						className="flex flex-col gap-0.5 border-t border-hairline px-4 py-2.5 text-ui first:border-t-0"
					>
						<span>
							{known ? (
								<>
									<span className="font-mono">{service.serviceId}</span>{" "}
									<span className="text-muted-foreground">
										{t("deploy.what.on", "on")}
									</span>{" "}
								</>
							) : null}
							<span className="font-mono">{target.name}</span>
						</span>
						<span className="text-xs text-muted-foreground">
							{known
								? service.events
										.map((eventId) => eventName(plan, eventId))
										.join(" · ") ||
									t(
										"deploy.what.noEventsKept",
										"None of its events are in this version",
									)
								: target.locked
									? t(
											"deploy.what.servicesLocked",
											"Its services load when you unlock.",
										)
									: t(
											"deploy.what.servicesLoading",
											"Its services load once it's connected.",
										)}
						</span>
					</li>
				);
			})}
		</ul>
	);
}

/**
 * Events that follow Latest which an updated service already serves and whose
 * flow moved on since: every update that sends a new copy takes the flow as it
 * is then, also one made for another reason.
 */
function movedLatestEvents(state: PlanStepProps["state"]): string[] {
	const { plan, devices, draft } = state;
	if (draft.version === "keep") return [];
	const moved = new Set<string>();
	for (const target of plan.targets) {
		const device = devices.find((row) => row.id === target.deviceId);
		for (const service of target.services) {
			const served =
				device?.services?.find((row) => row.serviceId === service.serviceId)
					?.events ?? [];
			for (const pin of served) {
				const event = plan.app?.events.find((row) => row.id === pin.event_id);
				if (!event || !service.events.includes(event.id)) continue;
				const flow = typeof event.flow === "object" ? event.flow : null;
				if (!flow || !appEventRule(event).followsLatest) continue;
				if (flow.current?.join(".") !== pin.board_version.join("."))
					moved.add(event.id);
			}
		}
	}
	return [...moved];
}

function UpdateTakesEdits({ state }: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const moved = movedLatestEvents(state);
	if (!moved.length) return null;
	return (
		<ul data-takes-edits="" className="flex flex-col gap-1 px-4 pb-3 text-xs">
			{moved.map((eventId) => (
				<li key={eventId} className="flex items-start gap-1.5 text-ink-2">
					<CircleArrowUp
						aria-hidden
						className="mt-0.5 size-3 shrink-0 text-info"
					/>
					{t(
						"deploy.what.updateTakesEdits",
						"Updating also takes the current flow edits of {{event}}.",
						{ event: eventName(state.plan, eventId) },
					)}
				</li>
			))}
		</ul>
	);
}

function UpdateEventsBlock(props: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { state } = props;
	const { draft } = state;
	return (
		<Block
			title={t("deploy.what.updateEvents", "Events in this update")}
			icon={Sparkles}
			stamp={
				<TargetsStamp targets={state.plan.targets} devices={state.devices} />
			}
			flush
			foot={
				draft.keepEvents
					? t(
							"deploy.what.updateFootKept",
							"Each service keeps the events it serves now. To add or drop events, update one service from its row.",
						)
					: draft.version === "keep"
						? t(
								"deploy.what.updateFootKeep",
								"Keeping the version: only events already in each service's version can be served.",
							)
						: t(
								"deploy.what.updateFootNewest",
								"Events the service doesn't serve yet are listed unticked. A service can serve 64 events at most.",
							)
			}
		>
			{draft.keepEvents ? (
				<KeptEvents {...props} />
			) : (
				<UpdateEventsTable {...props} />
			)}
			<UpdateTakesEdits {...props} />
			<div className={cx("px-4 pb-3 empty:hidden")}>
				<RemovedEvents {...props} />
			</div>
		</Block>
	);
}

function ServicesBlock({ state, update, check }: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { plan, draft } = state;
	if (draft.entry === "update" || !plan.services.length) return null;
	const events = plan.services.flatMap((service) => service.events);
	const showSplit = events.length > 1;
	return (
		<Block
			title={
				showSplit
					? t("deploy.what.services", "Services")
					: t("deploy.what.service", "Service")
			}
			icon={Box}
			count={showSplit ? plan.services.length : undefined}
			stamp={<LocalStamp text={t("deploy.stamp.yourChoice", "your choice")} />}
			foot={
				showSplit
					? t(
							"deploy.what.servicesFoot",
							"A service is what runs on a device: one endpoint, one set of settings and one cloud access. IDs are permanent once deployed, and a device that already uses one gets a -2 suffix.",
						)
					: undefined
			}
		>
			<ServicePlan
				plan={plan}
				check={check}
				showSplit={showSplit}
				onSplit={(split) => update({ split })}
				onRename={(key, id) =>
					update({ serviceIds: { ...draft.serviceIds, [key]: id } })
				}
			/>
		</Block>
	);
}

export function WhatStep(props: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { state } = props;
	const updating = state.draft.entry === "update";
	return (
		<div className="@container/whatstep flex min-w-0 flex-col gap-4">
			<WizardStepHeader
				step={1}
				total={8}
				title={t("deploy.title.what", "What to run")}
				lede={t(
					"deploy.what.lede",
					"Pick the version and the events. They decide which devices can take it.",
				)}
			/>
			<AppBlock {...props} />
			{state.app && !state.canReadFlows ? (
				<StateView
					kind="noaccess"
					title={t(
						"deploy.what.noFlows",
						"Your role can't read the flows of {{app}}",
						{ app: state.app.name },
					)}
					text={t(
						"deploy.what.noFlowsText",
						"Deploying needs Read boards on the app. Ask its owner for it.",
					)}
				/>
			) : state.app ? (
				<>
					<VersionBlock {...props} />
					{updating ? (
						<UpdateEventsBlock {...props} />
					) : (
						<>
							<EventsBlock {...props} />
							<ServicesBlock {...props} />
						</>
					)}
				</>
			) : props.scope.kind === "app" ? null : (
				<GateInline kind="hub">
					{t(
						"deploy.what.pickApp",
						"Pick an app to see its version and events.",
					)}
				</GateInline>
			)}
		</div>
	);
}
