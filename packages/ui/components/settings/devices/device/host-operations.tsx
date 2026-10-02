"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CircleCheck,
	CircleHelp,
	CircleX,
	LoaderCircle,
	type LucideIcon,
	Package,
	Power,
	RefreshCw,
	Undo2,
} from "lucide-react";
import { useState } from "react";
import { compareVersions } from "../../../../lib/device-management/model/device-view";
import type {
	GateResult,
	HostOperationView,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import type { ManagementResponse } from "../../../../lib/device-management/types";
import { type EnumValues, enumLabel } from "../copy/enum-labels";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import type { ConsequenceRows } from "../primitives/consequence-preview";
import { DvButton } from "../primitives/dv-button";
import { GatedAction } from "../primitives/gate-notice";
import { IdRef } from "../primitives/id-ref";
import { InlineResult } from "../primitives/inline-result";
import { StatusChip } from "../primitives/status-chip";
import type { ChipTone } from "../primitives/tone";
import { useActivityTray } from "../shell/activity-tray";
import {
	type GateTarget,
	actionResultKey,
	serviceGateExtra,
	useDeviceAction,
	useGate,
	useInlineResults,
	useReleaseTrust,
} from "../workspace";
import { GateFix } from "./device-header";
import {
	type DevicePage,
	type GateView,
	gateView,
	platformLabel,
} from "./use-device-page";

const RUNNING_UPDATE = new Set(["validating", "activating", "rolling_back"]);
const OPEN_OPERATION = new Set([
	"pending",
	"requesting",
	"requested",
	"draining",
	"staging",
]);
/** A management message carries at most 16 KiB; the signed manifest must fit beside its envelope. */
const MAX_RELEASE_JWS = 13_000;

const updating = (service: ServiceView) =>
	service.conv === "update_in_progress" ||
	(!!service.rollout && RUNNING_UPDATE.has(service.rollout.state));

/** What blocks a reboot or agent update besides permissions: a running update, another device operation. */
export function hostGateTarget(page: DevicePage): GateTarget {
	const busy = page.services?.find(updating);
	const operation = page.inspection?.hostOperation?.state;
	const extra = busy
		? serviceGateExtra(busy, operation)
		: serviceGateExtra(undefined, operation);
	return {
		extra: { ...extra, activeRollout: !!busy },
		...(busy ? { labels: { service: busy.serviceId } } : {}),
	};
}

function useHostGate(
	page: DevicePage,
	action: "reboot" | "agent_update",
	releaseTrust?: boolean,
): GateResult {
	const target = hostGateTarget(page);
	return useGate(action, page.deviceId, {
		...target,
		extra: {
			...target.extra,
			...(releaseTrust === undefined ? {} : { releaseTrust }),
		},
	});
}

function blockedByIdentity(t: DevicesT, page: DevicePage): GateView | null {
	return page.identity
		? {
				gate: {
					kind: "policy",
					reason: t(
						"devices:device.identity.blockedShort",
						"Management is blocked until the identity is confirmed.",
					),
				},
			}
		: null;
}

function runningServices(page: DevicePage) {
	const rows = page.services ?? [];
	return {
		running: rows
			.filter((row) => row.desired === "running")
			.map((row) => row.serviceId),
		stopped: rows
			.filter((row) => row.desired === "stopped")
			.map((row) => row.serviceId),
	};
}

/** SPEC §6.5 "Reboot device" rows; the agent update adds what gets installed. */
function useHostRows(page: DevicePage): (first?: string) => ConsequenceRows {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const list = (values: readonly string[]) =>
		new Intl.ListFormat(time.locale, { type: "conjunction" }).format(values);
	return (first) => {
		const { running, stopped } = runningServices(page);
		const restart = running.length
			? t("device.host.rows.whatServices", {
					count: running.length,
					device: page.name,
					services: list(running),
					defaultValue_one:
						"{{device}} restarts. {{services}} stops, then starts again as requested.",
					defaultValue_other:
						"{{device}} restarts. {{services}} stop, then start again as requested.",
				})
			: t("device.host.rows.what", "{{device}} restarts.", {
					device: page.name,
				});
		return {
			what: first ? `${first} ${restart}` : restart,
			who: running.length
				? t(
						"device.host.rows.who",
						"Everything these services serve is down for about 1–2 minutes.",
					)
				: t("device.host.rows.whoNone", "Nobody: no service is running."),
			...(stopped.length
				? {
						stays: t("device.host.rows.stays", {
							count: stopped.length,
							services: list(stopped),
							defaultValue_one: "{{services}} stays stopped, as you asked.",
							defaultValue_other: "{{services}} stay stopped, as you asked.",
						}),
					}
				: {}),
			when: t(
				"device.host.rows.when",
				"After the device finishes its current commands.",
			),
			undo: {
				reversible: null,
				text: t(
					"device.host.rows.undo",
					"Not needed. Progress shows in Activity.",
				),
			},
		};
	};
}

function Results({ scopeKey }: Readonly<{ scopeKey: string }>) {
	const { t } = useTranslation("devices");
	const results = useInlineResults(scopeKey);
	return (
		<>
			{results.map((result) => (
				<InlineResult
					key={result.id}
					tone={result.tone}
					onDismiss={result.dismiss}
					actions={
						result.activity ? (
							<DvButton
								size="xs"
								variant="link"
								onClick={() => useActivityTray.getState().setOpen(true)}
							>
								{t("device.host.follow", "Follow in activity")}
							</DvButton>
						) : null
					}
				>
					{result.text}
				</InlineResult>
			))}
		</>
	);
}

/** "Remote update and reboot need Linux with systemd": shown instead of controls that can't apply (R7). */
function NotOnThisSystem({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const platform = page.inspection?.isolation?.platform;
	return (
		<p className="text-xs text-muted-foreground">
			{platform
				? t(
						"device.host.needsLinuxOn",
						"Remote update and reboot need Linux with systemd. Update and restart this {{platform}} device from the device itself.",
						{ platform: platformLabel(t, platform) },
					)
				: t(
						"device.host.needsLinux",
						"Remote update and reboot need Linux with systemd. Update and restart this device from the device itself.",
					)}
		</p>
	);
}

/** Host layer action: Reboot device… (check + consequence preview, tracked in Activity). */
export function RebootAction({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const actions = useDeviceAction();
	const rows = useHostRows(page);
	const result = useHostGate(page, "reboot");
	const resultKey = actionResultKey("reboot", page.deviceId);
	if (!result.ok && result.hide) return <NotOnThisSystem page={page} />;
	const view = blockedByIdentity(t, page) ?? gateView(t, time, result);
	const bootId = page.inspection?.boot_id ?? null;
	const run = () =>
		actions.run<ManagementResponse>({
			action: "reboot",
			deviceId: page.deviceId,
			target: hostGateTarget(page),
			label: t("device.host.rebootLabel", "Reboot {{device}}", {
				device: page.name,
			}),
			consequence: rows(),
			strength: "check",
			confirm: {
				title: t("device.host.rebootTitle", "Reboot {{device}}?", {
					device: page.name,
				}),
				checkLabel: t(
					"device.host.rebootCheck",
					"Interrupt the running services now",
				),
				tone: "danger",
				icon: Power,
			},
			resultKey,
			call: (context) =>
				context.request({ type: "reboot", expected_boot_id: bootId }),
			activity: {
				kind: "reboot",
				deviceName: page.name,
				href: { screen: "device", deviceId: page.deviceId, tab: "settings" },
				resume: (response) => ({
					type: "host_operation",
					kind: "reboot",
					operationId: response.operation_id,
					bootIdBefore: bootId,
				}),
			},
		});
	const button = (
		<DvButton
			icon={Power}
			busy={actions.pending(resultKey)}
			onClick={() => void run()}
		>
			{t("device.host.reboot", "Reboot device…")}
		</DvButton>
	);
	return (
		<div className="flex flex-col items-start gap-2">
			{view ? (
				<span className="inline-flex flex-wrap items-start gap-2">
					<GatedAction gate={view.gate}>{button}</GatedAction>
					<GateFix view={view} />
				</span>
			) : (
				button
			)}
			<Results scopeKey={resultKey} />
		</div>
	);
}

/** Agent layer actions: check the hub for a newer verified release, and install it. */
export function AgentUpdateActions({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const actions = useDeviceAction();
	const rows = useHostRows(page);
	const release = useReleaseTrust();
	const result = useHostGate(page, "agent_update", release.configured);
	const resultKey = actionResultKey("agent_update", page.deviceId);
	const [checked, setChecked] = useState<{ at: number; version?: string }>();
	const [checking, setChecking] = useState(false);
	const hidden = !result.ok && result.hide;
	const latest = release.data?.manifest;
	const current = page.view.agent?.version;
	const upToDate =
		!!latest &&
		!!current &&
		compareVersions(current, latest.release_version) >= 0;
	const bootId = page.inspection?.boot_id ?? null;
	const view: GateView | null =
		blockedByIdentity(t, page) ??
		(upToDate
			? {
					gate: {
						kind: "busy",
						reason: t(
							"device.agent.upToDate",
							"Already running the latest verified release, {{version}}.",
							{ version: current },
						),
					},
				}
			: gateView(t, time, result));
	const check = async () => {
		setChecking(true);
		try {
			await release.refetch();
		} finally {
			setChecking(false);
			setChecked({ at: Math.floor(time.now / 1000) });
		}
	};
	const update = () => {
		const verified = release.data;
		if (!verified) return;
		return actions.run<ManagementResponse>({
			action: "agent_update",
			deviceId: page.deviceId,
			target: {
				...hostGateTarget(page),
				extra: {
					...hostGateTarget(page).extra,
					releaseTrust: release.configured,
				},
			},
			label: t("device.agent.updateLabel", "Update the agent on {{device}}", {
				device: page.name,
			}),
			consequence: rows(
				current
					? t(
							"device.agent.rows.install",
							"Installs verified release {{version}} (release #{{sequence, number}}). If the new agent doesn't start, the device rolls back to {{current}}.",
							{
								version: verified.manifest.release_version,
								sequence: verified.manifest.sequence,
								current,
							},
						)
					: t(
							"device.agent.rows.installNoCurrent",
							"Installs verified release {{version}} (release #{{sequence, number}}). If the new agent doesn't start, the device rolls back.",
							{
								version: verified.manifest.release_version,
								sequence: verified.manifest.sequence,
							},
						),
			),
			strength: "check",
			confirm: {
				title: t(
					"device.agent.updateTitle",
					"Update the agent on {{device}}?",
					{
						device: page.name,
					},
				),
				checkLabel: t(
					"device.agent.updateCheck",
					"Stop the running services and restart the agent with the verified release",
				),
				icon: Package,
			},
			resultKey,
			call: (context) => {
				if (verified.manifestJws.length > MAX_RELEASE_JWS)
					throw new Error(
						t(
							"device.agent.manifestTooLarge",
							"The release manifest is too large to send to the device. Check Hub status.",
						),
					);
				return context.request({
					type: "update_agent",
					expected_boot_id: bootId,
					release_jws: verified.manifestJws,
				});
			},
			activity: {
				kind: "agent_update",
				deviceName: page.name,
				href: { screen: "device", deviceId: page.deviceId, tab: "settings" },
				resume: (response) => ({
					type: "host_operation",
					kind: "update_agent",
					operationId: response.operation_id,
					bootIdBefore: bootId,
				}),
			},
		});
	};
	if (hidden) return null;
	const updateButton = (
		<DvButton
			icon={Package}
			busy={actions.pending(resultKey)}
			onClick={() => void update()}
		>
			{t("device.agent.update", "Update agent…")}
		</DvButton>
	);
	const checkButton = (
		<DvButton icon={RefreshCw} busy={checking} onClick={() => void check()}>
			{t("device.agent.check", "Check for agent update")}
		</DvButton>
	);
	return (
		<div className="flex flex-col items-start gap-2">
			<div className="flex flex-wrap items-start gap-2">
				{release.configured ? (
					checkButton
				) : (
					<GatedAction
						gate={{
							kind: "hub",
							reason: t(
								"device.agent.noReleaseTrust",
								"This hub has no trusted agent releases set up.",
							),
						}}
					>
						{checkButton}
					</GatedAction>
				)}
				{view ? (
					<span className="inline-flex flex-wrap items-start gap-2">
						<GatedAction gate={view.gate}>{updateButton}</GatedAction>
						<GateFix view={view} />
					</span>
				) : (
					updateButton
				)}
			</div>
			{checked ? (
				<InlineResult
					tone={release.error ? "warning" : "good"}
					onDismiss={() => setChecked(undefined)}
				>
					{release.error
						? t(
								"device.agent.checkFailed",
								"Couldn't check at {{time}}. The release shown was read earlier.",
								{ time: time.clock(checked.at) },
							)
						: latest
							? t(
									"device.agent.checked",
									"Checked at {{time}}: {{version}} is the latest verified release.",
									{
										time: time.clock(checked.at),
										version: latest.release_version,
									},
								)
							: t("device.agent.checkedNone", "Checked at {{time}}.", {
									time: time.clock(checked.at),
								})}
				</InlineResult>
			) : null}
			<Results scopeKey={resultKey} />
		</div>
	);
}

interface OperationLook {
	tone: ChipTone;
	icon: LucideIcon;
	spin: boolean;
}

const OPERATION_LOOK: Record<string, OperationLook> = {
	completed: { tone: "good", icon: CircleCheck, spin: false },
	failed: { tone: "critical", icon: CircleX, spin: false },
	rolled_back: { tone: "warning", icon: Undo2, spin: false },
};
const OPERATION_OPEN: OperationLook = {
	tone: "info",
	icon: LoaderCircle,
	spin: true,
};
const OPERATION_UNKNOWN: OperationLook = {
	tone: "unknown",
	icon: CircleHelp,
	spin: false,
};

/**
 * BG15: the device's current or last reboot / agent update, whoever started
 * it. Older agents report none; then only this computer's own operations show
 * (in Activity and the inline results).
 */
export function HostOperationLine({
	operation,
	kind,
}: Readonly<{
	operation: HostOperationView | null | undefined;
	kind: HostOperationView["kind"];
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	if (!operation || operation.kind !== kind) return null;
	const state =
		kind === "reboot"
			? enumLabel(
					t,
					"rebootState",
					operation.state as EnumValues["rebootState"],
				)
			: enumLabel(
					t,
					"agentUpdateState",
					operation.state as EnumValues["agentUpdateState"],
				);
	const by = {
		you: t("device.host.byYou", "started by you"),
		owner: t("device.host.byOwner", "started by the owner"),
		another_person: t("device.host.byOther", "started by another person"),
	}[operation.issued_by];
	const look = OPEN_OPERATION.has(operation.state)
		? OPERATION_OPEN
		: (OPERATION_LOOK[operation.state] ?? OPERATION_UNKNOWN);
	return (
		<p
			data-host-operation={operation.kind}
			className="flex flex-wrap items-center gap-x-2 gap-y-1 text-ui"
		>
			<StatusChip tone={look.tone} icon={look.icon} spin={look.spin}>
				{state}
			</StatusChip>
			<span>
				{enumLabel(t, "hostOpKind", operation.kind)}
				{" · "}
				<span title={time.abs(operation.created_at)}>
					{time.at(operation.created_at)}
				</span>
				{" · "}
				{by}
			</span>
			<IdRef
				id={operation.operation_id}
				copyLabel={t("device.host.copyCommand", "Copy command ID")}
			/>
		</p>
	);
}
