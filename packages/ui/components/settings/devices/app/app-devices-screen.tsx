"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	Boxes,
	Copy,
	Info,
	Lock,
	LockOpen,
	Rocket,
	Sparkles,
	TriangleAlert,
} from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useState,
} from "react";
import { useUserIdentity } from "../../../../hooks/use-user-lookup";
import type { AppView } from "../../../../lib/device-management/model/app-plan";
import type { AppDevicesRoute } from "../../../../lib/device-management/model/types";
import { gateCopy } from "../copy/gate-copy";
import { headlineCopy } from "../copy/headline-copy";
import { AppMetricsBlock } from "../observe/app-metrics";
import { useAreaTime, useHubFreshness } from "../primitives/area-context";
import { AttentionList } from "../primitives/attention-list";
import { Banner } from "../primitives/banner";
import { Block } from "../primitives/block";
import { CoverageLine } from "../primitives/coverage-line";
import { DvButton } from "../primitives/dv-button";
import {
	FreshnessStamp,
	MixedSourcesStamp,
	baseSource,
} from "../primitives/freshness-stamp";
import type { Gate } from "../primitives/gate-notice";
import { Headline } from "../primitives/headline";
import { StateView } from "../primitives/state-view";
import { cx } from "../primitives/tone";
import { UnderlineTabs } from "../primitives/underline-tabs";
import { copyText } from "../primitives/use-copy";
import { appEventsHref } from "../routing/devices-href";
import { ACCOUNT_SCOPE } from "../routing/devices-route";
import { useDevicesRoute } from "../routing/use-devices-route";
import type { ScreenProps } from "../screen-props";
import { stampOf, useAttentionEntries } from "../shell/attention-popover";
import { useDeviceRows, useDeviceWorkspace, useOverlay } from "../workspace";
import { AppAccess } from "./app-access";
import { AppHeader } from "./app-header";
import { AppInProgress, runTitle } from "./app-in-progress";
import {
	APP_LINKS,
	type AppPage,
	AppPageProvider,
	HostLink,
	LINK,
	LinkButton,
	useAppPage,
	useDeviceNames,
	useNameList,
} from "./app-shared";
import { AppSpending } from "./app-spending";
import {
	ATTENTION_CAP,
	SENTENCE_NAME_CAP,
	blocksDeploy,
	updateAllGate,
	updateRows,
	versionName,
} from "./app-view-local";
import { EverywhereElse } from "./everywhere-else";
import { HowRunsBlock, ModeBlock } from "./how-runs-block";
import {
	UpdateEverywhereSheet,
	UpdateRun,
	type UpdateRunSpec,
} from "./update-everywhere-sheet";
import {
	type AppDevicesData,
	useAppAttention,
	useAppDevices,
} from "./use-app-devices";
import { VersionsBlock } from "./versions-block";
import { WhereByDevice, WhereFoot } from "./where-by-device";
import { EventFoot, EventList, WhereByEvent } from "./where-by-event";

const DEFAULT_ROUTE: AppDevicesRoute = { screen: "app-devices", by: "device" };

/* Gates of the header buttons (APP §2.4). */

