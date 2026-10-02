"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	ArrowRight,
	Cloud,
	Copy,
	Ellipsis,
	KeyRound,
	Lock,
	LockOpen,
	type LucideIcon,
	Power,
	Rocket,
	Stethoscope,
} from "lucide-react";
import { type MouseEvent, memo } from "react";
import { useUserIdentity } from "../../../../hooks/use-user-lookup";
import {
	type PublicCertificateInventory,
	certificateStatus,
} from "../../../../lib/device-management/certificates";
import type {
	AgeState,
	AttentionItem,
	CopyRef,
	DeviceRow,
	DeviceViewModel,
	DevicesRoute,
	Freshness,
	FreshnessReason,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import type {
	Capability,
	ManagementGrant,
} from "../../../../lib/device-management/types";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../../../ui/dropdown-menu";
import { formatMoney } from "../copy/attention-copy";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import {
	CheckinLane,
	LANE_SLOTS,
	LANE_SLOT_SEC,
	type LaneTick,
} from "../primitives/checkin-lane";
import { DvButton } from "../primitives/dv-button";
import { CellSub, Td, Tr } from "../primitives/dv-table";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { IdRef } from "../primitives/id-ref";
import { SeverityWord } from "../primitives/severity-word";
import {
	HealthChip,
	KeyChip,
	PresenceChip,
	RelationshipChip,
} from "../primitives/status-chip";
import { cx } from "../primitives/tone";
import { useCopy } from "../primitives/use-copy";
import { useRouteLink } from "../routing/use-devices-route";
import { MENU_CONTENT_CLASS, MENU_ITEM_CLASS } from "../shell/area-nav";
import { stampOf } from "../shell/attention-popover";
import { isKeySessionOpen, keyChipOf } from "../shell/keys-popover";
import { attentionShort } from "../shell/rail-row";
import {
	useCertificateInventory,
	useMyAccess,
	useOverlayStore,
} from "../workspace";

/**
 * SPEC §5.1 column plan: Device · Health · Check-in · Services · Certificates · Access & keys · Agent · menu.
 * At 1440 Device holds the ID and the registration day on one line and
 * Check-in a dated "Offline since …"; Health holds its widest chip down to 1280.
 */
export const FLEET_DEVICE_COLS = [
	"18.5%",
	"14%",
	"15.5%",
	"14.5%",
	"12.5%",
	"12%",
	"9%",
	"auto",
] as const;

/** A chip primitive rendered as plain cell text (the table's own type scale, no pill). */
const PLAIN_CHIP =
	"h-auto max-w-full rounded-none border-0 bg-transparent p-0 text-ui font-normal text-inherit [&>span]:overflow-visible [&>span]:whitespace-normal";
/** The same without the chip's icon ("Yours", "Shared by Mira Novak · ends in 14 h"). */
const PLAIN_TEXT = `${PLAIN_CHIP} [&>svg]:hidden`;
/** A chip inside a table cell: 4 px corners, and it wraps instead of clipping (R10). */
export const CELL_CHIP =
	"h-auto min-h-5.5 rounded-md py-0.5 whitespace-normal [&>span]:overflow-visible [&>span]:whitespace-normal";
/**
 * A line of facts separated by "·" that may wrap: the dot sits in each
 * following fact's left gutter, which is clipped at the start of a line.
 */
const FACT_LINE = "-ml-1 overflow-x-clip pl-1";
const FACT_ROW = "-ml-3 flex flex-wrap items-center gap-y-0.5";
const FACT_FIRST = "pl-3";
const FACT_NEXT =
	"relative pl-3 whitespace-nowrap before:absolute before:left-1 before:content-['·']";

/**
 * The hub keeps one check-in time per device, so the lane states only what
 * that proves: every quarter-hour after it was missed, the one it falls in was
 * kept, and nothing is claimed about the ones before.
 */
export function laneTicksOf(
	row: Pick<DeviceRow, "status" | "registered_at" | "last_seen_at">,
	nowS: number,
): LaneTick[] {
	if (row.status === "revoked") return [];
	const slotOf = (atS: number) =>
		LANE_SLOTS - 1 - Math.floor(Math.max(0, nowS - atS) / LANE_SLOT_SEC);
	const registered = slotOf(row.registered_at);
	const last = row.last_seen_at === null ? null : slotOf(row.last_seen_at);
	return Array.from({ length: LANE_SLOTS }, (_, slot): LaneTick => {
		if (slot < registered) return "pre";
		if (last === null || slot > last) return "miss";
		return slot === last ? "ok" : "pre";
	});
}

const dayFormatters = new Map<string, Intl.DateTimeFormat>();

/** "2 Jun" (the year only when it differs from now). */
export function dayLabel(atS: number, nowMs: number, locale: string): string {
	const date = new Date(atS * 1000);
	const withYear = date.getFullYear() !== new Date(nowMs).getFullYear();
	const key = `${locale}|${withYear}`;
	let formatter = dayFormatters.get(key);
	if (!formatter) {
		formatter = new Intl.DateTimeFormat(locale, {
			day: "numeric",
			month: "short",
			year: withYear ? "numeric" : undefined,
		});
		dayFormatters.set(key, formatter);
	}
	return formatter.format(date);
}

/** The agent's operating system as words (R3: never the wire value). */
export function platformLabel(t: DevicesT, os: string | undefined): string {
	const labels: Record<string, string> = {
		linux: t("devices:fleet.platform.linux", "Linux"),
		macos: t("devices:fleet.platform.macos", "Mac"),
		windows: t("devices:fleet.platform.windows", "Windows"),
	};
	return (
		(os ? labels[os] : undefined) ??
		t("devices:fleet.platform.unknown", "Unknown platform")
	);
}

/** What shared access to a device covers, as far as this computer knows. */
export interface FleetShare {
	capabilities?: readonly Capability[];
	/** Set when every grant is limited to one app. */
	appId?: string;
}

/** Shared access as a row needs it: every permission granted, and the one app when all grants name the same one. */
export function shareOfGrants(
	grants:
		| readonly Pick<ManagementGrant, "scope" | "capabilities">[]
		| undefined,
): FleetShare | undefined {
	if (!grants?.length) return undefined;
	const apps = new Set(
		grants.map((grant) =>
			grant.scope.kind === "device" ? "" : grant.scope.project_id,
		),
	);
	const [only] = apps;
	return {
		capabilities: [...new Set(grants.flatMap((grant) => grant.capabilities))],
		...(apps.size === 1 && only ? { appId: only } : {}),
	};
}

/** A row action that comes from an attention item ("Delete keys…", "Revoke spending limit…"). */
export interface FleetRowAction {
	id: string;
	label: string;
	danger?: boolean;
	onSelect(): void;
}

/** One device as the Devices table shows it; derived once per fleet read by the table. */
export interface FleetDeviceEntry {
	view: DeviceViewModel;
	name: string;
	/** `isolation.platform` of the last status read. */
	os?: string;
	/** Where an empty service list was read from. */
	statusSource?: Freshness;
	share?: FleetShare;
	/** Spending the viewer still pays for on this device, in micros. */
	billing?: { usedMicros: number; limitMicros: number };
	actions: readonly FleetRowAction[];
	/** Lower-case text the search matches: name, ID, apps and services. */
	haystack: string;
}

export interface FleetDeviceRowProps {
	entry: FleetDeviceEntry;
	/** The fleet inventory row; `undefined` = not listed. */
	certificates?: PublicCertificateInventory;
	/** Hub without the fleet inventory route: each row asks for its own. */
	certificatesPerRow: boolean;
	appName(appId: string): string | undefined;
	onLock(deviceId: string): void;
}

/* Cells. */

/** The worst open item in a few words, and how many other reasons there are. */
function healthReasons(
	t: DevicesT,
	view: DeviceViewModel,
): { top: string; more: number } {
	if (view.health === "unknown")
		return {
			top:
				view.keys.state === "none"
					? t("devices:fleet.health.noKeys", "No keys on this computer")
					: t("devices:fleet.health.locked", "Locked on this computer"),
			more: 0,
		};
	const [top = "", ...rest] = [
		...new Set(
			view.attention
				.filter((item) => item.severity !== "info")
				.map((item) => attentionShort(t, item.key)),
		),
	];
	return { top, more: rest.length };
}

function HealthCell({ view }: Readonly<{ view: DeviceViewModel }>) {
	const { t } = useTranslation("devices");
	const { top, more } = healthReasons(t, view);
	return (
		<>
			<HealthChip level={view.health} className={CELL_CHIP} />
			{top ? (
				<CellSub>
					{more > 0 ? (
						<Trans
							t={t}
							i18nKey="fleet.health.andMore"
							defaults="{{reason}} · <1>+{{count, number}} more</1>"
							values={{ reason: top, count: more }}
							components={{ 1: <span className="whitespace-nowrap" /> }}
						/>
					) : (
						top
					)}
				</CellSub>
			) : null}
		</>
	);
}

/** When a device was revoked (hubs that keep the date) and when it last checked in. */
function revokedLine(
	t: DevicesT,
	row: DeviceRow,
	day: (atS: number) => string,
): string {
	const { revoked_at: revoked, last_seen_at: last } = row;
	if (revoked == null)
		return last === null
			? t("devices:fleet.checkin.revokedNever", "never checked in")
			: t("devices:fleet.checkin.revokedLast", "last check-in {{last}}", {
					last: day(last),
				});
	return last === null
		? t(
				"devices:fleet.checkin.revokedOnNever",
				"on {{date}} · never checked in",
				{ date: day(revoked) },
			)
		: t(
				"devices:fleet.checkin.revokedOnLast",
				"on {{date}} · last check-in {{last}}",
				{ date: day(revoked), last: day(last) },
			);
}

function CheckinCell({ view }: Readonly<{ view: DeviceViewModel }>) {
	const { t, i18n } = useTranslation("devices");
	const time = useAreaTime();
	const { row, presence } = view;
	const locale = i18n?.language ?? "en";
	const chip = (
		<PresenceChip
			kind={presence.kind}
			since={presence.kind === "revoked" ? undefined : presence.since}
			short
			className={PLAIN_CHIP}
		/>
	);
	if (presence.kind === "never")
		return (
			<>
				{chip}
				<CellSub title={time.abs(row.registered_at)}>
					{t("fleet.checkin.registeredAgo", "registered {{ago}}", {
						ago: time.ago(row.registered_at),
					})}
				</CellSub>
			</>
		);
	if (presence.kind === "revoked")
		return (
			<>
				{chip}
				<CellSub>
					{revokedLine(t, row, (atS) => dayLabel(atS, time.now, locale))}
				</CellSub>
			</>
		);
	return (
		<>
			{chip}
			<CellSub>
				<CheckinLane ticks={laneTicksOf(row, time.nowS)} />
			</CellSub>
		</>
	);
}

type Unavailable = { state: AgeState; reason?: CopyRef<FreshnessReason> };

function unavailableCopy(
	t: DevicesT,
	view: DeviceViewModel,
	unavailable: Unavailable,
): { title: string; sub: string; locked?: boolean } {
	if (view.row.status === "revoked")
		return {
			title: t("devices:fleet.services.notAvailable", "Not available"),
			sub: t(
				"devices:fleet.services.revokedNotRead",
				"revoked devices aren't read",
			),
		};
	const reason = unavailable.reason?.code;
	if (unavailable.state === "locked")
		return {
			title: t("devices:fleet.services.locked", "Locked"),
			sub: t("devices:fleet.services.unlockToRead", "unlock to read"),
			locked: true,
		};
	if (reason === "never_reported")
		return {
			title: t("devices:fleet.services.notLoaded", "Not loaded"),
			sub: t("devices:fleet.services.noStatusYet", "no status yet"),
		};
	if (reason === "no_keys_here")
		return {
			title: t("devices:fleet.services.notReadable", "Not readable"),
			sub: t("devices:fleet.services.noKeysHere", "no keys on this computer"),
		};
	if (unavailable.state === "noaccess")
		return {
			title: t("devices:fleet.services.noAccess", "No access"),
			sub:
				reason === "access_ended"
					? t("devices:fleet.services.accessEnded", "your access has ended")
					: t(
							"devices:fleet.services.needsStatus",
							"your access doesn't include View status",
						),
		};
	if (unavailable.state === "unsupported")
		return {
			title: t("devices:fleet.services.unsupported", "Not supported"),
			sub: t(
				"devices:fleet.services.updateAgent",
				"update the device agent to read status here",
			),
		};
	if (unavailable.state === "error")
		return {
			title: t("devices:fleet.services.error", "Couldn't read"),
			sub:
				reason === "integrity"
					? t(
							"devices:fleet.services.integrity",
							"the status failed its integrity check",
						)
					: t("devices:fleet.services.retrying", "trying again shortly"),
		};
	return {
		title: t("devices:fleet.services.notLoaded", "Not loaded"),
		sub: t("devices:fleet.services.waiting", "waiting for the first status"),
	};
}

const CRASHING = new Set<ServiceView["conv"]>([
	"crash_looping",
	"failed_stopped",
]);

/** "3 · 2 as requested · 1 updating": crashing first, in the severity order of the Services view. */
function serviceSummary(t: DevicesT, services: readonly ServiceView[]) {
	const count = (match: (conv: ServiceView["conv"]) => boolean) =>
		services.filter((service) => match(service.conv)).length;
	const crashing = count((conv) => CRASHING.has(conv));
	const parts = [
		[
			count((conv) => conv === "converged"),
			(n: number) =>
				t(
					"devices:fleet.services.asRequested",
					"{{count, number}} as requested",
					{
						count: n,
					},
				),
		],
		[
			count((conv) => conv === "update_in_progress"),
			(n: number) =>
				t("devices:fleet.services.updating", "{{count, number}} updating", {
					count: n,
				}),
		],
		[
			count((conv) => conv === "converging"),
			(n: number) =>
				t("devices:fleet.services.applying", "{{count, number}} applying", {
					count: n,
				}),
		],
		[
			count((conv) => conv === "stopped_by_user"),
			(n: number) =>
				t("devices:fleet.services.stopped", "{{count, number}} stopped", {
					count: n,
				}),
		],
		[
			count((conv) => conv === "unknown"),
			(n: number) =>
				t("devices:fleet.services.unknown", "{{count, number}} unknown", {
					count: n,
				}),
		],
	] as const;
	return {
		crashing,
		rest: parts.filter(([n]) => n > 0).map(([n, text]) => text(n)),
	};
}

function ServicesCell({ entry }: Readonly<{ entry: FleetDeviceEntry }>) {
	const { t } = useTranslation("devices");
	const { view } = entry;
	const { services } = view;
	if (!Array.isArray(services)) {
		const copy = unavailableCopy(t, view, services);
		return (
			<>
				<span
					data-services={services.state}
					className={cx(
						"inline-flex items-center gap-1",
						copy.locked && "text-locked",
					)}
				>
					{copy.locked ? <Lock aria-hidden className="size-3.5" /> : null}
					{copy.title}
				</span>
				<CellSub>{copy.sub}</CellSub>
			</>
		);
	}
	const source = services[0]?.freshness ?? entry.statusSource;
	const { crashing, rest } = serviceSummary(t, services);
	const lead = services.length > 1 ? `${services.length} · ` : "";
	return (
		<>
			<span data-services="read">
				{services.length === 0 ? (
					t("fleet.services.none", "No services")
				) : (
					<>
						{lead}
						{crashing > 0 ? (
							<b className="font-semibold text-critical">
								{t("fleet.services.crashing", "{{count, number}} crashing", {
									count: crashing,
								})}
							</b>
						) : null}
						{crashing > 0 && rest.length ? " · " : ""}
						{rest.join(" · ")}
					</>
				)}
			</span>
			{source ? (
				<CellSub>
					<FreshnessStamp {...stampOf(source)} compact />
				</CellSub>
			) : null}
		</>
	);
}

function CertificateSummary({
	inventory,
	never = false,
}: Readonly<{
	inventory: PublicCertificateInventory;
	/** The device has never checked in, so it can't have reported anything. */
	never?: boolean;
}>) {
	const { t, i18n } = useTranslation("devices");
	const time = useAreaTime();
	const locale = i18n?.language ?? "en";
	const { certificates, updated_at: updatedAt } = inventory;
	if (updatedAt === null)
		return (
			<>
				{t("fleet.certificates.notReported", "Not reported")}
				<CellSub>
					{never
						? t("fleet.certificates.neverCheckedIn", "never checked in")
						: t(
								"fleet.certificates.notReportedSub",
								"the device hasn't reported certificates",
							)}
				</CellSub>
			</>
		);
	const confirmed = (
		<CellSub title={time.abs(updatedAt)}>
			{t("fleet.certificates.confirmed", "confirmed {{ago}}", {
				ago: time.ago(updatedAt),
			})}
		</CellSub>
	);
	if (certificates.length === 0)
		return (
			<>
				{t("fleet.certificates.none", "None")}
				{confirmed}
			</>
		);
	const sorted = [...certificates].sort((a, b) => a.not_after - b.not_after);
	const next = sorted[0];
	const state = certificateStatus(next, time.nowS);
	const count = certificates.length;
	const date = dayLabel(next.not_after, time.now, locale);
	const text =
		state === "expired"
			? t(
					"fleet.certificates.expired",
					"{{count, number}} · expired {{date}}",
					{
						count,
						date,
					},
				)
			: state === "expiring"
				? t(
						"fleet.certificates.expiring",
						"{{count, number}} · next expires {{when}}",
						{ count, when: time.ago(next.not_after) },
					)
				: t("fleet.certificates.next", "{{count, number}} · next {{date}}", {
						count,
						date,
					});
	return (
		<>
			<span data-certificates={state} title={time.abs(next.not_after)}>
				{state === "expired" || state === "expiring" ? (
					<>
						<SeverityWord severity="warning" className="align-middle" />{" "}
					</>
				) : null}
				{text}
			</span>
			{confirmed}
		</>
	);
}

function CertificatesNoAccess() {
	const { t } = useTranslation("devices");
	return (
		<>
			{t("fleet.certificates.noAccess", "No access")}
			<CellSub>
				{t("fleet.certificates.noAccessSub", "needs whole-device View status")}
			</CellSub>
		</>
	);
}

/** Older hub (no fleet inventory): the row reads its own certificate inventory. */
function CertificatesPerRow({ deviceId }: Readonly<{ deviceId: string }>) {
	const { t } = useTranslation("devices");
	const read = useCertificateInventory(deviceId);
	if (read.data) return <CertificateSummary inventory={read.data} />;
	if (read.error?.code === "forbidden") return <CertificatesNoAccess />;
	if (read.loading)
		return (
			<span className="text-muted-foreground">
				{t("fleet.certificates.reading", "Reading…")}
			</span>
		);
	return (
		<>
			{t("fleet.certificates.notLoaded", "Not loaded")}
			<CellSub>
				{t(
					"fleet.certificates.notLoadedSub",
					"couldn't read the certificate list",
				)}
			</CellSub>
		</>
	);
}

function CertificatesCell({
	view,
	certificates,
	perRow,
}: Readonly<{
	view: DeviceViewModel;
	certificates?: PublicCertificateInventory;
	perRow: boolean;
}>) {
	if (view.row.status === "revoked") return "–";
	const never = view.presence.kind === "never";
	if (certificates)
		return <CertificateSummary inventory={certificates} never={never} />;
	if (perRow) return <CertificatesPerRow deviceId={view.row.device_id} />;
	if (view.relationship === "owner")
		return <CertificateSummary inventory={NOT_REPORTED} never={never} />;
	return <CertificatesNoAccess />;
}

const NOT_REPORTED: PublicCertificateInventory = {
	revision: 0,
	updated_at: null,
	certificates: [],
};

function OwnedByOther({ view }: Readonly<{ view: DeviceViewModel }>) {
	const owner = useUserIdentity(view.row.owner_id);
	const ends = view.row.access_expires_at;
	return (
		<RelationshipChip
			relationship={view.relationship}
			ownerName={owner.isResolved ? owner.label : undefined}
			endsAt={view.relationship === "shared" && ends != null ? ends : undefined}
			className={PLAIN_TEXT}
		/>
	);
}

/**
 * The key chip of a row. The key session calls keys "unusable" only once an
 * unlock was prepared; the hub row already says so for a revoked device and
 * for shared access that has ended.
 */
export function rowKeyChip(view: DeviceViewModel, nowS: number) {
	const ends = view.row.access_expires_at;
	const ended = view.relationship === "shared" && ends != null && ends <= nowS;
	return view.keys.state !== "none" && (view.row.status === "revoked" || ended)
		? { state: "stale" as const }
		: keyChipOf(view.keys, view.live);
}

function AccessCell({ entry }: Readonly<{ entry: FleetDeviceEntry }>) {
	const { t, i18n } = useTranslation("devices");
	const time = useAreaTime();
	const { view, billing } = entry;
	const chip = rowKeyChip(view, time.nowS);
	const locked = view.keys.state === "locked" && chip.state !== "stale";
	const money = (micros: number) => formatMoney(micros, i18n?.language ?? "en");
	const showKeys = !(
		view.relationship === "cloud_approval" && chip.state === "none"
	);
	return (
		<>
			{view.relationship === "owner" ? (
				<RelationshipChip relationship="owner" className={PLAIN_TEXT} />
			) : (
				<OwnedByOther view={view} />
			)}
			{billing ? (
				<CellSub data-billing="">
					{t("fleet.access.spent", "{{used}} of {{limit}}", {
						used: money(billing.usedMicros),
						limit: money(billing.limitMicros),
					})}
				</CellSub>
			) : null}
			{showKeys ? (
				<CellSub>
					<KeyChip {...chip} className={CELL_CHIP} />
				</CellSub>
			) : null}
			{locked || entry.actions.length ? (
				<span className="mt-1.5 flex flex-wrap gap-1.5">
					{locked ? (
						<DvButton
							size="sm"
							icon={LockOpen}
							onClick={() =>
								useOverlayStore.getState().openUnlock(view.row.device_id)
							}
						>
							{t("fleet.access.unlock", "Unlock…")}
						</DvButton>
					) : null}
					{entry.actions.map(renderRowAction)}
				</span>
			) : null}
		</>
	);
}

function renderRowAction(action: FleetRowAction) {
	return (
		<DvButton
			key={action.id}
			size="sm"
			variant={action.danger ? "danger-ghost" : "default"}
			onClick={action.onSelect}
		>
			{action.label}
		</DvButton>
	);
}

function AgentCell({ entry }: Readonly<{ entry: FleetDeviceEntry }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { view, os } = entry;
	if (view.row.status === "revoked") return "–";
	const platform = os ? platformLabel(t, os) : undefined;
	const { agent } = view;
	if (!agent) {
		const locked =
			!Array.isArray(view.services) && view.services.state === "locked";
		const sub =
			platform ??
			(locked
				? t("fleet.agent.untilUnlocked", "Unknown until unlocked")
				: undefined);
		return (
			<>
				<span className="font-sans">
					{locked
						? t("fleet.agent.locked", "Locked")
						: t("fleet.agent.notReported", "Not reported")}
				</span>
				{sub ? <CellSub className="font-sans">{sub}</CellSub> : null}
			</>
		);
	}
	const current = agent.source.age === "live" || agent.source.age === "current";
	const read =
		current || agent.source.at === undefined
			? undefined
			: t("fleet.agent.read", "read {{ago}}", {
					ago: time.ago(agent.source.at),
				});
	const sub = [platform, read].filter(Boolean).join(" · ");
	return (
		<>
			<span title={agent.version}>{agent.version}</span>
			{sub ? <CellSub className="font-sans">{sub}</CellSub> : null}
		</>
	);
}

/* Row menu. */

interface MenuEntry {
	id: string;
	label: string;
	icon: LucideIcon;
	route?: DevicesRoute;
	onSelect?: () => void;
	/** Why the item can't be used now (R7: visible, disabled, with its reason). */
	blocked?: string;
	note?: string;
	danger?: boolean;
	separated?: boolean;
}

interface MenuContext {
	t: DevicesT;
	entry: FleetDeviceEntry;
	/** The row's key chip state (unusable keys included). */
	keyState: ReturnType<typeof rowKeyChip>["state"];
	appName(appId: string): string | undefined;
	/** "11:00" / "29 Sept, 11:00" for a unix-seconds instant. */
	at(atS: number): string;
	onLock(deviceId: string): void;
	onCopy(text: string): void;
}

function deployBlocked({
	t,
	entry,
	keyState,
	at: offlineSince,
}: MenuContext): string | undefined {
	const { view, share } = entry;
	const caps = share?.capabilities;
	if (view.relationship !== "owner" && caps && !caps.includes("deploy"))
		return t(
			"devices:fleet.menu.deployNoAccess",
			"Your access doesn't include Deploy & configure.",
		);
	if (view.presence.kind === "never")
		return t(
			"devices:fleet.menu.deployNever",
			"It hasn't checked in yet. Deploying needs a live connection.",
		);
	if (view.presence.kind === "offline")
		return view.presence.since === undefined
			? t(
					"devices:fleet.menu.deployOffline",
					"It's offline. Deploying needs a live connection.",
				)
			: t(
					"devices:fleet.menu.deployOfflineSince",
					"Offline since {{time}}. Deploying needs a live connection.",
					{ time: offlineSince(view.presence.since) },
				);
	if (keyState === "none" || keyState === "stale")
		return t(
			"devices:fleet.menu.deployNoKeys",
			"This computer has no usable keys for it.",
		);
	return undefined;
}

/** "Deploy an app…", or "Deploy {App}…" on a device shared for one app only (APP §6.1). */
function deployEntry(ctx: MenuContext): MenuEntry | undefined {
	const { t, entry } = ctx;
	const { view } = entry;
	if (view.row.status !== "active" || view.relationship === "cloud_approval")
		return undefined;
	const appId = entry.share?.appId;
	const app = appId ? ctx.appName(appId) : undefined;
	const blocked = deployBlocked(ctx);
	return {
		id: "deploy",
		label: app
			? t("devices:fleet.menu.deployApp", "Deploy {{app}}…", { app })
			: t("devices:fleet.menu.deploy", "Deploy an app…"),
		icon: Rocket,
		route: {
			screen: "deploy",
			deviceIds: [view.row.device_id],
			...(appId ? { appId } : {}),
		},
		...(blocked ? { blocked } : {}),
		...(app && !blocked
			? {
					note: t(
						"devices:fleet.menu.deployAppNote",
						"Shared with you for {{app}} only",
						{ app },
					),
				}
			: {}),
	};
}

/** Lock while the keys are open, Unlock… while usable keys are closed. */
function keyEntry({
	t,
	entry,
	keyState,
	onLock,
}: MenuContext): MenuEntry | undefined {
	const deviceId = entry.view.row.device_id;
	if (isKeySessionOpen(keyState))
		return {
			id: "lock",
			label: t("devices:fleet.menu.lock", "Lock"),
			icon: Lock,
			onSelect: () => onLock(deviceId),
		};
	if (keyState !== "locked") return undefined;
	return {
		id: "unlock",
		label: t("devices:fleet.menu.unlock", "Unlock…"),
		icon: LockOpen,
		onSelect: () => useOverlayStore.getState().openUnlock(deviceId),
	};
}

function menuEntries(ctx: MenuContext): MenuEntry[] {
	const { t, entry, keyState } = ctx;
	const { view, name } = entry;
	const deviceId = view.row.device_id;
	const active = view.row.status === "active";
	const entries: (MenuEntry | undefined | false)[] = [
		{
			id: "open",
			label: t("devices:fleet.menu.open", "Open"),
			icon: ArrowRight,
			route: { screen: "device", deviceId, tab: "overview" },
		},
		{
			id: "copy-id",
			label: t("devices:fleet.menu.copyId", "Copy device ID"),
			icon: Copy,
			onSelect: () => ctx.onCopy(deviceId),
		},
		deployEntry(ctx),
		keyEntry(ctx),
		active && {
			id: "diagnose",
			label: t("devices:fleet.menu.diagnose", "Diagnose connection…"),
			icon: Stethoscope,
			onSelect: () => useOverlayStore.getState().openDiagnose(deviceId),
		},
		{
			id: "cloud",
			label: t("devices:fleet.menu.cloud", "Cloud approvals"),
			icon: Cloud,
			route: { screen: "device", deviceId, tab: "access" },
		},
		keyState !== "none" && {
			id: "keys",
			label: t("devices:fleet.menu.keys", "Keys & recovery"),
			icon: KeyRound,
			route: { screen: "keys", focusDeviceId: deviceId },
		},
		active &&
			view.relationship === "owner" && {
				id: "revoke",
				label: t("devices:fleet.menu.revoke", "Revoke {{device}}…", {
					device: name,
				}),
				icon: Power,
				route: {
					screen: "device",
					deviceId,
					tab: "settings",
					action: "revoke",
				},
				danger: true,
				separated: true,
			},
	];
	return entries.filter((item): item is MenuEntry => !!item);
}

function useMenuEntries(props: Readonly<FleetDeviceRowProps>): MenuEntry[] {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { copy } = useCopy();
	const { appName, onLock } = props;
	const { view } = props.entry;
	// The hub says what shared access covers; it is asked only while the menu is open.
	const access = useMyAccess(
		view.relationship === "shared" ? view.row.device_id : undefined,
	);
	const share = shareOfGrants(access.data?.grants) ?? props.entry.share;
	const entry = share ? { ...props.entry, share } : props.entry;
	return menuEntries({
		t,
		entry,
		keyState: rowKeyChip(entry.view, time.nowS).state,
		appName,
		at: time.at,
		onLock,
		onCopy: (text) => void copy(text),
	});
}

function MenuLabel({ entry }: Readonly<{ entry: MenuEntry }>) {
	const Icon = entry.icon;
	const sub = entry.blocked ?? entry.note;
	return (
		<>
			<Icon aria-hidden className="mt-0.5 size-3.5 shrink-0" />
			<span className="flex min-w-0 flex-col">
				<span>{entry.label}</span>
				{sub ? (
					<span className="max-w-[34ch] text-xs whitespace-normal text-muted-foreground">
						{sub}
					</span>
				) : null}
			</span>
		</>
	);
}

/** Mounted only while the menu is open, so a closed row costs nothing. */
function RowMenuItems(props: Readonly<FleetDeviceRowProps>) {
	const link = useRouteLink();
	const entries = useMenuEntries(props);
	return entries.map((entry) => {
		const className = cx(
			MENU_ITEM_CLASS,
			"items-start gap-2",
			entry.danger && "text-danger focus:text-danger",
		);
		const item =
			entry.route && !entry.blocked ? (
				<DropdownMenuItem asChild className={className}>
					<a data-menu-item={entry.id} {...link(entry.route)}>
						<MenuLabel entry={entry} />
					</a>
				</DropdownMenuItem>
			) : (
				<DropdownMenuItem
					data-menu-item={entry.id}
					disabled={!!entry.blocked}
					onSelect={entry.blocked ? undefined : entry.onSelect}
					className={cx(className, "data-disabled:opacity-100")}
				>
					<MenuLabel entry={entry} />
				</DropdownMenuItem>
			);
		return (
			<span key={entry.id} className="contents">
				{entry.separated ? <DropdownMenuSeparator /> : null}
				{item}
			</span>
		);
	});
}

function RowMenu(props: Readonly<FleetDeviceRowProps>) {
	const { t } = useTranslation("devices");
	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<DvButton
					size="xs"
					variant="ghost"
					iconOnly
					icon={Ellipsis}
					aria-label={t("fleet.menu.more", "More for {{device}}", {
						device: props.entry.name,
					})}
				/>
			</DropdownMenuTrigger>
			<DropdownMenuContent
				align="end"
				className={cx(MENU_CONTENT_CLASS, "min-w-56")}
			>
				<RowMenuItems {...props} />
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

/* Row. */

const INTERACTIVE = "a, button, input, select, [role=menu], [role=menuitem]";

export interface FleetColumnLabels {
	device: string;
	health: string;
	checkin: string;
	services: string;
	certificates: string;
	access: string;
	agent: string;
}

export function fleetColumnLabels(t: DevicesT): FleetColumnLabels {
	return {
		device: t("devices:fleet.column.device", "Device"),
		health: t("devices:fleet.column.health", "Health"),
		checkin: t("devices:fleet.column.checkin", "Check-in"),
		services: t("devices:fleet.column.services", "Services"),
		certificates: t("devices:fleet.column.certificates", "Certificates"),
		access: t("devices:fleet.column.access", "Access & keys"),
		agent: t("devices:fleet.column.agent", "Agent"),
	};
}

/** SPEC §4.13: one device in the N1 table; the whole row opens the device, controls inside it don't. */
export const FleetDeviceRow = memo(function FleetDeviceRow(
	props: Readonly<FleetDeviceRowProps>,
) {
	const { t, i18n } = useTranslation("devices");
	const time = useAreaTime();
	const link = useRouteLink();
	const { entry } = props;
	const { view, name } = entry;
	const { row } = view;
	const labels = fleetColumnLabels(t);
	const target = link({
		screen: "device",
		deviceId: row.device_id,
		tab: "overview",
	});
	const openRow = (event: MouseEvent<HTMLTableRowElement>) => {
		if ((event.target as HTMLElement).closest(INTERACTIVE)) return;
		if (globalThis.getSelection?.()?.toString()) return;
		target.onClick(event as unknown as MouseEvent<HTMLAnchorElement>);
	};
	return (
		<Tr
			data-device={row.device_id}
			dim={row.status === "revoked"}
			onClick={openRow}
			className="cursor-pointer"
		>
			<Td label={labels.device} kind="name">
				<a
					{...target}
					title={name}
					className="block truncate font-mono font-semibold text-foreground hover:underline"
				>
					{name}
				</a>
				<CellSub className={FACT_LINE}>
					<span className={FACT_ROW}>
						<span className={FACT_FIRST}>
							<IdRef
								id={row.device_id}
								copyLabel={t("fleet.device.copyId", "Copy device ID")}
							/>
						</span>
						<span title={time.abs(row.registered_at)} className={FACT_NEXT}>
							{t("fleet.device.registeredOn", "registered {{date}}", {
								date: dayLabel(
									row.registered_at,
									time.now,
									i18n?.language ?? "en",
								),
							})}
						</span>
					</span>
				</CellSub>
			</Td>
			<Td label={labels.health}>
				<HealthCell view={view} />
			</Td>
			<Td label={labels.checkin}>
				<CheckinCell view={view} />
			</Td>
			<Td label={labels.services}>
				<ServicesCell entry={entry} />
			</Td>
			<Td label={labels.certificates}>
				<CertificatesCell
					view={view}
					certificates={props.certificates}
					perRow={props.certificatesPerRow}
				/>
			</Td>
			<Td label={labels.access}>
				<AccessCell entry={entry} />
			</Td>
			<Td label={labels.agent} kind="mono" className="whitespace-normal">
				<AgentCell entry={entry} />
			</Td>
			<Td label="" kind="more">
				<RowMenu {...props} />
			</Td>
		</Tr>
	);
});

/** The attention items a row turns into buttons in its Access & keys cell. */
export const ROW_ACTION_KEYS: ReadonlySet<AttentionItem["key"]> = new Set([
	"stale_local_keys",
	"you_still_pay_for_a_revoked_device",
]);
