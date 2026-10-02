"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	Cloud,
	Database,
	Play,
	RotateCw,
	ScrollText,
	Stethoscope,
} from "lucide-react";
import { type ReactNode, useMemo } from "react";
import type { DeploymentRolloutStatus } from "../../../../lib/device-management/deployment";
import {
	deviceName,
	isLastKnown,
} from "../../../../lib/device-management/model/device-view";
import type {
	AttentionItem,
	DeviceViewModel,
	OfflineWritesSummary,
	PlacementConfigFacts,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import type { OfflineQueueStatus } from "../../../../lib/device-management/offline-queue";
import type { ActivityItem } from "../../../../lib/device-management/workspace/types";
import type { DeviceResources } from "../../../../lib/device-resources";
import { formatMoney } from "../copy/attention-copy";
import { enumLabel } from "../copy/enum-labels";
import { gateCopy } from "../copy/gate-copy";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import { Banner } from "../primitives/banner";
import { Block } from "../primitives/block";
import {
	type CheckState,
	Checklist,
	type ChecklistItem,
} from "../primitives/checklist";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GatedAction } from "../primitives/gate-notice";
import { LOCKED_DATA_CLASS } from "../primitives/state-view";
import { useRouteLink } from "../routing/use-devices-route";
import { stampOf } from "../shell/attention-popover";
import {
	type ServiceCommand,
	useActivity,
	useAttention,
	useAttentionState,
	useDeviceResources,
	useOverlay,
	useServiceCommands,
} from "../workspace";
import {
	rolloutEndedAt,
	rolloutOutcome,
	rolloutPhase,
	rolloutSettings,
} from "./current-update";
import type { InstanceDiagnostics } from "./instances-table";
import { desiredRun, observedRun } from "./service-header";

export interface ServiceVerdict {
	lead: string;
	rest: string;
	tone: "info" | "warning" | "critical";
}

const NEEDS_YOU = new Set(["conflict", "blocked", "outcome_unknown"]);

export interface QueueFacts {
	waiting: number;
	/** Queues that wait for a person. */
	needYou: number;
	conflict: boolean;
	/** Changes kept in queues that are paused because cloud access changed. */
	paused: number;
	quarantined: boolean;
}

/** Buffered changes of a service: the live queues when read, else the agent's summary. */
export function queueFacts(
	service: ServiceView,
	queues: readonly OfflineQueueStatus[] | undefined,
	needsAttention: number | undefined,
): QueueFacts | undefined {
	if (queues) {
		const needing = queues.filter(
			(queue) =>
				queue.quarantined || (queue.head && NEEDS_YOU.has(queue.head.state)),
		);
		return {
			waiting: queues.reduce((sum, queue) => sum + queue.pending_count, 0),
			needYou: needing.length,
			conflict: queues.some((queue) => queue.head?.state === "conflict"),
			paused: queues
				.filter((queue) => queue.quarantined)
				.reduce((sum, queue) => sum + queue.pending_count, 0),
			quarantined: queues.some((queue) => queue.quarantined),
		};
	}
	const writes = service.offlineWrites;
	if (!writes || writes === "not_loaded") return undefined;
	const headNeeds = !!writes.head && NEEDS_YOU.has(writes.head.state);
	const fromRow = writes.quarantined || headNeeds ? 1 : 0;
	return {
		waiting: writes.pending,
		needYou: needsAttention ?? fromRow,
		conflict: writes.head?.state === "conflict",
		paused: 0,
		quarantined: writes.quarantined,
	};
}

interface VerdictInput {
	t: DevicesT;
	time: AreaTime;
	device: DeviceViewModel;
	serviceId: string;
	service: ServiceView | undefined;
	rollout: DeploymentRolloutStatus | undefined;
	command: ActivityItem | undefined;
	queues: QueueFacts | undefined;
	cloudInvalid: boolean;
}

const info = (lead: string, rest = ""): ServiceVerdict => ({
	lead,
	rest,
	tone: "info",
});

/** Locked with the summary the keys kept from the last read: what the service did then. */
function lockedSummaryVerdict(
	input: VerdictInput,
	name: string,
): ServiceVerdict | undefined {
	const { t, time, device, serviceId } = input;
	const summary = device.keys.lockedSummary;
	const last = summary?.services.find((row) => row.serviceId === serviceId);
	if (!summary || !last) return undefined;
	const at = time.clock(summary.readAt);
	const unlock = t(
		"devices:service.verdict.unlockNow",
		"Unlock {{device}} to see what it does now.",
		{ device: name },
	);
	if (last.conv === "update_in_progress")
		return info(
			t(
				"devices:service.verdict.lockedUpdating",
				"Was switching to new settings when last read at {{time}}.",
				{ time: at },
			),
			unlock,
		);
	if (last.conv === "crash_looping" || last.conv === "failed_stopped")
		return info(
			t(
				"devices:service.verdict.lockedCrashing",
				"Wasn't running as you asked when last read at {{time}}.",
				{ time: at },
			),
			unlock,
		);
	return info(
		t(
			"devices:service.verdict.lockedOk",
			"Ran as you asked when last read at {{time}}.",
			{ time: at },
		),
		unlock,
	);
}