function useUpdateGate(
	view: AppView,
	data: AppDevicesData,
	runActive: boolean,
): Gate | null {
	const { t } = useTranslation("devices");
	return useMemo(() => {
		const code = updateAllGate(view, runActive);
		const newest = view.versions[0];
		if (code === "no_devices")
			return {
				kind: "busy",
				reason: t(
					"app.gate.updateNoDevices",
					"Nothing to update yet: you have no devices.",
				),
			};
		if (code === "unreadable")
			return {
				kind: "locked",
				reason: t(
					"app.gate.updateUnreadable",
					"Unlock first: which services run on your devices isn't readable yet.",
				),
			};
		if (code === "nothing_deployed")
			return {
				kind: "busy",
				reason: t(
					"app.gate.nothingDeployed",
					"Nothing to update: {{app}} isn't on any device you can see.",
					{ app: view.app.name },
				),
			};
		if (code === "run_active") {
			// A deploy that is still running is named; an update is "the current update".
			const deploy = data.runs.find(
				(entry) => entry.open && entry.run.title.code !== "update",
			);
			return {
				kind: "busy",
				reason: deploy
					? t("app.gate.runActiveNamed", "Wait for “{{title}}” to finish.", {
							title: runTitle(t, deploy.run.title, deploy.total),
						})
					: t("app.gate.runActive", "Wait for the current update to finish."),
			};
		}
		if (code === "all_newest")
			return {
				kind: "busy",
				reason: newest
					? t(
							"app.gate.allNewest",
							"Every service you can see already runs {{version}}.",
							{ version: versionName(newest) },
						)
					: t(
							"app.gate.allNewestUnnamed",
							"Every service you can see already runs the newest version.",
						),
			};
		if (code === "all_staged")
			return {
				kind: "busy",
				reason: newest
					? t(
							"app.gate.allStaged",
							"No service can take an update right now. An update to {{version}} is staged. Activate or discard it first.",
							{ version: versionName(newest) },
						)
					: t(
							"app.gate.allStagedUnnamed",
							"No service can take an update right now. An update is staged. Activate or discard it first.",
						),
			};
		// Every service left to update sits on a device that can't take one now (offline, no permission).
		const gates = updateRows(view).map((row) =>
			row.blocked === "staged"
				? null
				: (data.deployGates.get(row.deviceId) ?? null),
		);
		const first = gates.find((gate) => blocksDeploy(gate));
		if (first && gates.every((gate) => blocksDeploy(gate)))
			return {
				kind: first.kind,
				reason: t(
					"app.gate.noneCan",
					"No service can take an update right now. {{reason}}",
					{ reason: gateCopy(t, first).inline },
				),
			};
		return null;
	}, [view, data.deployGates, data.runs, runActive, t]);
}

function useDeployGate(view: AppView, data: AppDevicesData): Gate | null {
	const { t } = useTranslation("devices");
	const { href } = useDevicesRoute();
	return useMemo(() => {
		if (!view.events.rows.length)
			return {
				kind: "unsupported",
				reason: (
					<>
						{t(
							"app.gate.noEvents",
							"None of {{app}}'s events can run on a device. Pin a flow version or add a Web request, Chat, Page, REST, MCP or Background event.",
							{ app: view.app.name },
						)}{" "}
						<HostLink
							href={appEventsHref(data.appId)}
							className={cx(LINK, "underline")}
						>
							{t("app.gate.events", "Events")}
						</HostLink>
					</>
				),
			};
		const active = [...data.devices.values()].filter(
			(device) => device.presence.kind !== "revoked",
		);
		const able = active.some(
			(device) => !blocksDeploy(data.deployGates.get(device.row.device_id)),
		);
		if (able) return null;
		return {
			kind: "live",
			reason: (
				<>
					{t(
						"app.gate.noDevice",
						"You have no device that can take a deploy right now.",
					)}{" "}
					<a
						href={href({ screen: "setup" }, ACCOUNT_SCOPE)}
						className={cx(LINK, "underline")}
					>
						{t("app.gate.setUp", "Set up a device")}
					</a>
				</>
			),
		};
	}, [view, data.devices, data.deployGates, data.appId, href, t]);
}

/* Headline, coverage and notes (APP §2.6). */

