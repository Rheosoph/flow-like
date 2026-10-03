"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	Copy,
	Ellipsis,
	Fingerprint,
	KeyRound,
	Lock,
	LockOpen,
	Power,
	Radio,
	Rocket,
	Stethoscope,
	Timer,
	Trash2,
	Unplug,
} from "lucide-react";
import { type ReactNode, useMemo } from "react";
import {
	compareVersions,
	keysLocked,
} from "../../../../lib/device-management/model/device-view";
import { PRESENCE_ONLINE_S } from "../../../../lib/device-management/model/presence";
import type {
	AttentionKey,
	DeployRoute,
	DeviceRow,
	DevicesRoute,
	FixAction,
	GateResult,
} from "../../../../lib/device-management/model/types";
import type { InventoryScope } from "../../../../lib/device-management/types";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../../../ui/dropdown-menu";
import { enumLabel } from "../copy/enum-labels";
import { usableRelease, useAgentReleaseVerdict } from "../hub/hub-view";
import { useAreaTime } from "../primitives/area-context";
import { type Crumb, ObjectHeader } from "../primitives/block";
import {
	CheckinLane,
	LANE_SLOTS,
	LANE_SLOT_SEC,
	type LaneTick,
} from "../primitives/checkin-lane";
import { DvButton } from "../primitives/dv-button";
import { GateInline, GatedAction } from "../primitives/gate-notice";
import { IdRef } from "../primitives/id-ref";
import { PresenceGlyph } from "../primitives/presence-glyph";
import {
	type BackupChipState,
	HealthChip,
	KeyChip,
	PresenceChip,
	RelationshipChip,
	SafetyChip,
	StatusChip,
} from "../primitives/status-chip";
import { copyText } from "../primitives/use-copy";
import { ACCOUNT_SCOPE } from "../routing/devices-route";
import { useDevicesRoute, useRouteLink } from "../routing/use-devices-route";
import { useAppNames, useRunAttentionTarget } from "../shell/attention-popover";
import {
	type AppViewRead,
	useAttentionState,
	useGate,
	useKeyChip,
	useLiveSession,
	useMyAccess,
	useOverlay,
} from "../workspace";
import {
	type DevicePage,
	type GateView,
	gateView,
	platformLabel,
	scopedApp,
	usePersonName,
} from "./use-device-page";

const MENU_CONTENT =
	"min-w-56 border-border-strong bg-popover shadow-none backdrop-blur-none";
const MENU_ITEM =
	"gap-2 text-[13px]/[18px] focus:bg-row-hover focus:text-foreground";
const LINK =
	"underline decoration-border-strong underline-offset-2 hover:decoration-current";

/** A device, service or version name inside a sentence (R15). */
export function Mono({ children }: Readonly<{ children: ReactNode }>) {
	return <span className="font-mono text-[0.94em]">{children}</span>;
}

/* Fixes the page header or its banners already offer in the same state: not repeated next to every gated control. */
const OFFERED_BY_HEADER = new Set<FixAction["kind"]>([
	"unlock",
	"restore_keys",
	"import_key_file",
	"diagnose",
	"review_identity",
]);

/** The fix that belongs to a gate reason ("Connect live", "Update agent…"), unless the page header offers it. */
export function GateFix({ view }: Readonly<{ view: GateView | null }>) {
	const { navigate } = useDevicesRoute();
	const run = useRunAttentionTarget({ onNavigate: navigate });
	const fix = view?.fix;
	if (!fix || OFFERED_BY_HEADER.has(fix.action.kind)) return null;
	return (
		<DvButton size="xs" onClick={() => run(fix.action)}>
			{fix.label}
		</DvButton>
	);
}

/* Deploy: one target for the header, the Services tab and the empty states. */

export interface DeployTarget {
	label: string;
	title: string;
	route: DeployRoute;
	gate: GateView | null;
}

/**
 * Where "Deploy" on this page leads and why it can't run now. In an app the
 * target is that app; otherwise the wizard starts with the app picker.
 */