function unreadableVerdict(input: VerdictInput): ServiceVerdict {
	const { t, device } = input;
	const name = deviceName(device.row);
	const hubCant = t(
		"devices:service.verdict.hubCant",
		"The hub can't read services; only keys on a computer can.",
	);
	if (device.presence.kind === "revoked")
		return info(
			t("devices:service.verdict.revoked", "{{device}} was revoked.", {
				device: name,
			}),
			t(
				"devices:service.verdict.revokedRest",
				"Revoked devices aren't read, so nothing is known about this service any more.",
			),
		);
	if (device.keys.state === "none")
		return info(
			t(
				"devices:service.verdict.noKeys",
				"This computer has no keys for {{device}}.",
				{ device: name },
			),
			hubCant,
		);
	if (device.presence.kind === "never")
		return info(
			t("devices:service.verdict.never", "{{device}} hasn't checked in yet.", {
				device: name,
			}),
			t(
				"devices:service.verdict.neverRest",
				"It sends its first encrypted status after its first check-in.",
			),
		);
	return (
		lockedSummaryVerdict(input, name) ??
		info(
			t(
				"devices:service.verdict.unlock",
				"Unlock {{device}} to see this service.",
				{ device: name },
			),
			hubCant,
		)
	);
}

const COMMAND_VERB: Record<string, (t: DevicesT) => string> = {
	start: (t) => t("devices:service.verdict.starting", "Starting"),
	stop: (t) => t("devices:service.verdict.stopping", "Stopping"),
	restart: (t) => t("devices:service.verdict.restarting", "Restarting"),
	scale: (t) =>
		t("devices:service.verdict.scaling", "Changing the instance count"),
};

function rolloutVerdict(
	input: VerdictInput,
	service: ServiceView,
	rollout: DeploymentRolloutStatus,
	readyText: string,
): ServiceVerdict | undefined {
	const { t, time } = input;
	const { from, to } = rolloutSettings(t, rollout, service);
	const many = service.instances.requested > 1;
	const endedAt = rolloutEndedAt(rollout);
	const ended = endedAt === undefined ? "" : time.clock(endedAt);
	switch (rolloutPhase(rollout)) {
		case "activating": {
			const lead = t(
				"devices:service.verdict.updating",
				"Updating to settings v{{to}} with a safe update.",
				{ to },
			);
			if (rollout.stable_since)
				return info(
					lead,
					t("devices:service.verdict.updatingStable", {
						count: service.instances.requested,
						to,
						from,
						seconds: rollout.stabilization_seconds ?? 10,
						defaultValue_one:
							"Instance #0 is ready with v{{to}} and must stay healthy for {{seconds}} s. If it doesn't, the device restores v{{from}} on its own.",
						defaultValue_other:
							"All {{count, number}} instances are ready with v{{to}} and must stay healthy for {{seconds}} s. If they don't, the device restores v{{from}} on its own.",
					}),
				);
			const starting =
				service.instances.max <= 1 && rollout.updated_at !== undefined
					? t(
							"devices:service.verdict.updatingOneSlot",
							"The previous version stopped at {{time}} and instance #0 is starting.",
							{ time: time.clock(rollout.updated_at) },
						)
					: many
						? t(
								"devices:service.verdict.updatingMany",
								"The new instances are starting.",
							)
						: t(
								"devices:service.verdict.updatingOne",
								"The new instance is starting.",
							);
			const fallback = rollout.deadline_at
				? t(
						"devices:service.verdict.updatingDeadline",
						"If it isn't healthy by {{time}}, the device restores v{{from}} on its own.",
						{ time: time.clock(rollout.deadline_at), from },
					)
				: t(
						"devices:service.verdict.updatingNoDeadline",
						"If it isn't healthy in time, the device restores v{{from}} on its own.",
						{ from },
					);
			return info(lead, `${starting} ${fallback}`);
		}
		case "validating":
			return info(
				t(
					"devices:service.verdict.validating",
					"Checking settings v{{to}} before switching over.",
					{ to },
				),
				t(
					"devices:service.verdict.validatingRest",
					"Settings v{{from}} keeps running until the new version passes its checks.",
					{ from },
				),
			);
		case "staged":
			return info(
				t(
					"devices:service.verdict.staged",
					"Settings v{{to}} is ready to switch over.",
					{ to },
				),
				t(
					"devices:service.verdict.stagedRest",
					"Settings v{{from}} keeps running until you activate it. Staged updates are discarded if not activated within 24 h.",
					{ from },
				),
			);
		case "rolling_back":
			return {
				lead: t(
					"devices:service.verdict.rollingBack",
					"Restoring settings v{{from}}.",
					{ from },
				),
				rest: t(
					"devices:service.verdict.rollingBackRest",
					"The update to v{{to}} didn't stay healthy, so the device is switching back on its own.",
					{ to },
				),
				tone: "warning",
			};
		case "rolled_back":
			return {
				lead: t(
					"devices:service.verdict.rolledBack",
					"Runs settings v{{from}} again.",
					{ from },
				),
				rest: t(
					"devices:service.verdict.rolledBackRest",
					"The update to v{{to}} didn't hold, and the device restored v{{from}} on its own at {{time}}. Check the logs before you try again.",
					{ to, from, time: ended },
				),
				tone: "warning",
			};
		case "not_applied":
			return {
				lead: t(
					"devices:service.verdict.notApplied",
					"Runs settings v{{from}} as before.",
					{ from },
				),
				rest: t(
					"devices:service.verdict.notAppliedRest",
					"The update to v{{to}} failed its checks at {{time}}, so it was never switched on.",
					{ to, time: ended },
				),
				tone: "warning",
			};
		case "failed_stopped":
			return {
				lead: t(
					"devices:service.verdict.failedStopped",
					"Stopped after a failed update.",
				),
				rest: t(
					"devices:service.verdict.failedStoppedRest",
					"The update to v{{to}} crashed and settings v{{from}} couldn't be restored. Start runs v{{from}} again.",
					{ to, from },
				),
				tone: "critical",
			};
		case "healthy":
			return service.conv === "converged"
				? info(
						t(
							"devices:service.verdict.converged",
							"Runs as you asked: settings v{{settings}}, {{ready}}.",
							{ settings: service.settings.applied ?? to, ready: readyText },
						),
						t("devices:service.verdict.updatedAt", "Updated at {{time}}.", {
							time: ended,
						}),
					)
				: undefined;
		default:
			return undefined;
	}
}

