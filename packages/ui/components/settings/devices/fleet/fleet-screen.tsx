"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Boxes,
	CloudUpload,
	Plus,
	RefreshCw,
	Server,
	ShieldCheck,
	UserPlus,
} from "lucide-react";
import { useContext, useEffect, useMemo, useRef, useState } from "react";
import { AuthContext } from "react-oidc-context";
import { toHubError } from "../../../../lib/device-management/hub/endpoints";
import { headline } from "../../../../lib/device-management/model/headline";
import type {
	FleetFilter,
	FleetRoute,
	FleetView,
} from "../../../../lib/device-management/model/types";
import { headlineCopy, headlineNames } from "../copy/headline-copy";
import {
	Annunciator,
	type AnnunciatorCell,
	type AnnunciatorWindow,
} from "../primitives/annunciator";
import { useAreaTime } from "../primitives/area-context";
import { Banner } from "../primitives/banner";
import { Block, PageHeader } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import {
	FreshnessStamp,
	MixedSourcesStamp,
} from "../primitives/freshness-stamp";
import { Headline } from "../primitives/headline";
import { InlineResult, type ResultTone } from "../primitives/inline-result";
import { Segmented } from "../primitives/segmented";
import { StateView } from "../primitives/state-view";
import { useDevicesRoute, useRouteLink } from "../routing/use-devices-route";
import type { ScreenProps } from "../screen-props";
import { stampOf } from "../shell/attention-popover";
import { ScopeSwitch } from "../shell/scope-switch";
import {
	hubErrorCopy,
	useAttentionState,
	useDeviceRows,
	useDeviceWorkspace,
	useHubSupport,
	useLocalSummary,
} from "../workspace";
import { EncryptedStatusExceptions } from "./encrypted-status-exceptions";
import type { FleetDeviceEntry } from "./fleet-device-row";
import { FleetDevicesTable, useFleetEntries } from "./fleet-devices-table";
import { FleetServicesTable, useFleetServices } from "./fleet-services-table";
import { InProgressPanel } from "./in-progress-panel";
import { NeedsYou } from "./needs-you";
import { PendingSetups } from "./pending-setups";
import { SlotsCaption } from "./slots-caption";

export const FLEET_BLOCK_ID = "devices-fleet-list";

const FLEET_HOME: FleetRoute = { screen: "fleet", view: "devices" };

interface Notice {
	tone: ResultTone;
	text: string;
}

/* Header. */

/** The signed-in person as the identity provider names them; nothing when it sends no name. */
function useSignedInName() {
	const auth = useContext(AuthContext);
	if (!auth?.user) return undefined;
	const { name, preferred_username: handle, email } = auth.user.profile;
	return name || handle || email;
}

interface NoticeState {
	busy: boolean;
	notice: Notice | null;
}

const NO_NOTICE: NoticeState = { busy: false, notice: null };

/**
 * The inline result of one page-level request (R9). It belongs to the
 * workspace it started in: switching account or hub clears it and drops an
 * answer that arrives afterwards.
 */
function useWorkspaceNotice() {
	const workspace = useDeviceWorkspace();
	const mounted = useRef(workspace);
	const [state, setState] = useState(NO_NOTICE);
	useEffect(() => {
		mounted.current = workspace;
		setState(NO_NOTICE);
	}, [workspace]);
	const run = async (request: () => Promise<Notice>) => {
		if (state.busy) return;
		const started = mounted.current;
		setState({ busy: true, notice: null });
		const notice = await request();
		if (mounted.current === started) setState({ busy: false, notice });
	};
	return { ...state, run, dismiss: () => setState(NO_NOTICE) };
}

