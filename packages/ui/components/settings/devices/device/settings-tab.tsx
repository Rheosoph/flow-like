"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	IdCard,
	Layers,
	LockKeyhole,
	type LucideIcon,
	Package,
	Power,
	Server,
	Shield,
} from "lucide-react";
import type { ReactNode } from "react";
import { agentSupports } from "../../../../lib/device-management/agent-reads";
import { groupFingerprint } from "../../../../lib/device-management/fingerprint";
import { keysLocked } from "../../../../lib/device-management/model/device-view";
import type { DeviceRoute } from "../../../../lib/device-management/model/types";
import { humanFileSize } from "../../../../lib/utils";
import { enumLabel } from "../copy/enum-labels";
import { useAgentReleaseVerdict } from "../hub/hub-view";
import { useAreaTime } from "../primitives/area-context";
import { Banner } from "../primitives/banner";
import { Block } from "../primitives/block";
import { dayText } from "../primitives/day";
import {
	FreshnessStamp,
	MixedSourcesStamp,
} from "../primitives/freshness-stamp";
import { IdRef } from "../primitives/id-ref";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { StateView } from "../primitives/state-view";
import { TONE_TEXT, cx } from "../primitives/tone";
import { ACCOUNT_SCOPE } from "../routing/devices-route";
import { useDevicesRoute, useRouteLink } from "../routing/use-devices-route";
import { stampOf } from "../shell/attention-popover";
import { useDeviceRows, useFleetDeviceStates } from "../workspace";
import { CapacityBlock } from "./capacity-block";
import { DangerZone } from "./danger-zone";
import {
	AgentUpdateActions,
	HostOperationLine,
	RebootAction,
} from "./host-operations";
import { resourceSamples } from "./overview-tab";
import { RenameControl } from "./rename-sheet";
import { StatusSubscription } from "./status-subscription";
import { type DevicePage, platformLabel } from "./use-device-page";

const LINK =
	"underline decoration-border-strong underline-offset-2 hover:decoration-current";
const KNOWN_SANDBOX_REASON =
	"linux_sandbox requires Linux; use trusted_process only for a dedicated trusted account";
const EXT4_QUOTA = "enforced ext4 project quota";

const amount = (value: unknown): number | undefined =>
	typeof value === "number" && Number.isFinite(value) && value >= 0
		? value
		: undefined;

type LayerId =
	| "host"
	| "agent"
	| "isolation"
	| "capacity"
	| "subscription"
	| "identity";

/** One layer of the device (SPEC §5.2): a labelled card on the connector line. */
function Layer({
	id,
	layer,
	icon: Icon,
	title,
	stamp,
	children,
}: Readonly<{
	id: LayerId;
	layer: string;
	icon: LucideIcon;
	title: string;
	stamp?: ReactNode;
	children: ReactNode;
}>) {
	return (
		<Block
			id={`device-layer-${id}`}
			title={<LayerTitle layer={layer} icon={Icon} title={title} />}
			stamp={stamp}
		>
			{children}
		</Block>
	);
}

function LayerTitle({
	layer,
	icon: Icon,
	title,
	danger = false,
}: Readonly<{
	layer: string;
	icon: LucideIcon;
	title: string;
	danger?: boolean;
}>) {
	return (
		<span className="inline-flex items-center gap-2.5">
			<span
				className={cx(
					"rounded-sm border px-1.5 py-px text-label font-semibold uppercase tracking-[0.08em]",
					danger
						? "border-critical-line text-critical"
						: "border-border text-muted-foreground",
				)}
			>
				{layer}
			</span>
			<Icon
				aria-hidden
				className={cx(
					"size-4 shrink-0",
					danger ? "text-critical" : "text-ink-2",
				)}
			/>
			{title}
		</span>
	);
}

/** R5 for the layers a live read fills: its age, or why there is none. */
function LiveStamp({ page }: Readonly<{ page: DevicePage }>) {
	if (page.inspectionSource)
		return <FreshnessStamp {...stampOf(page.inspectionSource)} />;
	return (
		<FreshnessStamp
			source="live"
			age={keysLocked(page.view.keys) ? "locked" : "notloaded"}
		/>
	);
}

function Connector() {
	return <div aria-hidden className="ml-6 h-4 w-px bg-border-strong" />;
}

const NOT_KNOWN = "–";
const ADDRESS_CAP = 4;

