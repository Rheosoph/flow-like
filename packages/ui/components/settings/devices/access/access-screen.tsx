"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { Plus, Share2, UserPlus } from "lucide-react";
import {
	Fragment,
	useCallback,
	useContext,
	useEffect,
	useMemo,
	useState,
} from "react";
import { AuthContext } from "react-oidc-context";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import type {
	AccessRoute,
	AccessTab,
	AttentionKey,
} from "../../../../lib/device-management/model/types";
import {
	type AccessHeadline,
	accessHeadline,
} from "../../../../lib/device-management/sharing";
import { TabsContent } from "../../../ui/tabs";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { PageHeader } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { Headline } from "../primitives/headline";
import { cx } from "../primitives/tone";
import { type UnderlineTab, UnderlineTabs } from "../primitives/underline-tabs";
import { useDevicesRoute, useRouteLink } from "../routing/use-devices-route";
import type { ScreenProps } from "../screen-props";
import { useAttention } from "../workspace/use-attention";
import { useResourceSummary } from "../workspace/use-hub";
import { OBJECT_LINK } from "./access-parts";
import { AccessWizard, type AccessWizardStart } from "./add-people-sheet";
import { nextWizardId } from "./change-permissions-sheet";
import { AccessCloudTab } from "./cloud-tab";
import {
	ImportFileSheet,
	type RequestFileRow,
	requestsOfRows,
} from "./import-file-sheet";
import { PeopleTab } from "./people-tab";
import { RequestAccessFlow } from "./request-access-flow";
import { SharedWithMeTab } from "./shared-with-me-tab";
import {
	type DeviceAccess,
	type FleetAccess,
	type PersonNames,
	useAccessLocal,
	useFleetAccess,
	useOwnedAccess,
	usePersonNames,
} from "./use-access";

const ACCESS_HOME: AccessRoute = { screen: "access", tab: "people" };
/** PROTO `.main-in`: 24 px between header, headline, tabs and panel (the shell's column has 16 px). */
const PAGE = "flex min-w-0 flex-col gap-6";
const PANEL = "mt-6 flex flex-col gap-4";

const PEOPLE_KEYS: ReadonlySet<AttentionKey> = new Set([
	"grant_expiring",
	"sharing_policy_waiting_for_device",
	"sharing_policy_expiring",
	"sharing_policy_expired",
	"access_slots_nearly_full",
]);
const SHARED_KEYS: ReadonlySet<AttentionKey> = new Set([
	"shared_access_expiring",
	"shared_access_ended",
	"access_request_pending",
]);
const CLOUD_KEYS: ReadonlySet<AttentionKey> = new Set([
	"cloud_access_invalid",
	"cloud_access_ending",
	"spending_limit_low",
	"spending_limit_ending",
	"online_files_read_only",
]);

function useSignedInName(): string | undefined {
	const profile = useContext(AuthContext)?.user?.profile;
	return profile?.name ?? profile?.preferred_username ?? profile?.email;
}

function subLine(
	t: DevicesT,
	fleet: FleetAccess,
	approvals: number | undefined,
	name: string | undefined,
): string {
	return [
		fleet.loaded
			? t("devices:access.header.owned", {
					count: fleet.owned.length,
					defaultValue_one: "{{count, number}} device you own",
					defaultValue_other: "{{count, number}} devices you own",
				})
			: "",
		fleet.loaded
			? t("devices:access.header.shared", "{{count, number}} shared with you", {
					count: fleet.shared.length,
				})
			: "",
		approvals === undefined
			? ""
			: t("devices:access.header.approvals", {
					count: approvals,
					defaultValue_one: "{{count, number}} cloud approval",
					defaultValue_other: "{{count, number}} cloud approvals",
				}),
		name
			? t("devices:access.header.signedIn", "signed in as {{name}}", { name })
			: "",
	]
		.filter(Boolean)
		.join(" · ");
}

function DeviceLinks({
	devices,
}: Readonly<{ devices: readonly { deviceId: string; name: string }[] }>) {
	const link = useRouteLink();
	return (
		<>
			{devices.map((device, index) => (
				<Fragment key={device.deviceId}>
					{index ? ", " : null}
					<a
						{...link({
							screen: "device",
							deviceId: device.deviceId,
							tab: "access",
						})}
						className={cx(OBJECT_LINK, "text-[0.9em]")}
					>
						{device.name}
					</a>
				</Fragment>
			))}
		</>
	);
}