function stateVerdict(
	input: VerdictInput,
	service: ServiceView,
	readyText: string,
): ServiceVerdict {
	const { t } = input;
	switch (service.conv) {
		case "crash_looping": {
			const error = service.diagnostics?.lastError;
			return {
				lead: t("devices:service.verdict.crashing", "Crashing."),
				rest: error
					? t(
							"devices:service.verdict.crashingError",
							"{{ready}}. Last error: “{{error}}”",
							{ ready: readyText, error },
						)
					: t(
							"devices:service.verdict.crashingRest",
							"{{ready}}. The reason is only on the device.",
							{ ready: readyText },
						),
				tone: "critical",
			};
		}
		case "failed_stopped":
			return {
				lead: t(
					"devices:service.verdict.failedStopped",
					"Stopped after a failed update.",
				),
				rest: t(
					"devices:service.verdict.failedStoppedStart",
					"Start runs settings v{{settings}} again.",
					{ settings: service.settings.latest },
				),
				tone: "critical",
			};
		case "stopped_by_user":
			return info(
				t("devices:service.verdict.stopped", "Stopped, as you asked."),
				t(
					"devices:service.verdict.stoppedRest",
					"Start runs settings v{{settings}} again.",
					{ settings: service.settings.latest },
				),
			);
		case "converged":
			return info(
				t(
					"devices:service.verdict.converged",
					"Runs as you asked: settings v{{settings}}, {{ready}}.",
					{
						settings: service.settings.applied ?? service.settings.latest,
						ready: readyText,
					},
				),
			);
		case "unknown":
			return info(
				t("devices:service.verdict.unknown", "State unknown right now."),
				t(
					"devices:service.verdict.unknownRest",
					"The device is restarting or hasn't reported this service yet.",
				),
			);
		default:
			return info(
				t("devices:service.verdict.applying", "Applying your change."),
				`${readyText}.`,
			);
	}
}

function readyTextOf(t: DevicesT, service: ServiceView) {
	return t("devices:service.verdict.ready", {
		count: service.instances.requested,
		ready: service.instances.ready,
		defaultValue_one: "{{ready}} of {{count, number}} instance ready",
		defaultValue_other: "{{ready}} of {{count, number}} instances ready",
	});
}