function PartialNote({ deviceId }: Readonly<{ deviceId: string }>) {
	const { t } = useTranslation("devices");
	const { view, data } = useAppPage();
	const deviceName = useDeviceNames();
	const owner = useUserIdentity(data.devices.get(deviceId)?.row.owner_id);
	const values = { device: deviceName(deviceId), app: view.app.name };
	return (
		<Note>
			{owner.isResolved ? (
				<Trans
					t={t}
					i18nKey="app.coverage.partialOwner"
					defaults="Partial access on <1>{{device}}</1>: you see {{app}} only. {{owner}} owns it."
					values={{ ...values, owner: owner.label }}
					components={{ 1: <span className="font-mono" /> }}
				/>
			) : (
				<Trans
					t={t}
					i18nKey="app.coverage.partial"
					defaults="Partial access on <1>{{device}}</1>: you see {{app}} only."
					values={values}
					components={{ 1: <span className="font-mono" /> }}
				/>
			)}
		</Note>
	);
}

function Note({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<p
			data-coverage-note=""
			className="flex items-start gap-1.5 text-xs text-muted-foreground"
		>
			<Info aria-hidden className="mt-0.5 size-3.5 shrink-0" />
			<span className="min-w-0">{children}</span>
		</p>
	);
}

function AppCoverage() {
	const { t } = useTranslation("devices");
	const { view, data } = useAppPage();
	const overlay = useOverlay();
	const deviceName = useDeviceNames();
	const { coverage } = data;
	const never = coverage.never.length;
	const unknown = coverage.unknown.length - never;
	const locked = coverage.locked.filter((id) => !coverage.never.includes(id));
	const app = view.app.name;
	return (
		<div data-app-coverage="" className="flex flex-col gap-1.5">
			<CoverageLine
				readable={coverage.readable}
				total={coverage.total}
				live={coverage.live}
				snapshot={coverage.snapshot}
				unknown={unknown}
				never={never}
				actions={
					locked.length === 1 ? (
						<DvButton
							size="sm"
							icon={LockOpen}
							data-act="unlock"
							onClick={() => overlay.openUnlock(locked[0])}
						>
							{t("app.coverage.unlockOne", "Unlock {{device}}…", {
								device: deviceName(locked[0]),
							})}
						</DvButton>
					) : locked.length > 1 ? (
						<DvButton
							size="sm"
							icon={LockOpen}
							data-act="unlock-several"
							onClick={() => overlay.openUnlockSeveral()}
						>
							{t("app.coverage.unlockMany", "Unlock {{count, number}}…", {
								count: locked.length,
							})}
						</DvButton>
					) : undefined
				}
			/>
			{coverage.partial.length > 2 ? (
				<Note>
					{t(
						"app.coverage.partialMany",
						"Partial access on {{count, number}} devices: you see {{app}} only there.",
						{ count: coverage.partial.length, app },
					)}
				</Note>
			) : (
				coverage.partial.map((deviceId) => (
					<PartialNote key={deviceId} deviceId={deviceId} />
				))
			)}
			{coverage.noAccess.length > 2 ? (
				<Note>
					{t(
						"app.coverage.noAccessMany",
						"Your access doesn't cover {{app}} on {{count, number}} devices.",
						{ count: coverage.noAccess.length, app },
					)}
				</Note>
			) : (
				coverage.noAccess.map((deviceId) => (
					<Note key={deviceId}>
						<Trans
							t={t}
							i18nKey="app.coverage.noAccess"
							defaults="Your access to <1>{{device}}</1> doesn't cover {{app}}."
							values={{ device: deviceName(deviceId), app }}
							components={{ 1: <span className="font-mono" /> }}
						/>
					</Note>
				))
			)}
		</div>
	);
}

function AppHeadline() {
	const { t } = useTranslation("devices");
	const { data } = useAppPage();
	const time = useAreaTime();
	const copy = useMemo(
		() => (data.headline ? headlineCopy(t, data.headline, time) : null),
		[data.headline, t, time],
	);
	if (!copy) return null;
	return <Headline lead={copy.lead} rest={copy.rest} />;
}

/* Needs attention (APP §2.7). */