export function useDeployTarget(
	page: DevicePage,
	app: AppViewRead | null,
): DeployTarget {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const projectId = page.app?.id ?? scopedApp(page) ?? undefined;
	const result = useGate(
		"create_service",
		page.deviceId,
		projectId ? { projectId } : undefined,
	);
	return useMemo(() => {
		const blocked: GateView | null = page.identity
			? {
					gate: {
						kind: "policy",
						reason: t(
							"device.identity.blockedShort",
							"Management is blocked until the identity is confirmed.",
						),
					},
				}
			: gateView(t, time, result);
		if (!page.app)
			return {
				label: t("device.deploy.anApp", "Deploy an app…"),
				title: t(
					"device.deploy.anAppTitle",
					"Pick an app to deploy to {{device}}",
					{
						device: page.name,
					},
				),
				route: { screen: "deploy", deviceIds: [page.deviceId] },
				gate: blocked,
			};
		const view = app?.view;
		const noEvents =
			!!view && view.app.canReadFlows && view.events.rows.length === 0;
		const gate: GateView | null =
			blocked ??
			(noEvents
				? {
						gate: {
							kind: "unsupported",
							reason: t(
								"device.deploy.noEvents",
								"No event of this app can run on a device yet. Most types can: add one in Events. Inbound email, Teams and deep-link events can't.",
							),
						},
					}
				: null);
		return {
			label: t("device.deploy.appHere", "Deploy {{app}} here…", {
				app: page.app.name,
			}),
			title:
				view?.app.mode === "offline"
					? t(
							"device.deploy.appHereOffline",
							"{{app}} gets an offline copy on {{device}}",
							{ app: page.app.name, device: page.name },
						)
					: t(
							"device.deploy.appHereOnline",
							"{{app}} runs online on {{device}}",
							{
								app: page.app.name,
								device: page.name,
							},
						),
			route: { screen: "deploy", mode: "new", deviceIds: [page.deviceId] },
			gate,
		};
	}, [t, time, result, page, app?.view]);
}

/** Deploy as a link; gated: visible, disabled, with the reason and its fix (R7). */
export function DeployAction({
	target,
	primary = false,
	size = "md",
	fix = true,
	gateClassName,
}: Readonly<{
	target: DeployTarget;
	primary?: boolean;
	size?: "md" | "sm";
	/** `false` where the same fix is already offered next to it (a banner, the block's own state). */
	fix?: boolean;
	/** Layout of the disabled button with its reason (the header keeps the button next to its neighbours). */
	gateClassName?: string;
}>) {
	const link = useRouteLink();
	if (target.gate)
		return (
			<span className="inline-flex max-w-full flex-wrap items-start gap-2">
				<GatedAction gate={target.gate.gate} className={gateClassName}>
					<DvButton icon={Rocket} size={size}>
						{target.label}
					</DvButton>
				</GatedAction>
				{fix ? <GateFix view={target.gate} /> : null}
			</span>
		);
	return (
		<DvButton
			asChild
			variant={primary ? "primary" : "default"}
			icon={Rocket}
			size={size}
		>
			<a {...link(target.route)} title={target.title}>
				{target.label}
			</a>
		</DvButton>
	);
}

/* Check-in lane: the hub keeps the last check-in only, so earlier quarter-hours are known just for what this session saw. */

const seenSlots = new Map<string, Set<number>>();

function rememberCheckIn(deviceId: string, lastSeenAt: number | null) {
	if (lastSeenAt === null) return seenSlots.get(deviceId);
	const slots = seenSlots.get(deviceId) ?? new Set<number>();
	slots.add(Math.floor(lastSeenAt / LANE_SLOT_SEC));
	seenSlots.set(deviceId, slots);
	return slots;
}

export function laneTicks(
	row: Pick<DeviceRow, "device_id" | "registered_at" | "last_seen_at">,
	nowS: number,
): { ticks: LaneTick[]; endAt: number } {
	const seen = rememberCheckIn(row.device_id, row.last_seen_at);
	const current = Math.floor(nowS / LANE_SLOT_SEC);
	const online =
		row.last_seen_at !== null && nowS - row.last_seen_at <= PRESENCE_ONLINE_S;
	const ticks: LaneTick[] = [];
	for (let slot = current - LANE_SLOTS + 1; slot <= current; slot++) {
		const start = slot * LANE_SLOT_SEC;
		if (start + LANE_SLOT_SEC <= row.registered_at) ticks.push("pre");
		else if (seen?.has(slot)) ticks.push("ok");
		else if (slot === current && online) ticks.push("pre");
		else if (row.last_seen_at === null || start > row.last_seen_at)
			ticks.push("miss");
		else ticks.push("pre");
	}
	return { ticks, endAt: (current + 1) * LANE_SLOT_SEC };
}