function queueVerdict(
	t: DevicesT,
	base: ServiceVerdict,
	service: ServiceView,
	queues: QueueFacts,
): ServiceVerdict {
	const parts = [
		t("devices:service.verdict.writesWaiting", {
			count: queues.waiting,
			defaultValue_one: "{{count, number}} change waits to reach the cloud.",
			defaultValue_other: "{{count, number}} changes wait to reach the cloud.",
		}),
		queues.conflict
			? t(
					"devices:service.verdict.writesConflict",
					"One conflicts with newer cloud data.",
				)
			: "",
		queues.quarantined
			? queues.paused
				? t("devices:service.verdict.writesPaused", {
						count: queues.paused,
						defaultValue_one:
							"{{count, number}} is paused because cloud access changed.",
						defaultValue_other:
							"{{count, number}} are paused because cloud access changed.",
					})
				: t(
						"devices:service.verdict.writesPausedSome",
						"Some are paused because cloud access changed.",
					)
			: "",
	];
	return {
		lead:
			service.conv === "converged"
				? t(
						"devices:service.verdict.writesLead",
						"Runs as you asked, but buffered changes need you.",
					)
				: base.lead,
		rest: parts.filter(Boolean).join(" "),
		tone: base.tone === "critical" ? "critical" : "warning",
	};
}

export function serviceVerdict(input: VerdictInput): ServiceVerdict {
	const { t, time, device, service, rollout, command, queues } = input;
	if (!service) return unreadableVerdict(input);
	const name = deviceName(device.row);
	const readyText = readyTextOf(t, service);
	const lastKnown = isLastKnown(service.freshness);
	if (lastKnown && service.conv === "crash_looping") {
		const seen =
			service.freshness.at === undefined
				? t(
						"devices:service.verdict.crashingLastSeenNoTime",
						"Crashing when last seen.",
					)
				: t(
						"devices:service.verdict.crashingLastSeen",
						"Crashing when last seen, {{ago}}.",
						{ ago: time.ago(service.freshness.at) },
					);
		const offline =
			device.presence.kind === "offline" && device.presence.since
				? t(
						"devices:service.verdict.offlineSince",
						"{{device}} is offline since {{when}}, so nothing newer is known.",
						{ device: name, when: time.at(device.presence.since) },
					)
				: t(
						"devices:service.verdict.nothingNewer",
						"Nothing newer is known until {{device}} is read again.",
						{ device: name },
					);
		return { lead: seen, rest: `${readyText}. ${offline}`, tone: "critical" };
	}
	const verb =
		command && COMMAND_VERB[String(command.label.params?.command ?? "")];
	let verdict: ServiceVerdict;
	if (command && verb)
		verdict = info(
			t(
				"devices:service.verdict.command",
				"{{verb}}, as you asked at {{time}}.",
				{ verb: verb(t), time: time.clock(command.startedAt / 1000) },
			),
			t(
				"devices:service.verdict.commandRest",
				"Results show under the action bar and in Activity.",
			),
		);
	else
		verdict =
			(rollout && rolloutVerdict(input, service, rollout, readyText)) ||
			stateVerdict(input, service, readyText);
	if (queues && (queues.conflict || queues.quarantined))
		verdict = queueVerdict(t, verdict, service, queues);
	if (input.cloudInvalid)
		verdict = {
			lead: t(
				"devices:service.verdict.cloudRevoked",
				"Bound to a revoked approval.",
			),
			rest: t(
				"devices:service.verdict.cloudRevokedRest",
				"Its instances can't get cloud credentials: model calls and access to Project files fail, and buffered writes pause and are kept.",
			),
			tone: "critical",
		};
	return verdict;
}

export interface ServiceDiagnosis {
	verdict: ServiceVerdict;
	queues: QueueFacts | undefined;
	cloudInvalid: boolean;
	attention: AttentionItem[];
	endpoint: PlacementConfigFacts | undefined;
}

/**
 * Whether the service buffers writes, as far as this computer can tell: its
 * settings were read here and say so, or the device holds queues for it. The
 * agent's summary exists for every service, so its mere presence says nothing.
 */
function buffersWrites(
	endpoint: PlacementConfigFacts | undefined,
	queues: readonly OfflineQueueStatus[] = [],
	scopes = 0,
) {
	const settings = endpoint ? endpoint.offlineWrites : undefined;
	return settings !== undefined || queues.length > 0 || scopes > 0;
}