type HeadlineDevice = Readonly<{ deviceId: string; name: string }>;

/** "Mira Novak's access to edge-berlin-01 ends at 20:00, and studio-mac-mini hasn't applied your last change yet." */
function ReachSentence({
	soon,
	waiting,
	names,
}: Readonly<{
	soon: AccessHeadline["soon"];
	waiting: readonly HeadlineDevice[];
	names: PersonNames;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const waitingLinks = <DeviceLinks devices={waiting} />;
	if (!soon)
		return waiting.length === 1 ? (
			<Trans
				t={t}
				i18nKey="access.headline.waitingOne"
				defaults="<1/> hasn't applied your last change yet."
				components={{ 1: waitingLinks }}
			/>
		) : (
			<Trans
				t={t}
				i18nKey="access.headline.waitingSeveral"
				defaults="<1/> haven't applied your last changes yet."
				components={{ 1: waitingLinks }}
			/>
		);
	const values = {
		name: names(soon.userId).name,
		time: time.at(soon.expiresAt),
	};
	const soonLink = (
		<DeviceLinks
			devices={[{ deviceId: soon.deviceId, name: soon.deviceName }]}
		/>
	);
	if (!waiting.length)
		return (
			<Trans
				t={t}
				i18nKey="access.headline.soon"
				defaults="{{name}}'s access to <1/> ends at {{time}}."
				values={values}
				components={{ 1: soonLink }}
			/>
		);
	return waiting.length === 1 ? (
		<Trans
			t={t}
			i18nKey="access.headline.soonAndWaitingOne"
			defaults="{{name}}'s access to <1/> ends at {{time}}, and <2/> hasn't applied your last change yet."
			values={values}
			components={{ 1: soonLink, 2: waitingLinks }}
		/>
	) : (
		<Trans
			t={t}
			i18nKey="access.headline.soonAndWaitingSeveral"
			defaults="{{name}}'s access to <1/> ends at {{time}}, and <2/> haven't applied your last changes yet."
			values={values}
			components={{ 1: soonLink, 2: waitingLinks }}
		/>
	);
}

/** SPEC §6.4: the page's one conclusion about who can reach the viewer's devices. */
function AccessConclusion({
	fleet,
	headline,
	names,
}: Readonly<{
	fleet: FleetAccess;
	headline: AccessHeadline;
	names: PersonNames;
}>) {
	const { t } = useTranslation("devices");
	if (!fleet.loaded) return null;
	if (!fleet.owned.length && !fleet.shared.length)
		return (
			<Headline
				lead={t(
					"access.headline.none",
					"You don't own any devices, and none are shared with you.",
				)}
				rest={t(
					"access.headline.noneRest",
					"Set up a device to share it, or ask an owner for access to theirs.",
				)}
			/>
		);
	if (!fleet.owned.length)
		return (
			<Headline
				lead={t("access.headline.sharedOnly", {
					count: fleet.shared.length,
					defaultValue_one: "{{count, number}} device is shared with you.",
					defaultValue_other: "{{count, number}} devices are shared with you.",
				})}
				rest={t(
					"access.headline.sharedOnlyRest",
					"You don't own a device, so there is nobody to give access to.",
				)}
			/>
		);
	const { people, soon, waiting, unread } = headline;
	const lead = people
		? t("access.headline.people", {
				count: people,
				defaultValue_one: "{{count, number}} person can reach your devices.",
				defaultValue_other: "{{count, number}} people can reach your devices.",
			})
		: unread.length
			? t(
					"access.headline.unread",
					"Unlock your shared devices to see who can reach them.",
				)
			: t("access.headline.onlyYou", "Only you can reach your devices.");
	const reach =
		soon || waiting.length ? (
			<ReachSentence soon={soon} waiting={waiting} names={names} />
		) : null;
	const locked =
		unread.length && people ? (
			<Trans
				t={t}
				i18nKey="access.headline.unreadRest"
				defaults="Unlock <1/> to see who else has access there."
				components={{ 1: <DeviceLinks devices={unread} /> }}
			/>
		) : null;
	const rest =
		reach || locked ? (
			<>
				{reach}
				{reach && locked ? " " : null}
				{locked}
			</>
		) : unread.length ? undefined : (
			t(
				"access.headline.calm",
				"Every device uses the access rules you saved, and nobody's access ends in the next 12 hours.",
			)
		);
	return <Headline lead={lead} rest={rest} />;
}

function useTabs(
	t: DevicesT,
	requests: number,
	pending: number,
): UnderlineTab<AccessTab>[] {
	const items = useAttention();
	return useMemo(() => {
		const count = (keys: ReadonlySet<AttentionKey>) =>
			items.filter((item) => keys.has(item.key)).length;
		const people = count(PEOPLE_KEYS) + requests;
		const shared = Math.max(count(SHARED_KEYS), pending);
		const cloud = count(CLOUD_KEYS);
		return [
			{
				value: "people",
				label: t("devices:access.tab.people", "People"),
				...(people
					? {
							count: {
								count: people,
								tone: "info" as const,
								label: t(
									"devices:access.tab.peopleCount",
									"{{count, number}} to look at: requests, ending access or changes a device hasn't applied",
									{ count: people },
								),
							},
						}
					: {}),
			},
			{
				value: "shared",
				label: t("devices:access.tab.shared", "Shared with me"),
				shortLabel: t("devices:access.tab.sharedShort", "Shared"),
				...(shared
					? {
							count: {
								count: shared,
								tone: "info" as const,
								label: t(
									"devices:access.tab.sharedCount",
									"{{count, number}} waiting for an owner or ending soon",
									{ count: shared },
								),
							},
						}
					: {}),
			},
			{
				value: "cloud",
				label: t("devices:access.tab.cloud", "Cloud approvals & spending"),
				shortLabel: t("devices:access.tab.cloudShort", "Cloud"),
				...(cloud
					? {
							count: {
								count: cloud,
								tone: "warning" as const,
								label: t("devices:access.tab.cloudCount", {
									count: cloud,
									defaultValue_one:
										"{{count, number}} cloud approval or spending limit needs a look",
									defaultValue_other:
										"{{count, number}} cloud approvals or spending limits need a look",
								}),
							},
						}
					: {}),
			},
		];
	}, [items, requests, pending, t]);
}

function AccessHeader({
	fleet,
	onAdd,
	onRequest,
}: Readonly<{ fleet: FleetAccess; onAdd(): void; onRequest(): void }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const { href, navigate } = useDevicesRoute();
	const name = useSignedInName();
	const summary = useResourceSummary();
	const approvals = summary.data?.devices.reduce(
		(total, device) =>
			total +
			device.approvals.filter((approval) => approval.status === "active")
				.length,
		0,
	);
	const home = { screen: "fleet", view: "devices" } as const;
	return (
		<PageHeader
			crumbs={[
				{
					label: t("access.header.crumbDevices", "Devices"),
					href: href(home),
					onNavigate: () => navigate(home),
				},
				{ label: t("access.title", "Access") },
			]}
			title={t("access.title", "Access")}
			sub={subLine(t, fleet, approvals, name)}
			actions={
				<>
					<DvButton icon={Share2} onClick={onRequest}>
						{t("access.action.request", "Request shared access")}
					</DvButton>
					{fleet.loaded && !fleet.owned.length ? (
						<DvButton variant="primary" icon={Plus} asChild>
							<a {...link({ screen: "setup" })}>
								{t("access.action.setup", "Set up a device")}
							</a>
						</DvButton>
					) : (
						<DvButton variant="primary" icon={UserPlus} onClick={onAdd}>
							{t("access.action.addPeople", "Add people…")}
						</DvButton>
					)}
				</>
			}
		/>
	);
}

function peopleIds(
	devices: readonly DeviceAccess[],
	fleet: FleetAccess,
	requestUsers: readonly string[],
	owners: readonly string[],
): string[] {
	return [
		...devices.flatMap(
			(device) => device.rows?.map((row) => row.grant.user_id) ?? [],
		),
		...fleet.shared.map((row) => row.owner_id),
		...requestUsers,
		...owners,
	];
}

/** SPEC §5.7 / IA §6.2 N7: who can do what on which device, for the viewer's devices and the ones shared with them. */
export function AccessScreen({ route, scope }: Readonly<ScreenProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { navigate, clearParam } = useDevicesRoute();
	const accessRoute = route.screen === "access" ? route : ACCESS_HOME;
	const tab = accessRoute.tab ?? "people";
	const fleet = useFleetAccess();
	const devices = useOwnedAccess(fleet.owned);
	const local = useAccessLocal();
	const pendingRequests = useAttention().filter(
		(item) => item.key === "access_request_pending",
	).length;
	const [wizard, setWizard] = useState<AccessWizardStart | null>(null);
	const [requesting, setRequesting] = useState(false);
	const [importing, setImporting] = useState(false);

	const names = usePersonNames(
		t,
		useMemo(
			() =>
				peopleIds(
					devices,
					fleet,
					local.requests.map((request) => request.userId),
					local.changes.flatMap((change) =>
						change.entries.map((entry) => entry.userId),
					),
				),
			[devices, fleet, local.requests, local.changes],
		),
	);
	const headline = useMemo(
		() =>
			accessHeadline(
				devices.map((device) => ({
					deviceId: device.deviceId,
					name: device.name,
					rules: device.rules,
					...(device.rows ? { rows: device.rows } : {}),
				})),
				Math.floor(time.nowS / 30) * 30,
			),
		[devices, time.nowS],
	);
	const tabs = useTabs(t, local.requests.length, pendingRequests);

	const setTab = useCallback(
		(next: AccessTab) =>
			navigate({ screen: "access", tab: next }, { replace: true }),
		[navigate],
	);
	const startAdd = useCallback(
		() => setWizard({ id: nextWizardId(), mode: "add" }),
		[],
	);
	const startRequest = useCallback(() => {
		if (tab !== "shared") setTab("shared");
		setRequesting(true);
	}, [tab, setTab]);

	// One-shot deep links: open the flow, then drop the param so a reload doesn't reopen it.
	const { action, import: importKind } = accessRoute;
	useEffect(() => {
		if (!action && !importKind) return;
		if (action === "add-people") startAdd();
		if (importKind === "request") setImporting(true);
		if (action === "request" || importKind === "connection") {
			// The request ends as a pending row on Shared with me.
			setRequesting(true);
			navigate({ screen: "access", tab: "shared" }, { replace: true });
			return;
		}
		clearParam("action", "import");
	}, [action, importKind, startAdd, clearParam, navigate]);

	const onImported = useCallback(
		(rows: RequestFileRow[]) => {
			setImporting(false);
			local.store.addRequests(requestsOfRows(rows, Math.floor(time.nowS)));
			const byId = new Map(devices.map((device) => [device.deviceId, device]));
			setWizard({
				id: nextWizardId(),
				mode: "add",
				step: "files",
				deviceIds: [
					...new Set(rows.flatMap((row) => row.deviceId ?? [])),
				].filter((id) => byId.get(id)?.keys.state === "unlocked"),
				files: rows,
			});
		},
		[devices, local.store, time.nowS],
	);

	return (
		<div className={PAGE}>
			<AccessHeader fleet={fleet} onAdd={startAdd} onRequest={startRequest} />
			<AccessConclusion fleet={fleet} headline={headline} names={names} />
			<UnderlineTabs
				label={t("access.tabs", "Access sections")}
				tabs={tabs}
				value={tab}
				onValueChange={setTab}
			>
				<TabsContent value="people" className={PANEL}>
					<PeopleTab
						fleet={fleet}
						devices={devices}
						names={names}
						onWizard={setWizard}
					/>
				</TabsContent>
				<TabsContent value="shared" className={PANEL}>
					<SharedWithMeTab
						fleet={fleet}
						names={names}
						onRequest={startRequest}
						onCloud={() => setTab("cloud")}
					/>
				</TabsContent>
				<TabsContent value="cloud" className={PANEL}>
					<AccessCloudTab route={route} scope={scope} />
				</TabsContent>
			</UnderlineTabs>
			<AccessWizard
				start={wizard}
				devices={devices}
				sharedNames={fleet.shared.map((row) => deviceName(row))}
				onClose={() => setWizard(null)}
			/>
			<RequestAccessFlow
				open={requesting}
				onClose={() => setRequesting(false)}
			/>
			<ImportFileSheet
				open={importing}
				onOpenChange={setImporting}
				ownedIds={fleet.owned.map((row) => row.device_id)}
				onImported={onImported}
			/>
		</div>
	);
}