function AppAttention() {
	const { t } = useTranslation("devices");
	const { data } = useAppPage();
	const { navigate } = useDevicesRoute();
	const items = useAppAttention(data.appId);
	const [expanded, setExpanded] = useState(false);
	const entries = useAttentionEntries(items, { onNavigate: navigate });
	const base = useMemo(
		() => baseSource(entries.map((entry) => entry.stamp)),
		[entries],
	);
	if (!items.length) return null;
	return (
		<Block
			id="ad-attn"
			icon={TriangleAlert}
			title={t("app.attention.title", "Needs attention")}
			count={items.length}
			stamp={base ? <FreshnessStamp {...base} /> : <MixedSourcesStamp />}
			flush
		>
			<AttentionList
				items={entries}
				compact
				base={base}
				emptyText={false}
				cap={expanded ? items.length : ATTENTION_CAP}
				expanded={expanded}
				onShowAll={() => setExpanded(true)}
				onShowFewer={() => setExpanded(false)}
			/>
		</Block>
	);
}

/* Where it runs (APP §2.9, §2.10). */

function WhereItRuns() {
	const { t } = useTranslation("devices");
	const { view, route } = useAppPage();
	const { navigate } = useDevicesRoute();
	const rows = useDeviceRows();
	const stamps = useMemo(
		() => view.services.map((row) => stampOf(row.view.freshness)),
		[view.services],
	);
	const base = useMemo(
		() => (stamps.length === 1 ? stamps[0] : (baseSource(stamps) ?? null)),
		[stamps],
	);
	const by = route.by;
	return (
		<Block
			id="ad-where"
			icon={Boxes}
			title={t("app.where.title", "Where it runs")}
			count={view.services.length}
			summary={
				<UnderlineTabs
					label={t("app.where.views", "Views of where it runs")}
					value={by}
					onValueChange={(next) => navigate({ ...route, by: next })}
					tabs={[
						{ value: "device", label: t("app.where.byDevice", "By device") },
						{ value: "event", label: t("app.where.byEvent", "By event") },
					]}
					className="@min-[720px]/app:-my-3 @min-[720px]/app:translate-y-px"
					listClassName="w-auto border-b-0"
				/>
			}
			stamp={
				base ? (
					<FreshnessStamp {...base} />
				) : stamps.length ? (
					<MixedSourcesStamp />
				) : (
					<FreshnessStamp {...stampOf(rows.freshness)} />
				)
			}
			flush
			foot={by === "event" ? <EventFoot /> : <WhereFoot />}
		>
			{by === "event" ? <WhereByEvent /> : <WhereByDevice base={base} />}
		</Block>
	);
}

function AllUnknown() {
	const { t } = useTranslation("devices");
	const { view, data } = useAppPage();
	const overlay = useOverlay();
	const deviceName = useDeviceNames();
	const nameList = useNameList();
	const { coverage } = data;
	const locked = coverage.locked.filter((id) => !coverage.never.includes(id));
	const lockable = coverage.locked.length > 0;
	return (
		<Block
			id="ad-where"
			icon={Boxes}
			title={t("app.where.title", "Where it runs")}
			stamp={
				<FreshnessStamp
					source="local"
					age="locked"
					text={t("app.where.allLockedStamp", {
						count: coverage.unknown.length,
						defaultValue_one: "locked · {{count, number}} device",
						defaultValue_other: "locked · {{count, number}} devices",
					})}
				/>
			}
		>
			<StateView
				kind="locked"
				title={t(
					"app.where.allLockedTitle",
					"Unlock to see where {{app}} runs",
					{ app: view.app.name },
				)}
				text={
					locked.length
						? t("app.where.allLockedNamed", {
								count: locked.length,
								names: nameList(locked.map(deviceName), SENTENCE_NAME_CAP),
								defaultValue_one:
									"Which services run on a device is only readable with that device's keys on this computer. {{names}} is locked here. Until you unlock it, it counts as unknown, never as not deployed.",
								defaultValue_other:
									"Which services run on a device is only readable with that device's keys on this computer. {{count, number}} devices are locked here: {{names}}. Until you unlock them they count as unknown, never as not deployed.",
							})
						: t(
								"app.where.allLockedText",
								"The hub doesn't know which apps run on your devices; only keys on this computer can read it. Until then every device counts as unknown, never as not deployed.",
							)
				}
				actions={
					lockable ? (
						<DvButton
							size="sm"
							icon={LockOpen}
							data-act="unlock-several"
							onClick={() => overlay.openUnlockSeveral()}
						>
							{t("app.coverage.unlockSeveral", "Unlock several…")}
						</DvButton>
					) : undefined
				}
			/>
		</Block>
	);
}