/** "Checked at 14:00:31. The device list is current." next to Refresh, until dismissed (R9). */
function useRefresh() {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const workspace = useDeviceWorkspace();
	const { freshness } = useDeviceRows();
	const { rows } = useAttentionState();
	const result = useWorkspaceNotice();
	const read = async (): Promise<Notice> => {
		// The answer of this very read: the hook's state follows a render later.
		const outcome = (await rows.refetch().catch((error: unknown) => ({
			error,
		}))) as { error?: unknown } | undefined;
		const at = time.clock(workspace.clock.now() / 1000);
		if (!outcome?.error)
			return {
				tone: "good",
				text: t(
					"fleet.refresh.done",
					"Checked at {{time}}. The device list is current.",
					{ time: at },
				),
			};
		const reason = hubErrorCopy(t, toHubError(outcome.error).code);
		// A failed read keeps the rows it had: their time is the one known before.
		const from = freshness.dataFrom ?? freshness.at;
		return {
			tone: "warning",
			text:
				from === undefined
					? t(
							"fleet.refresh.failed",
							"Couldn't refresh at {{time}}: {{reason}}",
							{
								time: at,
								reason,
							},
						)
					: t(
							"fleet.refresh.failedKept",
							"Couldn't refresh at {{time}}: {{reason}} Showing data from {{from}}.",
							{ time: at, reason, from: time.clock(from) },
						),
		};
	};
	return {
		busy: result.busy,
		notice: result.notice,
		refresh: () => result.run(read),
		dismiss: result.dismiss,
	};
}

interface HeaderProps {
	/** `undefined` while the list has not loaded. */
	entries?: readonly FleetDeviceEntry[];
	/** The empty state carries the two start actions itself (R2: one coral per view). */
	startActions?: boolean;
}

function FleetHeader({ entries, startActions = true }: Readonly<HeaderProps>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const { href, navigate } = useDevicesRoute();
	const { host } = useHubSupport();
	const name = useSignedInName();
	const refresh = useRefresh();
	const shared = entries?.filter(
		({ view }) => view.relationship === "shared",
	).length;
	const sub = [
		entries === undefined
			? t("fleet.header.on", "Devices on {{host}}", { host })
			: t("fleet.header.count", {
					count: entries.length,
					host,
					defaultValue_one: "{{count, number}} device on {{host}}",
					defaultValue_other: "{{count, number}} devices on {{host}}",
				}),
		shared
			? t("fleet.header.shared", "{{count, number}} shared with you", {
					count: shared,
				})
			: "",
		name ? t("fleet.header.signedIn", "signed in as {{name}}", { name }) : "",
	]
		.filter(Boolean)
		.join(" · ");
	return (
		<PageHeader
			crumbs={[
				{
					label: t("fleet.header.crumbDevices", "Devices"),
					href: href(FLEET_HOME),
					onNavigate: () => navigate(FLEET_HOME),
				},
				{ label: t("fleet.title", "Fleet overview") },
			]}
			title={t("fleet.title", "Fleet overview")}
			sub={sub}
			actions={
				<>
					<ScopeSwitch />
					<DvButton
						icon={RefreshCw}
						busy={refresh.busy}
						onClick={() => void refresh.refresh()}
					>
						{t("fleet.header.refresh", "Refresh")}
					</DvButton>
					{startActions ? (
						<>
							<DvButton icon={UserPlus} asChild>
								<a
									{...link({
										screen: "access",
										tab: "shared",
										action: "request",
									})}
								>
									{t("fleet.header.request", "Request shared access")}
								</a>
							</DvButton>
							<DvButton
								variant="primary"
								icon={Plus}
								asChild
								className="@max-[720px]/devices:order-first @max-[720px]/devices:w-full"
							>
								<a {...link({ screen: "setup" })}>
									{t("fleet.header.setup", "Set up a device")}
								</a>
							</DvButton>
						</>
					) : null}
					{refresh.notice ? (
						<InlineResult
							tone={refresh.notice.tone}
							onDismiss={refresh.dismiss}
							className="basis-full"
						>
							{refresh.notice.text}
						</InlineResult>
					) : null}
				</>
			}
		/>
	);
}

/* Web only: the browser may evict the keys (IA §6.5 `storage_not_persistent`). */

const UNSAFE_STORAGE = new Set(["denied", "unavailable"]);