/** The page's conclusion (SPEC §6.4 N3) and the facts the diagnosis block lists under it. */
export function useServiceDiagnosis(
	device: DeviceViewModel,
	serviceId: string,
	service: ServiceView | undefined,
	rollout: DeploymentRolloutStatus | undefined,
	/** The agent's summary of the service's buffered writes, when it reports one. */
	summary: Pick<OfflineWritesSummary, "needs_attention" | "scopes"> | undefined,
): ServiceDiagnosis {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { input } = useAttentionState();
	const deviceId = device.row.device_id;
	const attention = useAttention({ deviceId, serviceId });
	const commands = useActivity({ deviceId, serviceId, kind: "command" });
	const command = commands.inProgress[0];
	const live = input.live[deviceId];
	const liveQueues = live?.offlineQueues?.[serviceId];
	const endpoint = live?.placements?.[serviceId];
	const needsAttention = summary?.needs_attention;
	const scopes = summary?.scopes;
	const queues = useMemo(() => {
		if (!service) return undefined;
		const facts = queueFacts(service, liveQueues, needsAttention);
		// A service that doesn't buffer has nothing to be "up to date" with.
		const used =
			buffersWrites(endpoint, liveQueues, scopes) || (facts?.waiting ?? 0) > 0;
		return used ? facts : undefined;
	}, [service, liveQueues, needsAttention, scopes, endpoint]);
	const cloudInvalid = attention.some(
		(item) => item.key === "cloud_access_invalid",
	);
	const verdict = useMemo(
		() =>
			serviceVerdict({
				t,
				time,
				device,
				serviceId,
				service,
				rollout,
				command,
				queues,
				cloudInvalid,
			}),
		[
			t,
			time,
			device,
			serviceId,
			service,
			rollout,
			command,
			queues,
			cloudInvalid,
		],
	);
	return { verdict, queues, cloudInvalid, attention, endpoint };
}

const LOOPBACK = new Set(["127.0.0.1", "::1", "localhost"]);

function rolloutCheck(
	t: DevicesT,
	time: AreaTime,
	service: ServiceView,
	rollout: DeploymentRolloutStatus,
): ChecklistItem | undefined {
	const { to } = rolloutSettings(t, rollout, service);
	const ended = rolloutEndedAt(rollout);
	const at = ended === undefined ? "" : time.clock(ended);
	const base = { id: "update" };
	switch (rolloutPhase(rollout)) {
		case "activating":
			return {
				...base,
				state: "active",
				label: rollout.stable_since
					? t(
							"devices:service.diagnosis.activatingStable",
							"Safe update to v{{to}}: switching over, must stay healthy",
							{ to },
						)
					: rollout.deadline_at
						? t(
								"devices:service.diagnosis.activating",
								"Safe update to v{{to}}: switching over, time limit {{time}}",
								{ to, time: time.clock(rollout.deadline_at) },
							)
						: t(
								"devices:service.diagnosis.activatingNoLimit",
								"Safe update to v{{to}}: switching over",
								{ to },
							),
			};
		case "validating":
			return {
				...base,
				state: "active",
				label: t(
					"devices:service.diagnosis.validating",
					"Safe update to v{{to}}: checking the new version",
					{ to },
				),
			};
		case "rolling_back":
			return {
				...base,
				state: "active",
				label: t(
					"devices:service.diagnosis.rollingBack",
					"Update to v{{to}}: the device is restoring the previous version",
					{ to },
				),
			};
		case "staged":
			return {
				...base,
				state: "warn",
				label: t(
					"devices:service.diagnosis.staged",
					"Safe update to v{{to}} is staged and waits for Activate",
					{ to },
				),
			};
		case "healthy":
			return {
				...base,
				state: "pass",
				label: t(
					"devices:service.diagnosis.healthy",
					"Safe update to v{{to}} finished at {{time}}",
					{ to, time: at },
				),
			};
		case "rolled_back":
			return {
				...base,
				state: "warn",
				label: t(
					"devices:service.diagnosis.rolledBack",
					"Update to v{{to}} rolled back",
					{ to },
				),
				note: rolloutOutcome(t, rollout),
			};
		case "not_applied":
			return {
				...base,
				state: "warn",
				label: t(
					"devices:service.diagnosis.notApplied",
					"Update to v{{to}} not applied",
					{ to },
				),
				note: rolloutOutcome(t, rollout),
			};
		case "failed_stopped":
			return {
				...base,
				state: "fail",
				label: t(
					"devices:service.diagnosis.failedStopped",
					"Update to v{{to}} failed and the rollback failed: service stopped",
					{ to },
				),
			};
		default:
			return undefined;
	}
}