/* Never deployed (APP §2.18). */

function CanRun() {
	const { t } = useTranslation("devices");
	const { view, data } = useAppPage();
	const time = useAreaTime();
	const total = view.events.rows.length + view.events.ineligible.length;
	const checkedAt = view.app.localOnly ? undefined : data.readAt;
	return (
		<Block
			id="ad-can-run"
			icon={Sparkles}
			title={
				<>
					{t("app.never.canRun", "What can run on a device")}
					<span className={COUNT_PILL}>
						{t(
							"app.never.canRunCount",
							"{{eligible, number}} of {{total, number}}",
							{ eligible: view.events.rows.length, total },
						)}
					</span>
				</>
			}
			stamp={
				<FreshnessStamp
					source={view.app.localOnly ? "local" : "hub"}
					age="current"
					text={
						checkedAt
							? t("app.never.eventsStampChecked", "events · checked {{ago}}", {
									ago: time.ago(checkedAt),
								})
							: t("app.never.eventsStamp", "events")
					}
					{...(checkedAt ? { observedAt: checkedAt } : {})}
				/>
			}
			tools={
				<DvButton size="sm" variant="ghost" icon={Sparkles} asChild>
					<HostLink href={appEventsHref(data.appId)}>
						{t("app.header.manageEvents", "Manage events")}
					</HostLink>
				</DvButton>
			}
			flush
		>
			<EventList />
		</Block>
	);
}

function DeployPrompt({ gate }: Readonly<{ gate: Gate | null }>) {
	const { t } = useTranslation("devices");
	const { view } = useAppPage();
	return (
		<StateView
			kind="empty"
			icon={Rocket}
			title={t("app.never.deployTitle", "Deploy {{app}} to a device", {
				app: view.app.name,
			})}
			text={t(
				"app.never.deployText",
				"Pick what to run and where. Nothing is sent until you review it.",
			)}
			actions={
				<LinkButton
					route={APP_LINKS.deploy()}
					gate={gate}
					variant="primary"
					icon={Rocket}
					act="ad-deploy-empty"
				>
					{t("app.header.deploy", "Deploy to devices…")}
				</LinkButton>
			}
		/>
	);
}

function NoDevices() {
	const { t } = useTranslation("devices");
	const { view } = useAppPage();
	return (
		<StateView
			kind="empty"
			title={t("app.noDevices.title", "You don't have any devices yet")}
			text={t("app.noDevices.text", "Set one up, then deploy {{app}} to it.", {
				app: view.app.name,
			})}
			actions={
				<LinkButton
					route={{ screen: "setup" }}
					scope={ACCOUNT_SCOPE}
					variant="primary"
					act="set-up-device"
				>
					{t("app.gate.setUp", "Set up a device")}
				</LinkButton>
			}
		/>
	);
}

function HubFailing() {
	const { t } = useTranslation("devices");
	const hub = useHubFreshness();
	const time = useAreaTime();
	if (!hub.failing) return null;
	return (
		<Banner
			tone="warning"
			title={t("app.failing.title", "The hub can't be reached right now")}
			actions={
				hub.onRetry ? (
					<DvButton size="sm" onClick={hub.onRetry}>
						{t("app.failing.retry", "Retry now")}
					</DvButton>
				) : undefined
			}
		>
			{hub.dataFrom
				? t(
						"app.failing.textFrom",
						"The device list and cloud access are from {{time}}. Status from unlocked devices keeps updating.",
						{ time: time.clock(hub.dataFrom) },
					)
				: t(
						"app.failing.text",
						"The device list and cloud access may be out of date. Status from unlocked devices keeps updating.",
					)}
		</Banner>
	);
}