/** "Keep keys safely": asks the browser for persistent storage and says what it answered. */
function useKeepKeys(count: number) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const workspace = useDeviceWorkspace();
	const result = useWorkspaceNotice();
	const answer = (granted: string | undefined, at: string): Notice => {
		if (granted === "persisted")
			return {
				tone: "good",
				text: t("fleet.keysRisk.granted", {
					count,
					time: at,
					defaultValue_one:
						"The browser granted persistent storage at {{time}}. Keys for {{count, number}} device are kept safely here.",
					defaultValue_other:
						"The browser granted persistent storage at {{time}}. Keys for {{count, number}} devices are kept safely here.",
				}),
			};
		if (granted === undefined)
			return {
				tone: "warning",
				text: t(
					"fleet.keysRisk.failed",
					"Couldn't ask the browser. Back up your keys so you can restore them.",
				),
			};
		return {
			tone: "warning",
			text: t(
				"fleet.keysRisk.denied",
				"The browser didn't grant persistent storage at {{time}}. Back up your keys so you can restore them.",
				{ time: at },
			),
		};
	};
	const ask = async () => {
		const granted = await workspace.local
			.requestPersistence()
			.catch(() => undefined);
		return answer(granted, time.clock(workspace.clock.now() / 1000));
	};
	return {
		busy: result.busy,
		notice: result.notice,
		keep: () => result.run(ask),
		dismiss: result.dismiss,
	};
}

function KeysRiskBanner() {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const local = useLocalSummary();
	const count = local.vaults.length;
	const { busy, notice, keep, dismiss } = useKeepKeys(count);
	const atRisk =
		local.platform === "web" &&
		count > 0 &&
		UNSAFE_STORAGE.has(local.persistence);
	const result = notice ? (
		<InlineResult tone={notice.tone} onDismiss={dismiss}>
			{notice.text}
		</InlineResult>
	) : null;
	if (!atRisk) return result;
	return (
		<Banner
			tone="warning"
			title={t(
				"fleet.keysRisk.title",
				"This browser may delete your device keys.",
			)}
			actions={
				<>
					{local.persistence === "denied" ? (
						<DvButton
							size="sm"
							icon={ShieldCheck}
							busy={busy}
							onClick={() => void keep()}
						>
							{t("fleet.keysRisk.keep", "Keep keys safely")}
						</DvButton>
					) : null}
					<DvButton size="sm" icon={CloudUpload} asChild>
						<a {...link({ screen: "keys" })}>
							{t("fleet.keysRisk.backUp", "Back up keys")}
						</a>
					</DvButton>
					{result}
				</>
			}
		>
			{t("fleet.keysRisk.text", {
				count,
				defaultValue_one:
					"This site hasn't been granted persistent storage, so clearing site data or running low on disk can remove the keys for {{count, number}} device. Without them you can't manage it from here.",
				defaultValue_other:
					"This site hasn't been granted persistent storage, so clearing site data or running low on disk can remove the keys for {{count, number}} devices. Without them you can't manage those devices from here.",
			})}
		</Banner>
	);
}

/* Annunciator. */

function useAnnunciatorCells(
	entries: readonly FleetDeviceEntry[],
): Record<AnnunciatorWindow, AnnunciatorCell> {
	const { t } = useTranslation("devices");
	return useMemo(() => {
		const names = (match: (entry: FleetDeviceEntry) => boolean) =>
			entries.filter(match).map((entry) => entry.name);
		const cell = (list: string[], extra: Partial<AnnunciatorCell> = {}) => ({
			count: list.length,
			names: list,
			...extra,
		});
		const unknown = entries.filter(({ view }) => view.health === "unknown");
		const offline = entries.filter(
			({ view }) => view.presence.kind === "offline",
		);
		const offlineCritical = offline.filter(
			({ view }) => view.health === "critical",
		).length;
		const revoked = entries.filter(({ view }) => view.health === "revoked");
		const billed = revoked.filter(({ view }) =>
			view.attention.some(
				(item) => item.key === "you_still_pay_for_a_revoked_device",
			),
		).length;
		const allLocked =
			unknown.length > 0 &&
			unknown.every(({ view }) => view.keys.state !== "none");
		return {
			critical: cell(names(({ view }) => view.health === "critical")),
			attention: cell(names(({ view }) => view.health === "attention")),
			unknown: cell(
				unknown.map((entry) => entry.name),
				allLocked ? { note: t("fleet.annunciator.locked", "locked") } : {},
			),
			healthy: cell(names(({ view }) => view.health === "healthy")),
			offline: cell(
				offline.map((entry) => entry.name),
				offlineCritical === 0
					? {}
					: {
							note:
								offlineCritical === offline.length
									? t("fleet.annunciator.alsoCritical", "also critical")
									: t(
											"fleet.annunciator.someCritical",
											"{{count, number}} also critical",
											{ count: offlineCritical },
										),
						},
			),
			revoked: cell(
				revoked.map((entry) => entry.name),
				billed > 0
					? {
							sub: t(
								"fleet.annunciator.billed",
								"{{count, number}} still billed to you",
								{ count: billed },
							),
						}
					: {},
			),
		};
	}, [entries, t]);
}