function errorCheck(
	t: DevicesT,
	device: string,
	service: ServiceView,
	diagnostics: InstanceDiagnostics,
): ChecklistItem {
	const crashed =
		service.conv === "crash_looping" || service.conv === "failed_stopped";
	if (diagnostics !== "reported")
		return {
			id: "error",
			state: "pending",
			label: (
				<Sourced
					stamp={
						<FreshnessStamp
							source="device"
							age="notloaded"
							text={t(
								"devices:service.diagnosis.onDevice",
								"only on the device",
							)}
						/>
					}
				>
					{crashed
						? t(
								"devices:service.diagnosis.errorOnDevice",
								"Last error and restart count: only on the device",
							)
						: t(
								"devices:service.diagnosis.errorNotReported",
								"Last error per instance isn't reported",
							)}
				</Sourced>
			),
			note:
				diagnostics === "old_agent" ? (
					<Trans
						t={t}
						i18nKey="devices:service.diagnosis.errorCommandOld"
						defaults="Run <1>flow-like-standalone status</1> on {{device}} to read it, or update its agent to see it here."
						values={{ device }}
						components={{ 1: <span className="font-mono" /> }}
					/>
				) : (
					<Trans
						t={t}
						i18nKey="devices:service.diagnosis.errorCommand"
						defaults="Run <1>flow-like-standalone status</1> on {{device}} to read it. A live connection shows it here."
						values={{ device }}
						components={{ 1: <span className="font-mono" /> }}
					/>
				),
		};
	const facts = service.diagnostics;
	const restarts = facts?.restarts;
	const note = restarts?.failures
		? t("devices:service.diagnosis.restarts", {
				count: restarts.failures,
				max: restarts.max_restarts,
				defaultValue_one:
					"Restarted {{count, number}} time of {{max, number}} allowed.",
				defaultValue_other:
					"Restarted {{count, number}} times of {{max, number}} allowed.",
			})
		: undefined;
	if (facts?.lastError)
		return {
			id: "error",
			state: crashed ? "fail" : "warn",
			label: t(
				"devices:service.diagnosis.lastError",
				"Last error: “{{error}}”",
				{ error: facts.lastError },
			),
			note,
		};
	if (facts?.hasError)
		return {
			id: "error",
			state: crashed ? "fail" : "warn",
			label: t(
				"devices:service.diagnosis.errorHidden",
				"An error was recorded for an instance",
			),
			note: t(
				"devices:service.diagnosis.errorHiddenNote",
				"Its text shows over a live connection to people with Deploy & configure.",
			),
		};
	return {
		id: "error",
		state: "pass",
		label: t(
			"devices:service.diagnosis.noError",
			"No error recorded for its instances",
		),
		note,
	};
}

/** A line read from another source than the block's head carries its own stamp (R5). */
function Sourced({
	stamp,
	children,
}: Readonly<{ stamp: ReactNode; children: ReactNode }>) {
	return (
		<span className="flex flex-wrap items-start justify-between gap-x-3 gap-y-0.5">
			<span className="min-w-0">{children}</span>
			<span className="mt-px shrink-0">{stamp}</span>
		</span>
	);
}

/** What the hub says about the service's cloud access: revoked, none, or active with its spending. */
function cloudLine(
	t: DevicesT,
	time: AreaTime,
	serviceId: string,
	resources: DeviceResources,
	invalid: boolean,
): { state: CheckState; text: string } {
	const grants = resources.grants.filter(
		(row) => row.placement_id === serviceId,
	);
	const active = grants.find((row) => row.status === "active");
	if (invalid || (!active && grants.length))
		return {
			state: "fail",
			text: t(
				"devices:service.diagnosis.cloudRevoked",
				"Cloud access revoked: the service is still bound to it",
			),
		};
	if (!active)
		return {
			state: "pass",
			text: t(
				"devices:service.diagnosis.cloudNone",
				"Runs with local resources only · no cloud access approved",
			),
		};
	const billing = resources.billing.find(
		(row) => row.grant_id === active.grant_id && row.status === "active",
	);
	const until = time.at(active.effective_expires_at ?? active.expires_at);
	return {
		state: "pass",
		text: billing
			? t(
					"devices:service.diagnosis.cloudSpent",
					"Cloud access active until {{until}} · {{used}} of {{limit}} spent",
					{
						until,
						used: formatMoney(billing.used_micros, time.locale),
						limit: formatMoney(billing.limit_micros, time.locale),
					},
				)
			: t(
					"devices:service.diagnosis.cloudNoLimit",
					"Cloud access active until {{until}} · no spending limit",
					{ until },
				),
	};
}