/* Page. */

/* The count pill of a block head, for a count that is words ("2 of 4"). */
const COUNT_PILL =
	"ml-2 inline-flex h-5 items-center rounded-full bg-muted px-1.5 align-middle font-mono text-xs font-medium tabular-nums text-ink-2";

/* PROTO `.main-in`: 24 px between the page's sections. */
const PAGE = "@container/app flex min-w-0 flex-col gap-6";

function AppBody({
	data,
	view,
	route,
	scope,
}: Readonly<{
	data: AppDevicesData;
	view: AppView;
	route: AppDevicesRoute;
	scope: ScreenProps["scope"];
}>) {
	const { clearParam } = useDevicesRoute();
	const [sheet, setSheet] = useState(false);
	const [run, setRun] = useState<UpdateRunSpec | null>(null);
	const [runId, setRunId] = useState<string | undefined>();
	const [runDone, setRunDone] = useState(false);
	const runActive =
		(!!run && !runDone) || data.runs.some((entry) => entry.open);
	const updateGate = useUpdateGate(view, data, runActive);
	const deployGate = useDeployGate(view, data);
	const openUpdateAll = useCallback(() => setSheet(true), []);
	const page: AppPage = useMemo(
		() => ({
			data,
			view,
			scope,
			route,
			openUpdateAll,
			updateAllGate: updateGate,
		}),
		[data, view, scope, route, openUpdateAll, updateGate],
	);
	const wantsUpdate = route.action === "update-all";
	useEffect(() => {
		if (!wantsUpdate) return;
		if (!updateGate) setSheet(true);
		clearParam("action");
	}, [wantsUpdate, updateGate, clearParam]);
	const runBlock = run ? (
		<UpdateRun
			spec={run}
			view={view}
			onRunId={setRunId}
			onDismiss={() => {
				setRun(null);
				setRunId(undefined);
			}}
		/>
	) : undefined;
	// A finished run no longer blocks the next one.
	useEffect(() => {
		if (!runId) return;
		const tracked = data.runs.find((entry) => entry.run.id === runId);
		setRunDone(!tracked?.open && !!run);
	}, [runId, data.runs, run]);
	const never = view.layout === "never";
	return (
		<AppPageProvider value={page}>
			<div data-app-devices={view.layout} className={PAGE}>
				<AppHeader
					deployGate={deployGate}
					plainDeploy={never || view.layout === "no_devices"}
				/>
				<HubFailing />
				<HowRunsBlock />
				<AppHeadline />
				{view.layout === "no_devices" ||
				(never &&
					data.coverage.unknown.length === 0 &&
					!data.coverage.noAccess.length) ? null : (
					<AppCoverage />
				)}
				{view.layout === "no_devices" ? <NoDevices /> : null}
				{view.layout === "normal" ? (
					<>
						<AppAttention />
						<AppInProgress updateRun={runBlock} skipRunId={runId} />
						<WhereItRuns />
						<VersionsBlock />
						<div className="grid grid-cols-2 items-start gap-6 @max-[1100px]/app:grid-cols-1">
							<EverywhereElse />
							<AppSpending />
						</div>
						<AppMetricsBlock appId={data.appId} />
						<AppAccess />
					</>
				) : null}
				{view.layout === "all_unknown" ? (
					<>
						<AllUnknown />
						<VersionsBlock />
						<div className="grid grid-cols-2 items-start gap-6 @max-[1100px]/app:grid-cols-1">
							<EverywhereElse />
							<AppSpending />
						</div>
					</>
				) : null}
				{never ? (
					<>
						<CanRun />
						<ModeBlock />
						<EverywhereElse variant="could" />
						<DeployPrompt gate={deployGate} />
					</>
				) : null}
			</div>
			<UpdateEverywhereSheet
				open={sheet}
				onOpenChange={setSheet}
				onStart={(spec) => {
					setRunDone(false);
					setRun(spec);
				}}
			/>
		</AppPageProvider>
	);
}