const WINDOW_FILTERS = new Set<FleetFilter>([
	"critical",
	"attention",
	"unknown",
	"healthy",
	"offline",
	"revoked",
]);

function FleetAnnunciator({
	route,
	entries,
}: Readonly<{ route: FleetRoute; entries: readonly FleetDeviceEntry[] }>) {
	const { t } = useTranslation("devices");
	const { navigate } = useDevicesRoute();
	const cells = useAnnunciatorCells(entries);
	const pressed =
		route.view === "devices" && route.filter && WINDOW_FILTERS.has(route.filter)
			? (route.filter as AnnunciatorWindow)
			: null;
	return (
		<Annunciator
			cells={cells}
			pressed={pressed}
			onSelect={(window) => {
				const { filter: _filter, ...rest } = route;
				navigate(
					{ ...rest, view: "devices", ...(window ? { filter: window } : {}) },
					{ replace: true },
				);
				if (window)
					globalThis.document
						?.getElementById(FLEET_BLOCK_ID)
						?.scrollIntoView?.({ block: "start", behavior: "smooth" });
			}}
			caption={
				<>
					{t(
						"fleet.annunciator.caption",
						"Health counts each device once, using its worst open item. Offline counts check-ins and can overlap with health.",
					)}{" "}
					<SlotsCaption />
				</>
			}
		/>
	);
}

/* Devices / Services block. */

function FleetList({
	route,
	entries,
}: Readonly<{ route: FleetRoute; entries: readonly FleetDeviceEntry[] }>) {
	const { t } = useTranslation("devices");
	const { navigate } = useDevicesRoute();
	const { host } = useHubSupport();
	const rows = useDeviceRows();
	const fleet = useFleetServices(entries);
	const devices = route.view === "devices";
	const setView = (view: FleetView) => {
		const { q: _q, filter: _filter, ...rest } = route;
		navigate({ ...rest, view }, { replace: true });
	};
	return (
		<Block
			id={FLEET_BLOCK_ID}
			title={
				<span className="sr-only">
					{t("fleet.list.title", "Devices and services")}
				</span>
			}
			summary={
				<Segmented<FleetView>
					label={t("fleet.list.view", "View")}
					value={route.view}
					onChange={setView}
					options={[
						{
							value: "devices",
							label: t("fleet.list.devices", "Devices"),
							icon: Server,
							count: entries.length,
						},
						{
							value: "services",
							label: t("fleet.list.services", "Services"),
							icon: Boxes,
							count: fleet.services.length,
						},
					]}
				/>
			}
			stamp={
				devices ? (
					<FreshnessStamp {...stampOf(rows.freshness)} />
				) : fleet.base ? (
					<FreshnessStamp {...fleet.base} />
				) : (
					<MixedSourcesStamp />
				)
			}
			flush
			className="scroll-mt-4"
			foot={
				devices
					? t(
							"fleet.list.footDevices",
							"Name, status, check-ins and certificate expiry come from the hub and need no password. Services and agent versions need keys on this computer.",
						)
					: t(
							"fleet.list.footServices",
							"Requested is what you asked for; actual is what the device reports. The agent restarts crashed instances on its own, up to 5 times.",
						)
			}
		>
			{devices ? (
				<FleetDevicesTable route={route} entries={entries} host={host} />
			) : (
				<FleetServicesTable route={route} fleet={fleet} />
			)}
		</Block>
	);
}