/** BG20: the addresses the device reports for its own interfaces (live read, whole-device View status). */
function NetworkAddresses({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const inspection = page.inspection;
	if (!inspection)
		return (
			<span className="text-muted-foreground">
				{t("device.settings.unknownUntilLive", "Unknown until it is read live")}
			</span>
		);
	if (!agentSupports(inspection.features, "network_interfaces"))
		return (
			<span className="text-muted-foreground">
				{t(
					"device.settings.addressesUnsupported",
					"Not reported by this agent version",
				)}
			</span>
		);
	if (!inspection.network)
		return (
			<span className="text-muted-foreground">
				{page.owner
					? t("device.settings.addressesLeftOut", "Not included in this read.")
					: t(
							"device.settings.addressesNone",
							"Not included in this read. They need whole-device View status.",
						)}
			</span>
		);
	const addresses = inspection.network.interfaces
		.filter((entry) => !entry.loopback)
		.flatMap((entry) => entry.addresses);
	if (!addresses.length)
		return (
			<span className="text-muted-foreground">
				{t("device.settings.addressesEmpty", "None reported")}
			</span>
		);
	return (
		<span className="inline-flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
			{addresses.slice(0, ADDRESS_CAP).map((address) => (
				<span key={address} className="font-mono text-[12.5px]">
					{address}
				</span>
			))}
			{addresses.length > ADDRESS_CAP ? (
				<span className="text-xs text-muted-foreground">
					{t("device.settings.addressesMore", "and {{count, number}} more", {
						count: addresses.length - ADDRESS_CAP,
					})}
				</span>
			) : null}
		</span>
	);
}

/** BG10: the agent's background loops; the open items on Overview name a failing one. */
function BackgroundTasks({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const inspection = page.inspection;
	if (!inspection)
		return (
			<span className="text-muted-foreground">
				{t("device.settings.unknownUntilLive", "Unknown until it is read live")}
			</span>
		);
	const tasks = inspection.tasks;
	if (!tasks || !agentSupports(inspection.features, "task_health"))
		return (
			<span className="text-muted-foreground">
				{t(
					"device.settings.tasksUnsupported",
					"Not reported by this agent version",
				)}
			</span>
		);
	const failing = tasks.filter((task) => task.state !== "ok").length;
	if (!tasks.length)
		return <>{t("device.settings.tasksNone", "None reported")}</>;
	return failing ? (
		<span className="text-warning">
			{t("device.settings.tasksFailing", {
				count: failing,
				total: tasks.length,
				defaultValue_one:
					"{{count, number}} of {{total, number}} is failing · see Overview",
				defaultValue_other:
					"{{count, number}} of {{total, number}} are failing · see Overview",
			})}
		</span>
	) : (
		<>
			{t("device.settings.tasksOk", {
				count: tasks.length,
				defaultValue_one: "{{count, number}} running normally",
				defaultValue_other: "All {{count, number}} running normally",
			})}
		</>
	);
}

function HostLayer({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const fleet = useFleetDeviceStates()[page.deviceId];
	const snapshot = fleet?.metrics?.find(
		(entry) => entry.scope.kind === "device",
	);
	const sample = resourceSamples(snapshot?.sample, snapshot?.observedAt).at(
		-1,
	)?.data;
	const inspection = page.inspection;
	const platform = inspection?.isolation?.platform;
	const cpus = amount(sample?.logical_cpus);
	const memory = amount(sample?.memory_total_bytes);
	const volume = sample?.storage_volume as
		| { total_bytes?: unknown }
		| undefined;
	const disk = amount(volume?.total_bytes);
	const booted = inspection?.host?.booted_at;
	return (
		<Layer
			id="host"
			layer={t("device.settings.layer.host", "Host")}
			icon={Server}
			title={t("device.settings.host", "Host")}
			stamp={<LiveStamp page={page} />}
		>
			<KeyValueList>
				<KvRow label={t("device.settings.platform", "Platform")}>
					{platform ? (
						platformLabel(t, platform)
					) : (
						<span className="text-muted-foreground">
							{t(
								"device.settings.unknownUntilLive",
								"Unknown until it is read live",
							)}
						</span>
					)}
				</KvRow>
				<KvRow label={t("device.settings.cpus", "Logical CPUs")}>
					{cpus === undefined ? (
						NOT_KNOWN
					) : (
						<span className="font-mono tabular-nums">{cpus}</span>
					)}
				</KvRow>
				<KvRow label={t("device.settings.memory", "Memory")}>
					{memory === undefined ? NOT_KNOWN : humanFileSize(memory)}
				</KvRow>
				<KvRow label={t("device.settings.disk", "Data disk")}>
					{disk === undefined ? NOT_KNOWN : humanFileSize(disk)}
				</KvRow>
				<KvRow label={t("device.settings.lastBoot", "Last boot")}>
					{booted ? (
						<span title={time.abs(booted)}>{time.at(booted)}</span>
					) : (
						<span className="text-muted-foreground">
							{inspection && !inspection.host
								? t(
										"device.settings.lastBootUnsupported",
										"Not reported by this agent version",
									)
								: t(
										"device.settings.unknownUntilLive",
										"Unknown until it is read live",
									)}
						</span>
					)}
				</KvRow>
				<KvRow label={t("device.settings.addresses", "Network addresses")}>
					<NetworkAddresses page={page} />
				</KvRow>
			</KeyValueList>
			<HostOperationLine operation={inspection?.hostOperation} kind="reboot" />
			<RebootAction page={page} />
		</Layer>
	);
}

/** The hub's release on the device page: the same verdict as Hub status, in one line. */
function LatestRelease() {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const link = useRouteLink();
	const verdict = useAgentReleaseVerdict();
	const hubStatus = (
		<a {...link({ screen: "hub" }, { scope: ACCOUNT_SCOPE })} className={LINK}>
			{t("device.settings.hubStatus", "Hub status")}
		</a>
	);
	if (verdict.kind === "ok" || verdict.kind === "ends_soon") {
		const { manifest, signerFingerprint } = verdict.release;
		return (
			<span className="inline-flex flex-wrap items-baseline gap-x-1.5">
				<span className="font-mono">{manifest.release_version}</span>
				<span>
					{t(
						"device.settings.releaseFacts",
						"· release number {{sequence, number}} · signed by",
						{ sequence: manifest.sequence },
					)}
				</span>
				<span className="font-mono">
					{groupFingerprint(signerFingerprint.slice(0, 8))}
				</span>
				{verdict.kind === "ends_soon" ? (
					<span className={TONE_TEXT.warning}>
						{t("device.settings.releaseExpires", "· runs out on {{date}}", {
							date: dayText(time, manifest.expires_at),
						})}
					</span>
				) : null}
				<span aria-hidden>·</span>
				{hubStatus}
			</span>
		);
	}
	if (verdict.kind === "expired" || verdict.kind === "failed")
		return (
			<span className="inline-flex flex-wrap items-baseline gap-x-1.5">
				<span className={TONE_TEXT.critical}>
					{verdict.kind === "expired"
						? t(
								"device.settings.releaseRanOut",
								"The hub's agent release ran out on {{date}}",
								{ date: dayText(time, verdict.facts.expires_at) },
							)
						: t(
								"device.settings.releaseFailed",
								"The hub's agent release failed a check",
							)}
				</span>
				<span aria-hidden>·</span>
				{hubStatus}
			</span>
		);
	const waiting: Record<typeof verdict.kind, string> = {
		missing: t(
			"device.settings.releaseNone",
			"This hub has no trusted agent releases set up",
		),
		unfetched: t(
			"device.settings.releaseError",
			"Couldn't be read from the hub",
		),
		checking: t("device.settings.releaseLoading", "Reading…"),
		waiting: t("device.settings.releaseLoading", "Reading…"),
	};
	return <span className="text-muted-foreground">{waiting[verdict.kind]}</span>;
}

function AgentLayer({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const agent = page.view.agent;
	const hostOperations = page.inspection?.hostOperations;
	return (
		<Layer
			id="agent"
			layer={t("device.settings.layer.agent", "Agent")}
			icon={Package}
			title={t("device.settings.agent", "Agent")}
			stamp={
				agent ? (
					<FreshnessStamp {...stampOf(agent.source)} />
				) : (
					<LiveStamp page={page} />
				)
			}
		>
			<p className="text-sm text-muted-foreground">
				{t(
					"device.settings.agentIncludesRuntime",
					"The agent binary contains the orchestrator and workflow runtime. Updating the agent replaces both.",
				)}
			</p>
			<KeyValueList>
				<KvRow
					label={t("device.settings.running", "Running")}
					provenance={
						agent
							? agent.source.age === "live"
								? t("device.settings.liveRead", "live read")
								: agent.source.at === undefined
									? undefined
									: t("device.settings.readAgo", "read {{ago}}", {
											ago: time.ago(agent.source.at),
										})
							: undefined
					}
				>
					{agent ? (
						<span className="font-mono">{agent.version}</span>
					) : (
						<span className="text-muted-foreground">
							{t("device.settings.notReported", "Not reported")}
						</span>
					)}
				</KvRow>
				<KvRow
					label={t("device.settings.latestRelease", "Latest verified release")}
				>
					<LatestRelease />
				</KvRow>
				<KvRow label={t("device.settings.remoteUpdate", "Remote update")}>
					{hostOperations === undefined
						? t(
								"device.settings.unknownUntilLive",
								"Unknown until it is read live",
							)
						: hostOperations.update_agent
							? page.inspection?.isolation?.platform === "macos"
								? t("device.settings.remoteUpdateMac", "macOS with launchd")
								: t(
										"device.settings.remoteUpdateSupported",
										"Available on this device",
									)
							: t(
									"device.settings.remoteUpdateNo",
									"Not supported on this system: update the agent from the device itself",
								)}
				</KvRow>
				<KvRow label={t("device.settings.tasks", "Background tasks")}>
					<BackgroundTasks page={page} />
				</KvRow>
			</KeyValueList>
			<HostOperationLine
				operation={page.inspection?.hostOperation}
				kind="update_agent"
			/>
			<AgentUpdateActions page={page} />
		</Layer>
	);
}

function IsolationLayer({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const inspection = page.inspection;
	const facts = inspection?.isolation;
	const mode = inspection?.hostIsolation ?? undefined;
	const title = t("device.settings.isolation", "Isolation");
	if (!facts)
		return (
			<Layer
				id="isolation"
				layer={t("device.settings.layer.isolation", "Isolation")}
				icon={Shield}
				title={title}
				stamp={<LiveStamp page={page} />}
			>
				<StateView
					kind="notloaded"
					title={t("device.settings.isolationNotRead", "Not read yet")}
					text={t(
						"device.settings.isolationNotReadText",
						"Isolation details come from a live read of the device. They need whole-device View status.",
					)}
				/>
			</Layer>
		);
	const hint =
		mode === "required"
			? t(
					"device.settings.isolationRequired",
					"Every service runs sandboxed with CPU, memory, process and disk limits.",
				)
			: mode === "optional"
				? t(
						"device.settings.isolationOptional",
						"A service runs sandboxed when its settings ask for it; others run as the agent with full device access.",
					)
				: facts.reason === KNOWN_SANDBOX_REASON
					? t(
							"device.settings.isolationNeedsLinux",
							"Sandboxing needs Linux. Services here run as the agent with full device access, so use a dedicated account.",
						)
					: t(
							"device.settings.isolationNone",
							"Services here run as the agent with full device access, so use a dedicated account.",
						);
	return (
		<Layer
			id="isolation"
			layer={t("device.settings.layer.isolation", "Isolation")}
			icon={Shield}
			title={title}
			stamp={<LiveStamp page={page} />}
		>
			<KeyValueList>
				<KvRow label={t("device.settings.isolation", "Isolation")}>
					{enumLabel(t, "hostIsolation", mode ?? "unknown")}
					<p className="mt-0.5 text-xs text-muted-foreground">{hint}</p>
				</KvRow>
			</KeyValueList>
			{facts.sandbox_available ? (
				<Banner
					tone="warning"
					title={t(
						"device.settings.networkTitle",
						"Sandboxed services still share the device's network.",
					)}
				>
					{t(
						"device.settings.networkText",
						"They can reach local and cloud metadata addresses. Don't run services from people you don't trust with that network.",
					)}
				</Banner>
			) : null}
			<KeyValueList>
				<KvRow label={t("device.settings.landlock", "Landlock ABI")}>
					{facts.landlock_abi == null ? (
						NOT_KNOWN
					) : (
						<span className="font-mono tabular-nums">{facts.landlock_abi}</span>
					)}
				</KvRow>
				<KvRow label={t("device.settings.diskLimits", "Disk limits")}>
					{facts.disk_requirement === undefined
						? NOT_KNOWN
						: facts.disk_requirement === EXT4_QUOTA
							? t("device.settings.diskQuota", "Enforced ext4 project quota")
							: t(
									"device.settings.diskOther",
									"As the device reports: {{value}}",
									{
										value: facts.disk_requirement,
									},
								)}
				</KvRow>
			</KeyValueList>
		</Layer>
	);
}

function IdentityLayer({
	page,
	renaming,
	onRenaming,
}: Readonly<{
	page: DevicePage;
	renaming: boolean;
	onRenaming(open: boolean): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const rows = useDeviceRows();
	const { row } = page.view;
	return (
		<Layer
			id="identity"
			layer={t("device.settings.layer.identity", "Identity")}
			icon={IdCard}
			title={t("device.settings.identity", "Identity")}
			stamp={<FreshnessStamp {...stampOf(rows.freshness)} />}
		>
			<KeyValueList>
				<KvRow label={t("device.settings.name", "Name")}>
					<span className="font-mono">{page.name}</span>
					{row.display_name ? (
						<span className="ml-1.5 text-xs text-muted-foreground">
							<Trans
								t={t}
								i18nKey="device.settings.setupName"
								defaults="set up as <1/>"
								components={{
									1: <span className="font-mono">{row.name}</span>,
								}}
							/>
						</span>
					) : (
						<span className="ml-1.5 text-xs text-muted-foreground">
							{t("device.settings.nameFromSetup", "set at setup")}
						</span>
					)}
				</KvRow>
				<KvRow label={t("device.settings.deviceId", "Device ID")}>
					<IdRef
						id={page.deviceId}
						copyLabel={t("device.settings.copyDeviceId", "Copy device ID")}
					/>
				</KvRow>
				<KvRow label={t("device.settings.keyGeneration", "Key generation")}>
					<span className="font-mono tabular-nums">{row.auth_epoch}</span>
				</KvRow>
				<KvRow label={t("device.settings.registered", "Registered")}>
					<span title={time.abs(row.registered_at)}>
						{time.at(row.registered_at)}
					</span>
				</KvRow>
			</KeyValueList>
			{page.owner ? (
				<RenameControl page={page} open={renaming} onOpenChange={onRenaming} />
			) : (
				<p className="text-xs text-muted-foreground">
					{t(
						"device.settings.renameOwner",
						"Only the owner can rename this device.",
					)}
				</p>
			)}
		</Layer>
	);
}

export interface DeviceSettingsTabProps {
	page: DevicePage;
	/** One-shot route action: open the revoke flow or the rename sheet. */
	action?: DeviceRoute["action"];
}

/** SPEC §5.2 Device settings: the device as layers, from the host down to its identity. */
export function DeviceSettingsTab({
	page,
	action,
}: Readonly<DeviceSettingsTabProps>) {
	const { t } = useTranslation("devices");
	const { clearParam, navigate } = useDevicesRoute();
	const fleet = useFleetDeviceStates()[page.deviceId];
	const renaming = action === "rename";
	const setRenaming = (open: boolean) => {
		if (open)
			navigate(
				{
					screen: "device",
					deviceId: page.deviceId,
					tab: "settings",
					action: "rename",
				},
				{ replace: true },
			);
		else clearParam("action");
	};
	const layers: ReactNode[] = [
		<HostLayer key="host" page={page} />,
		<AgentLayer key="agent" page={page} />,
		<IsolationLayer key="isolation" page={page} />,
		<Layer
			key="capacity"
			id="capacity"
			layer={t("device.settings.layer.capacity", "Capacity")}
			icon={Layers}
			title={t("device.settings.capacity", "Capacity")}
			stamp={<MixedSourcesStamp />}
		>
			<CapacityBlock page={page} />
		</Layer>,
		<Layer
			key="subscription"
			id="subscription"
			layer={t("device.settings.layer.subscription", "Subscription")}
			icon={LockKeyhole}
			title={t("device.settings.subscription", "Encrypted status subscription")}
			stamp={
				fleet ? <FreshnessStamp {...stampOf(fleet.freshness.status)} /> : null
			}
		>
			<StatusSubscription page={page} />
		</Layer>,
		<IdentityLayer
			key="identity"
			page={page}
			renaming={renaming}
			onRenaming={setRenaming}
		/>,
	];
	return (
		<div data-device-settings="" className="flex min-w-0 flex-col">
			{layers.map((layer, index) => (
				// biome-ignore lint/suspicious/noArrayIndexKey: the layer order is fixed
				<div key={index} className="flex min-w-0 flex-col">
					{index ? <Connector /> : null}
					{layer}
				</div>
			))}
			{page.owner && !page.revoked ? (
				<div className="flex min-w-0 flex-col">
					<Connector />
					<DangerZone
						page={page}
						title={
							<LayerTitle
								danger
								layer={t("device.settings.layer.danger", "Danger")}
								icon={Power}
								title={t("device.settings.danger", "Danger zone")}
							/>
						}
						autoOpen={action === "revoke"}
						onAutoOpened={() => clearParam("action")}
					/>
				</div>
			) : null}
		</div>
	);
}