function LockedPanel({ data }: Readonly<{ data: AppDevicesData }>) {
	const { t } = useTranslation("devices");
	const [copied, setCopied] = useState(false);
	const role = data.role.roleName;
	const request = t(
		"app.locked.request",
		"Hi, I'd like to see where {{app}} runs on devices. Could you give my role the “Read boards” permission on {{app}}?",
		{ app: data.appName },
	);
	return (
		<div data-app-devices="no-role" className={PAGE}>
			<h1 className="text-2xl/[30px] font-semibold tracking-[-0.015em]">
				{t("app.header.title", "Devices")}
			</h1>
			<StateView
				kind="noaccess"
				icon={Lock}
				title={t("app.locked.title", "Devices is locked for your role")}
				text={
					<>
						{role
							? t(
									"app.locked.textRole",
									"Your role on {{app}} ({{role}}) can't read its flows, so this page can't show where it runs. That doesn't mean it runs nowhere.",
									{ app: data.appName, role },
								)
							: t(
									"app.locked.text",
									"Your role on {{app}} can't read its flows, so this page can't show where it runs. That doesn't mean it runs nowhere.",
									{ app: data.appName },
								)}{" "}
						<span className="text-muted-foreground">
							{t("app.locked.needs", "Needs: Read boards.")}
						</span>
					</>
				}
				actions={
					<DvButton
						size="sm"
						icon={Copy}
						data-act="copy-request"
						onClick={() => void copyText(request).then((ok) => setCopied(ok))}
					>
						{copied
							? t("app.locked.copied", "Copied")
							: t("app.locked.copy", "Copy a request for the owner")}
					</DvButton>
				}
			/>
		</div>
	);
}

/** App › Devices (APP §2): where and how one app runs, from inside the app's own settings. */
export function AppDevicesScreen({
	route,
	scope,
	appId,
}: Readonly<ScreenProps & { appId: string }>) {
	const { t } = useTranslation("devices");
	const workspace = useDeviceWorkspace();
	const appRoute = route.screen === "app-devices" ? route : DEFAULT_ROUTE;
	const data = useAppDevices(appId, appRoute.focusDeviceId);
	if (data.role.loading)
		return (
			<StateView
				kind="loading"
				rows={4}
				title={t("app.state.loading", "Loading where this app runs…")}
			/>
		);
	if (!data.role.canReadFlows) return <LockedPanel data={data} />;
	if (data.visibility === "Offline" && workspace.deps.platform === "web")
		return (
			<div data-app-devices="web-local" className={PAGE}>
				<StateView
					kind="gate"
					gate="platform"
					title={t(
						"app.state.webLocal",
						"{{app}} only exists on the computer that created it",
						{ app: data.appName },
					)}
					text={t(
						"app.state.webLocalText",
						"Open it in the desktop app there to see where it runs and to deploy it.",
					)}
				/>
			</div>
		);
	if (!data.view)
		return data.error ? (
			<StateView
				kind="error"
				title={t("app.state.error", "This app couldn't be loaded")}
				text={t(
					"app.state.errorText",
					"Its events or settings couldn't be read, so where it runs can't be shown. Try again in a moment.",
				)}
				actions={
					<DvButton size="sm" onClick={() => void data.refresh()}>
						{t("app.state.retry", "Try again")}
					</DvButton>
				}
			/>
		) : (
			<StateView
				kind="loading"
				rows={4}
				title={t("app.state.loading", "Loading where this app runs…")}
			/>
		);
	return (
		<AppBody
			key={appId}
			data={data}
			view={data.view}
			route={appRoute}
			scope={scope}
		/>
	);
}