/* States. */

function FleetEmpty() {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	return (
		<StateView
			kind="empty"
			icon={Server}
			title={t("fleet.empty.title", "No devices yet")}
			text={t(
				"fleet.empty.text",
				"A device is a computer you run Flow-Like apps on, like a Raspberry Pi, a server or a Mac. Set one up here, or ask someone to share theirs.",
			)}
			actions={
				<>
					<DvButton variant="primary" icon={Plus} asChild>
						<a {...link({ screen: "setup" })}>
							{t("fleet.header.setup", "Set up a device")}
						</a>
					</DvButton>
					<DvButton icon={UserPlus} asChild>
						<a
							{...link({ screen: "access", tab: "shared", action: "request" })}
						>
							{t("fleet.empty.request", "Request access to someone's device")}
						</a>
					</DvButton>
					<DvButton variant="ghost" icon={Server} asChild>
						<a {...link({ screen: "hub" })}>
							{t("fleet.empty.hub", "Check hub status")}
						</a>
					</DvButton>
				</>
			}
		/>
	);
}

/** SPEC §6.4: the page's conclusion; on its own so the area clock re-renders only this sentence. */
function FleetHeadline() {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { input, items } = useAttentionState();
	const model = useMemo(() => headline(input, { items }), [input, items]);
	const sentence = useMemo(
		() => headlineCopy(t, model, time),
		[t, model, time],
	);
	const names = useMemo(() => headlineNames(model), [model]);
	return <Headline lead={sentence.lead} rest={sentence.rest} names={names} />;
}

function FleetLoaded({
	route,
	entries,
}: Readonly<{ route: FleetRoute; entries: readonly FleetDeviceEntry[] }>) {
	return (
		<>
			<FleetHeadline />
			<FleetAnnunciator route={route} entries={entries} />
			<div className="grid min-w-0 grid-cols-[minmax(0,1fr)_380px] items-start gap-6 @max-[1024px]/devices:grid-cols-1">
				<NeedsYou focus={route.focus === "attention"} />
				<div className="flex min-w-0 flex-col gap-4">
					<InProgressPanel />
					<EncryptedStatusExceptions entries={entries} />
				</div>
			</div>
			<FleetList route={route} entries={entries} />
			{route.view === "devices" ? <PendingSetups /> : null}
		</>
	);
}

/** SPEC §5.1 / IA §6.2 N1: the daily triage page over every device of the account. */
export function FleetScreen({ route }: Readonly<ScreenProps>) {
	const { t } = useTranslation("devices");
	const rows = useDeviceRows();
	const entries = useFleetEntries();
	const fleetRoute = route.screen === "fleet" ? route : FLEET_HOME;
	const loaded = rows.rows !== undefined;

	if (!loaded)
		return (
			<div className={PAGE_STACK}>
				<FleetHeader />
				{rows.error ? (
					<StateView
						kind="error"
						title={t("fleet.error.title", "Couldn't read the device list")}
						text={hubErrorCopy(t, rows.error.code)}
						actions={
							<DvButton size="sm" onClick={() => void rows.refetch()}>
								{t("fleet.error.retry", "Try again")}
							</DvButton>
						}
					/>
				) : (
					<StateView
						kind="loading"
						title={t("fleet.loading", "Reading the device list…")}
						rows={6}
					/>
				)}
			</div>
		);

	if (entries.length === 0)
		return (
			<div className={PAGE_STACK}>
				<KeysRiskBanner />
				<FleetHeader entries={entries} startActions={false} />
				<FleetEmpty />
				<PendingSetups />
			</div>
		);

	return (
		<div className={PAGE_STACK}>
			<KeysRiskBanner />
			<FleetHeader entries={entries} />
			<FleetLoaded route={fleetRoute} entries={entries} />
		</div>
	);
}

/** The page's sections sit 24 px apart, like the Hub status and Keys pages. */
const PAGE_STACK = "flex min-w-0 flex-col gap-6";