function queueCheck(t: DevicesT, queues: QueueFacts): ChecklistItem {
	if (queues.needYou)
		return {
			id: "writes",
			state: "warn",
			label: t("devices:service.diagnosis.writesNeedYou", {
				count: queues.needYou,
				waiting: queues.waiting,
				defaultValue_one:
					"Write buffering: {{waiting, number}} changes waiting · {{count, number}} queue needs you",
				defaultValue_other:
					"Write buffering: {{waiting, number}} changes waiting · {{count, number}} queues need you",
			}),
		};
	return {
		id: "writes",
		state: "pass",
		label: queues.waiting
			? t("devices:service.diagnosis.writesWaiting", {
					count: queues.waiting,
					defaultValue_one: "Write buffering: {{count, number}} change waiting",
					defaultValue_other:
						"Write buffering: {{count, number}} changes waiting",
				})
			: t(
					"devices:service.diagnosis.writesUpToDate",
					"Write buffering up to date · 0 waiting",
				),
	};
}

function endpointCheck(
	t: DevicesT,
	endpoint: PlacementConfigFacts,
): ChecklistItem | undefined {
	if (!endpoint.host || !endpoint.port) return undefined;
	const address = `${endpoint.host}:${endpoint.port}`;
	if (endpoint.tlsCertificateId)
		return {
			id: "endpoint",
			state: "pass",
			label: t(
				"devices:service.diagnosis.endpointTls",
				"Endpoint answers encrypted with a certificate of this device",
			),
		};
	if (LOOPBACK.has(endpoint.host))
		return {
			id: "endpoint",
			state: "pass",
			label: (
				<Trans
					t={t}
					i18nKey="devices:service.diagnosis.endpointLoopback"
					defaults="Endpoint answers only on the device itself (<1>{{address}}</1>), so no certificate is needed"
					values={{ address }}
					components={{ 1: <span className="font-mono whitespace-nowrap" /> }}
				/>
			),
		};
	return {
		id: "endpoint",
		state: "warn",
		label: t(
			"devices:service.diagnosis.endpointOpen",
			"Endpoint is unencrypted and reachable from the network",
		),
	};
}