const LANE_MIN_SEEN = 4;

/** A lane with one known quarter-hour says less than the presence chip next to it. */
export function laneTellsSomething(ticks: readonly LaneTick[]): boolean {
	return (
		ticks.includes("miss") ||
		ticks.filter((tick) => tick === "ok").length >= LANE_MIN_SEEN
	);
}

/* Header facts. */

function PlatformFact({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const platform = page.inspection?.isolation?.platform;
	if (!platform)
		return (
			<span className="text-muted-foreground">
				{keysLocked(page.view.keys)
					? t("device.header.unknownUntilUnlocked", "Unknown until unlocked")
					: t("device.header.notReported", "Not reported")}
			</span>
		);
	return (
		<span>
			{page.inspection?.hostIsolation === "required"
				? t(
						"device.header.platformSandbox",
						"{{platform}} · sandbox required",
						{
							platform: platformLabel(t, platform),
						},
					)
				: platformLabel(t, platform)}
		</span>
	);
}

function AgentFact({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const release = usableRelease(useAgentReleaseVerdict());
	const agent = page.view.agent;
	if (!agent)
		return (
			<span className="text-muted-foreground">
				{keysLocked(page.view.keys)
					? t("device.header.unknownUntilUnlocked", "Unknown until unlocked")
					: t("device.header.notReported", "Not reported")}
			</span>
		);
	const latest = release?.manifest.release_version;
	const behind = latest ? compareVersions(agent.version, latest) < 0 : false;
	const read = agent.source.age !== "live" && agent.source.at !== undefined;
	return (
		<span className="inline-flex flex-wrap items-baseline gap-x-1">
			<span className="font-mono">{agent.version}</span>
			{latest && !behind ? (
				<span>{t("device.header.agentNewest", "· newest verified")}</span>
			) : null}
			{latest && behind ? (
				<span className="text-warning">
					{t("device.header.agentAvailable", "· {{version}} available", {
						version: latest,
					})}
				</span>
			) : null}
			{read ? (
				<span className="text-muted-foreground">
					{t("device.header.agentRead", "(read {{ago}})", {
						ago: time.ago(agent.source.at ?? time.nowS),
					})}
				</span>
			) : null}
		</span>
	);
}

/* Safety chip: the attention engine decides the backup state; the chip repeats it. */

const BACKUP_BY_KEY: Partial<Record<AttentionKey, BackupChipState>> = {
	keys_not_backed_up_to_account: "never",
	account_backup_upload_pending: "upload_pending",
	account_backup_hub_newer: "hub_newer",
	account_backup_old_password: "local_changes",
};

function DeviceSafetyChip({ page }: Readonly<{ page: DevicePage }>) {
	const { input } = useAttentionState();
	const { keys } = page.view;
	if (keys.state === "none" || keys.state === "stale" || page.revoked)
		return null;
	const atRisk = page.attention.some(
		(item) => item.key === "storage_not_persistent",
	);
	const open = page.attention
		.map((item) => BACKUP_BY_KEY[item.key])
		.find((state) => state !== undefined);
	const revision = input.accountBackups[page.deviceId]?.revision ?? 0;
	const backup = open ?? (revision > 0 ? "in_sync" : undefined);
	if (!atRisk && !backup) return null;
	return (
		<SafetyChip
			atRisk={atRisk}
			backup={backup}
			revision={backup === "in_sync" ? revision : undefined}
		/>
	);
}

/** The key manager blocks a session only for a changed identity; the shared chip's wording for that state is about the browser. */
function DeviceKeyChip({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	if (!page.identity) return <KeyChip {...page.keyChip} />;
	return (
		<StatusChip
			tone="critical"
			icon={Fingerprint}
			data-key-state="blocked"
			title={t(
				"device.header.identityChangedTitle",
				"The hub reports other keys than the ones trusted on this computer. The keys here stay closed until you confirm the identity.",
			)}
		>
			{t("device.header.identityChanged", "Identity changed")}
		</StatusChip>
	);
}

function IdleLockChip({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const { keys } = page.view;
	if (keys.state !== "unlocked") return null;
	if (keys.keepUnlocked || keys.idleLocksAt === undefined)
		return (
			<StatusChip tone="outline" icon={Timer}>
				{t("device.header.staysUnlocked", "Stays unlocked until you lock it")}
			</StatusChip>
		);
	const minutes = Math.max(
		1,
		Math.round(
			(keys.idleLocksAt - (keys.lastUsedAt ?? keys.idleLocksAt)) / 60_000,
		),
	);
	return (
		<StatusChip tone="outline" icon={Timer}>
			{t("device.header.idleLock", "Locks after {{count, number}} min unused", {
				count: minutes,
			})}
		</StatusChip>
	);
}

/* Recipient access line (BG1/BG22). */

function ScopeText({
	page,
	scope,
}: Readonly<{ page: DevicePage; scope: InventoryScope }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const appName = useAppNames();
	if (scope.kind === "device")
		return <>{t("device.access.scopeDevice", "Whole device")}</>;
	if (scope.kind === "placement")
		return (
			<Trans
				t={t}
				i18nKey="device.access.scopeService"
				defaults="Service <1/>"
				components={{ 1: <Mono>{scope.placement_id}</Mono> }}
			/>
		);
	const name = appName(scope.project_id) ?? scope.project_id;
	return (
		<Trans
			t={t}
			i18nKey="device.access.scopeApp"
			defaults="App <1/>"
			components={{
				1: (
					<a
						{...link(
							{
								screen: "app-devices",
								by: "device",
								focusDeviceId: page.deviceId,
							},
							{ scope: { kind: "app", appId: scope.project_id } },
						)}
						className={LINK}
					>
						{name}
					</a>
				),
			}}
		/>
	);
}

function YourAccess({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	// An older hub doesn't say whose device it is: anyone but the owner is then a possible recipient.
	const recipient =
		!page.revoked &&
		(page.view.relationship === "shared" ||
			page.view.relationship === "unknown");
	const access = useMyAccess(recipient ? page.deviceId : undefined);
	if (!recipient) return null;
	const grants =
		access.data?.grants.map((grant) => ({
			id: grant.grant_id,
			scope: grant.scope,
			caps: grant.capabilities,
			expiresAt: grant.expires_at as number | undefined,
		})) ??
		page.capabilities?.map((grant, index) => ({
			id: String(index),
			scope: grant.scope,
			caps: [...grant.caps],
			expiresAt: grant.expiresAt,
		}));
	const list = new Intl.ListFormat(time.locale, { type: "conjunction" });
	return (
		<span
			data-device-access=""
			className="flex min-w-0 basis-full flex-wrap items-baseline gap-x-1.5 gap-y-1"
		>
			<span className="text-muted-foreground">
				{t("device.access.yours", "Your access")}
			</span>
			{grants?.length ? (
				grants.map((grant) => (
					<span key={grant.id} className="min-w-0">
						{list.format(
							grant.caps.map((cap) => enumLabel(t, "capability", cap)),
						)}
						{" · "}
						<ScopeText page={page} scope={grant.scope} />
						{grant.expiresAt === undefined ? null : (
							<>
								{" · "}
								<span title={time.abs(grant.expiresAt)}>
									{t("device.access.until", "until {{time}}", {
										time: time.at(grant.expiresAt),
									})}
								</span>
							</>
						)}
					</span>
				))
			) : access.data ? (
				<span>
					{t(
						"device.access.none",
						"None right now. Ask the owner to renew your access.",
					)}
				</span>
			) : (
				<span>
					{t(
						"device.access.unlockToCheck",
						"Unlock to check your permissions.",
					)}
				</span>
			)}
			{access.data && !access.data.applied ? (
				<span className="text-muted-foreground">
					{t(
						"device.access.waiting",
						"· waiting for the device to apply the newest access rules",
					)}
				</span>
			) : null}
		</span>
	);
}

/* Header actions. */

function OverflowMenu({
	page,
	onRevoke,
}: Readonly<{ page: DevicePage; onRevoke(): void }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const overlay = useOverlay();
	const chip = useKeyChip();
	const live = useLiveSession(page.deviceId);
	const connectGate = gateView(t, time, useGate("connect_live", page.deviceId));
	const open = page.view.keys.state === "unlocked";
	const connected =
		live.state.kind === "live" || live.state.kind === "renewing";
	const connecting =
		live.state.kind === "connecting" || live.state.kind === "reconnecting";
	return (
		<DropdownMenu modal={false}>
			<DropdownMenuTrigger asChild>
				<DvButton
					iconOnly
					icon={Ellipsis}
					aria-label={t("device.header.more", "More actions for {{device}}", {
						device: page.name,
					})}
				/>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end" className={MENU_CONTENT}>
				<DropdownMenuItem
					className={MENU_ITEM}
					onSelect={() => void copyText(page.deviceId)}
				>
					<Copy aria-hidden className="size-3.5" />
					{t("device.header.copyId", "Copy device ID")}
				</DropdownMenuItem>
				{page.revoked || page.consentOnly ? null : connected || connecting ? (
					<DropdownMenuItem className={MENU_ITEM} onSelect={() => live.close()}>
						<Unplug aria-hidden className="size-3.5" />
						{t("device.header.disconnect", "Disconnect")}
					</DropdownMenuItem>
				) : (
					<DropdownMenuItem
						className={MENU_ITEM}
						disabled={!!connectGate || !!page.identity}
						onSelect={() => void live.connect()}
					>
						<Radio aria-hidden className="size-3.5" />
						<span className="flex min-w-0 flex-col">
							{t("device.header.connect", "Connect live")}
							{connectGate ? (
								<span className="text-xs text-muted-foreground">
									{connectGate.gate.reason}
								</span>
							) : null}
						</span>
					</DropdownMenuItem>
				)}
				{page.revoked || page.consentOnly ? null : (
					<DropdownMenuItem
						className={MENU_ITEM}
						onSelect={() => overlay.openDiagnose(page.deviceId)}
					>
						<Stethoscope aria-hidden className="size-3.5" />
						{t("device.header.diagnose", "Diagnose connection…")}
					</DropdownMenuItem>
				)}
				{open ? (
					<DropdownMenuItem
						className={MENU_ITEM}
						onSelect={() => chip.lock(page.deviceId)}
					>
						<Lock aria-hidden className="size-3.5" />
						{t("device.header.lock", "Lock")}
					</DropdownMenuItem>
				) : null}
				{page.owner && !page.revoked ? (
					<>
						<DropdownMenuSeparator />
						<DropdownMenuItem
							className={`${MENU_ITEM} text-critical focus:text-critical`}
							onSelect={onRevoke}
						>
							<Power aria-hidden className="size-3.5" />
							{t("device.header.revoke", "Revoke device…")}
						</DropdownMenuItem>
					</>
				) : null}
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

/** Beside the identity the disabled button stays next to Lock; its reason hangs under it, flush right. */
const GATE_IN_HEADER = "@min-[900px]/devices:items-end";

function PrimaryActions({
	page,
	deploy,
	onOpenKeys,
}: Readonly<{ page: DevicePage; deploy: DeployTarget; onOpenKeys(): void }>) {
	const { t } = useTranslation("devices");
	const overlay = useOverlay();
	const chip = useKeyChip();
	const link = useRouteLink();
	const { keys } = page.view;
	if (page.consentOnly) return null;
	if (page.revoked)
		return keys.state === "none" ? null : (
			<DvButton variant="danger-ghost" icon={Trash2} onClick={onOpenKeys}>
				{t("device.header.deleteKeys", "Delete keys from this computer…")}
			</DvButton>
		);
	if (page.identity)
		return <DeployAction target={deploy} gateClassName={GATE_IN_HEADER} />;
	if (keys.state === "none" || keys.state === "stale")
		return (
			<DvButton asChild icon={KeyRound}>
				<a
					{...link(
						{ screen: "keys", focusDeviceId: page.deviceId },
						{ scope: ACCOUNT_SCOPE },
					)}
				>
					{t("device.header.restoreKeys", "Restore keys…")}
				</a>
			</DvButton>
		);
	if (keysLocked(keys))
		return (
			<DvButton
				variant="primary"
				icon={LockOpen}
				busy={keys.state === "unlocking"}
				onClick={() => overlay.openUnlock(page.deviceId, { connectLive: true })}
			>
				{t("device.header.unlock", "Unlock…")}
			</DvButton>
		);
	const hideDeploy =
		!page.app &&
		!!deploy.gate &&
		deploy.gate.gate.kind === "noaccess" &&
		!deploy.gate.fix;
	return (
		<>
			{hideDeploy ? null : (
				<DeployAction target={deploy} primary gateClassName={GATE_IN_HEADER} />
			)}
			<DvButton icon={Lock} onClick={() => chip.lock(page.deviceId)}>
				{t("device.header.lock", "Lock")}
			</DvButton>
		</>
	);
}

export interface DeviceHeaderProps {
	page: DevicePage;
	app: AppViewRead | null;
	onOpenKeys(): void;
	onRevoke(): void;
}

/** SPEC §5.2 header: who the device is, how it is doing, and the one action that fits its state. */
export function DeviceHeader({
	page,
	app,
	onOpenKeys,
	onRevoke,
}: Readonly<DeviceHeaderProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { href, navigate } = useDevicesRoute();
	const deploy = useDeployTarget(page, app);
	const { view } = page;
	const ownerName = usePersonName(view.row.owner_id, !page.owner);
	const lane = useMemo(
		() => (page.revoked ? null : laneTicks(view.row, time.nowS)),
		[page.revoked, view.row, time.nowS],
	);
	const counted = page.attention.filter((item) => item.severity !== "info");
	const fleet: DevicesRoute = page.app
		? { screen: "app-devices", by: "device", focusDeviceId: page.deviceId }
		: { screen: "fleet", view: "devices" };
	const crumbs: Crumb[] = [
		{
			label: t("device.header.crumbDevices", "Devices"),
			href: href(fleet),
			onNavigate: () => navigate(fleet),
		},
		{ label: <span className="font-mono">{page.name}</span> },
	];
	const booted = page.inspection?.host?.booted_at;
	return (
		<ObjectHeader
			crumbs={crumbs}
			glyph={
				<PresenceGlyph
					kind={view.presence.kind}
					label={enumLabel(t, "presence", view.presence.kind)}
				/>
			}
			name={page.name}
			nameChips={
				<RelationshipChip
					relationship={view.relationship}
					ownerName={ownerName}
					endsAt={view.row.access_expires_at ?? undefined}
				/>
			}
			chips={
				<>
					<PresenceChip kind={view.presence.kind} since={view.presence.since} />
					{page.revoked ? null : (
						<HealthChip
							level={view.health}
							count={
								view.health === "critical" || view.health === "attention"
									? counted.length
									: undefined
							}
						/>
					)}
					{page.consentOnly ? null : <DeviceKeyChip page={page} />}
					<DeviceSafetyChip page={page} />
					<IdleLockChip page={page} />
				</>
			}
			facts={
				page.consentOnly || page.revoked
					? []
					: [
							{
								id: "platform",
								label: t("device.header.platform", "Platform"),
								value: <PlatformFact page={page} />,
							},
							{
								id: "agent",
								label: t("device.header.agent", "Agent"),
								value: <AgentFact page={page} />,
							},
							...(booted
								? [
										{
											id: "boot",
											label: t("device.header.lastRestart", "Last restart"),
											value: (
												<span title={time.abs(booted)}>{time.at(booted)}</span>
											),
										},
									]
								: []),
							...(lane && laneTellsSomething(lane.ticks)
								? [
										{
											id: "lane",
											label: t("device.header.checkIns", "Check-ins"),
											value: (
												<CheckinLane ticks={lane.ticks} endAt={lane.endAt} />
											),
										},
									]
								: []),
						]
			}
			extra={
				<>
					<IdRef
						id={page.deviceId}
						label={t("device.header.deviceId", "Device ID")}
						copyLabel={t("device.header.copyId", "Copy device ID")}
					/>
					<YourAccess page={page} />
				</>
			}
			actions={
				<div className="flex max-w-full flex-wrap items-start justify-end gap-2">
					<PrimaryActions page={page} deploy={deploy} onOpenKeys={onOpenKeys} />
					<OverflowMenu page={page} onRevoke={onRevoke} />
				</div>
			}
		/>
	);
}

/** The reason line under a control that can't run, for callers that render their own button. */
export function GateReason({ result }: Readonly<{ result: GateResult }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const view = gateView(t, time, result);
	if (!view) return null;
	return <GateInline kind={view.gate.kind}>{view.gate.reason}</GateInline>;
}