function CommandStep({
	command,
	icon,
	children,
}: Readonly<{
	command: ServiceCommand;
	icon: typeof Play;
	children: string;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const gate = command.gate.ok
		? null
		: {
				kind: command.gate.kind,
				reason: gateCopy(t, command.gate, time).inline,
			};
	return (
		<GatedAction gate={gate}>
			<DvButton
				size="sm"
				icon={icon}
				busy={command.pending}
				onClick={() => void command.run()}
			>
				{children}
			</DvButton>
		</GatedAction>
	);
}

const isCrashed = (service: ServiceView) =>
	service.conv === "crash_looping" || service.conv === "failed_stopped";

function present<T>(item: T | undefined): T[] {
	return item ? [item] : [];
}

function stateCheck(
	t: DevicesT,
	service: ServiceView,
	rollout: DeploymentRolloutStatus | undefined,
): ChecklistItem {
	const waitsStaged =
		rollout?.state === "staged" && service.observed === service.desired;
	const settled =
		service.conv === "converged" ||
		service.conv === "stopped_by_user" ||
		waitsStaged;
	const failing = isCrashed(service) ? "fail" : "active";
	return {
		id: "state",
		state: settled ? "pass" : failing,
		label: (
			<Trans
				t={t}
				i18nKey="devices:service.diagnosis.state"
				defaults="Requested <1>{{requested}}</1>, actual <2>{{actual}}</2> · {{ready}} of {{count}} ready"
				values={{
					requested: enumLabel(t, "desired", desiredRun(service)),
					actual: enumLabel(t, "observed", observedRun(service)),
					ready: service.instances.ready,
					count: service.instances.requested,
				}}
				components={{
					1: <b className="font-semibold" />,
					2: <b className="font-semibold" />,
				}}
			/>
		),
	};
}

/** The critical state inside the block: a crash loop, or an update that left the service stopped. */
function CrashBanner({
	device,
	service,
}: Readonly<{ device: DeviceViewModel; service: ServiceView }>) {
	const { t } = useTranslation("devices");
	if (service.conv === "failed_stopped")
		return (
			<Banner
				tone="critical"
				title={t(
					"service.diagnosis.failedTitle",
					"The update failed and the previous version couldn't be restored.",
				)}
			>
				{t(
					"service.diagnosis.failedText",
					"The service is stopped. Start runs settings v{{settings}} again; check the logs for the crash first.",
					{ settings: service.settings.latest },
				)}
			</Banner>
		);
	if (service.conv !== "crash_looping") return null;
	const offline =
		device.presence.kind === "online"
			? ""
			: t(
					"service.diagnosis.crashOffline",
					"Start needs {{device}} to be online.",
					{ device: deviceName(device.row) },
				);
	return (
		<Banner
			tone="critical"
			title={t(
				"service.diagnosis.crashTitle",
				"Crashing: an instance keeps restarting and is waiting before the next attempt.",
			)}
		>
			{t(
				"service.diagnosis.crashText",
				"The agent restarts crashed instances on its own, up to {{max}} times, then waits. Start clears the crash-loop limit.",
				{ max: service.diagnostics?.restarts?.max_restarts ?? 5 },
			)}{" "}
			{offline}
		</Banner>
	);
}

/** What to do next: commands through the action layer, the rest as links into the other tabs. */
function NextSteps({
	device,
	service,
	writesNeedYou,
}: Readonly<{
	device: DeviceViewModel;
	service: ServiceView;
	writesNeedYou: boolean;
}>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const overlay = useOverlay();
	const deviceId = device.row.device_id;
	const { serviceId } = service;
	const commands = useServiceCommands(deviceId, serviceId);
	const live = !isLastKnown(service.freshness);
	const crashed = isCrashed(service);
	const unsettled = service.conv === "converging" || service.conv === "unknown";
	const restartable = !crashed && service.desired === "running" && unsettled;
	const tabLink = (tab: "activity" | "cloud" | "offline", errors = false) =>
		link({
			screen: "service",
			deviceId,
			serviceId,
			tab,
			...(errors ? { stream: "errors" as const } : {}),
		});
	return (
		<div className="flex flex-wrap items-start gap-2">
			{live && device.presence.kind === "online" ? null : (
				<DvButton
					size="sm"
					icon={Stethoscope}
					onClick={() => overlay.openDiagnose(deviceId, serviceId)}
				>
					{t("service.diagnosis.diagnose", "Diagnose")}
				</DvButton>
			)}
			{live && crashed ? (
				<CommandStep command={commands.start} icon={Play}>
					{t("service.diagnosis.start", "Start")}
				</CommandStep>
			) : null}
			{live && restartable ? (
				<CommandStep command={commands.restart} icon={RotateCw}>
					{t("service.diagnosis.restart", "Restart…")}
				</CommandStep>
			) : null}
			{live ? (
				<DvButton asChild size="sm" icon={ScrollText}>
					<a {...tabLink("activity", true)}>
						{t("service.diagnosis.viewLogs", "View logs (errors only)")}
					</a>
				</DvButton>
			) : null}
			<DvButton asChild size="sm" icon={Cloud}>
				<a {...tabLink("cloud")}>
					{t("service.diagnosis.checkCloud", "Check cloud access")}
				</a>
			</DvButton>
			{writesNeedYou ? (
				<DvButton asChild size="sm" icon={Database}>
					<a {...tabLink("offline")}>
						{t("service.diagnosis.reviewWrites", "Review buffered writes")}
					</a>
				</DvButton>
			) : null}
		</div>
	);
}

/** SPEC §5.3 Status › Diagnosis: what is wrong, the evidence per source, and the next steps. */
export function DiagnosisBlock({
	device,
	service,
	rollout,
	diagnosis,
	diagnostics,
	lockedData = false,
}: Readonly<{
	device: DeviceViewModel;
	service: ServiceView;
	rollout: DeploymentRolloutStatus | undefined;
	diagnosis: ServiceDiagnosis;
	diagnostics: InstanceDiagnostics;
	lockedData?: boolean;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const resources = useDeviceResources(device.row.device_id);
	const name = deviceName(device.row);
	const cloud =
		resources.data &&
		cloudLine(
			t,
			time,
			service.serviceId,
			resources.data,
			diagnosis.cloudInvalid,
		);
	const items: ChecklistItem[] = [
		stateCheck(t, service, rollout),
		...present(rollout && rolloutCheck(t, time, service, rollout)),
		errorCheck(t, name, service, diagnostics),
		...present(
			cloud && {
				id: "cloud",
				state: cloud.state,
				label: (
					<Sourced stamp={<FreshnessStamp {...stampOf(resources.freshness)} />}>
						{cloud.text}
					</Sourced>
				),
			},
		),
		...present(diagnosis.queues && queueCheck(t, diagnosis.queues)),
		...present(diagnosis.endpoint && endpointCheck(t, diagnosis.endpoint)),
	];

	return (
		<Block
			icon={Stethoscope}
			title={t("service.diagnosis.title", "Diagnosis")}
			stamp={<FreshnessStamp {...stampOf(service.freshness)} />}
			className={lockedData ? LOCKED_DATA_CLASS : undefined}
			foot={t(
				"service.diagnosis.foot",
				"Lines without their own stamp come from the source in this block's head. Lines marked On-device only need a shell on the device.",
			)}
		>
			<CrashBanner device={device} service={service} />
			<Checklist
				items={items}
				label={t("service.diagnosis.evidence", "Evidence")}
			/>
			<NextSteps
				device={device}
				service={service}
				writesNeedYou={!!diagnosis.queues?.needYou}
			/>
		</Block>
	);
}
