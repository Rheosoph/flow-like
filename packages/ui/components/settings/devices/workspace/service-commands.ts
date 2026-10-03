"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import { buildServiceViews } from "../../../../lib/device-management/model/attention";
import {
	deviceName,
	rolloutEndsAt,
} from "../../../../lib/device-management/model/device-view";
import { evaluateGates } from "../../../../lib/device-management/model/gates";
import type {
	GateExtra,
	GateResult,
	PlacementStatusPlus,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import type { ManagementResponse } from "../../../../lib/device-management/types";
import type { DeviceWorkspace } from "../../../../lib/device-management/workspace/types";
import type { DevicesT } from "../primitives/area-context";
import type { ConfirmStrength } from "../primitives/confirm-sheet";
import type { ConsequenceRows } from "../primitives/consequence-preview";
import {
	type GateTarget,
	buildGateContext,
	useAttentionState,
} from "./use-attention";
import {
	type DeviceActionOutcome,
	type DeviceActions,
	useDeviceAction,
} from "./use-device-action";

export const SERVICE_COMMANDS = [
	"start",
	"stop",
	"restart",
	"scale",
	"remove_service",
] as const;
export type ServiceCommandId = (typeof SERVICE_COMMANDS)[number];

export interface ServiceCommand {
	action: ServiceCommandId;
	/** R7: the control stays visible; a failing gate disables it with its reason. */
	gate: GateResult;
	/** Verb + object: button, confirm button, tray and result ("Stop support-bot"). */
	label: string;
	/** "Stop support-bot?" */
	title: string;
	/** Where it runs ("on edge-berlin-01"). */
	sub: string;
	rows: ConsequenceRows;
	strength: ConfirmStrength;
	/** The text to type for `typed`. */
	typed?: string;
	tone: "danger" | "default";
	/** From the click until the device answered. */
	pending: boolean;
	/**
	 * Sends the command once. `confirmed` skips the sheet when the screen
	 * already showed these rows inline (only for strength `none`).
	 */
	run(options?: {
		confirmed?: boolean;
	}): Promise<DeviceActionOutcome<ManagementResponse>>;
}

export interface ServiceCommands {
	/** `undefined` while the service's row can't be read. */
	service: ServiceView | undefined;
	/** `useInlineResults(resultKey)` shows what these commands did. */
	resultKey: string;
	start: ServiceCommand;
	stop: ServiceCommand;
	restart: ServiceCommand;
	scale(replicas: number): ServiceCommand;
	remove: ServiceCommand;
}

export function serviceResultKey(deviceId: string, serviceId: string): string {
	return `service:${deviceId}/${serviceId}`;
}

export interface ServiceCommandOptions {
	/**
	 * Whether the service has schedules, from a screen that reads its app's
	 * events. Without it only what a running service reports is known.
	 */
	hasSchedules?: boolean;
	/** The same for bots. */
	hasBots?: boolean;
}

const ACTIVE_ROLLOUT = new Set([
	"staged",
	"validating",
	"activating",
	"rolling_back",
]);
const OPEN_HOST_OPERATION = new Set([
	"pending",
	"requesting",
	"requested",
	"draining",
	"staging",
]);

/** IA §3.3 "Other": what the service's row says about rollouts, state and instance limits. */
export function serviceGateExtra(
	service: ServiceView | undefined,
	hostOperationState?: string,
): GateExtra {
	if (!service)
		return hostOperationState === undefined
			? {}
			: { hostOperationActive: OPEN_HOST_OPERATION.has(hostOperationState) };
	const rollout = service.rollout;
	const endsAt = rollout ? rolloutEndsAt(rollout) : undefined;
	return {
		activeRollout:
			service.conv === "update_in_progress" ||
			(!!rollout && ACTIVE_ROLLOUT.has(rollout.state)),
		...(rollout ? { rolloutState: rollout.state } : {}),
		...(endsAt ? { rolloutDeadlineAt: endsAt } : {}),
		desiredState: service.desired,
		observedState: service.observed,
		maxReplicas: service.instances.max,
		...(hostOperationState === undefined
			? {}
			: { hostOperationActive: OPEN_HOST_OPERATION.has(hostOperationState) }),
	};
}

interface Copy {
	label: string;
	rows: ConsequenceRows;
	strength: ConfirmStrength;
	typed?: string;
	tone: "danger" | "default";
}

interface CopyInput {
	t: DevicesT;
	service: string;
	device: string;
	instances: number;
	settings: number;
	crashLooping: boolean;
	/** The service runs schedules: of an online app (the hub coordinates them) or of an offline copy; null when it has none that is known. */
	schedules: "online" | "offline" | null;
	/** The service runs bots, as far as is known. */
	bots: boolean;
	/** Interpolation values shared by the sentences. */
	names: { service: string; device: string; settings: number };
}

/** Sentences that follow another in one consequence row. */
const after = (first: string, ...next: (string | null)[]) =>
	[first, ...next.filter((line): line is string => line !== null)].join(" ");

/** Stopping a service disconnects its bots; what happens to their messages meanwhile. */
const stopBots = (c: CopyInput) =>
	c.bots
		? c.t(
				"devices:action.service.stop.bots",
				"Its bots disconnect. Telegram keeps messages for a day; after a start {{device}} answers those of the last 15 minutes. Discord messages sent meanwhile are not answered.",
				c.names,
			)
		: null;

const startBots = (c: CopyInput) =>
	c.bots
		? c.t("devices:action.service.start.bots", "Its bots connect again.")
		: null;

/** Stopping a service stops its schedules, and the hub does not take them over by itself. */
function stopSchedules(c: CopyInput): string | null {
	const { t } = c;
	if (c.schedules === "online")
		return t(
			"devices:action.service.stop.schedules",
			"Its schedules stop. A run in progress is cut off. The hub doesn't take them over. To run one on the hub meanwhile, choose Run it on the hub again in Events.",
		);
	return c.schedules === "offline"
		? t(
				"devices:action.service.stop.schedulesLocal",
				"Its schedules stop. A run in progress is cut off.",
			)
		: null;
}

const immediately = (t: DevicesT) =>
	t("devices:action.service.when.now", "Immediately.");

function startCopy(c: CopyInput): Copy {
	const { t } = c;
	const what = t("devices:action.service.start.what", {
		count: c.instances,
		settings: c.settings,
		defaultValue_one:
			"Starts {{count, number}} instance with settings v{{settings}}.",
		defaultValue_other:
			"Starts {{count, number}} instances with settings v{{settings}}.",
	});
	return {
		label: t(
			"devices:action.service.start.label",
			"Start {{service}}",
			c.names,
		),
		rows: {
			what: after(
				c.crashLooping
					? `${what} ${t("devices:action.service.start.clearsLimit", "This also clears the crash-loop limit.")}`
					: what,
				c.schedules
					? t(
							"devices:action.service.start.schedules",
							"Schedules run again from their next time. Missed runs are not made up.",
						)
					: null,
				startBots(c),
			),
			who: t(
				"devices:action.service.start.who",
				"Runs the app's code on {{device}}.",
				c.names,
			),
			when: immediately(t),
			undo: {
				reversible: true,
				text: t("devices:action.service.start.undo", "Stop it again."),
			},
		},
		strength: "none",
		tone: "default",
	};
}

function stopCopy(c: CopyInput): Copy {
	const { t } = c;
	return {
		label: t("devices:action.service.stop.label", "Stop {{service}}", c.names),
		rows: {
			what: t("devices:action.service.stop.what", {
				count: c.instances,
				defaultValue_one:
					"Its instance stops. The device keeps it stopped until someone starts it.",
				defaultValue_other:
					"Its {{count, number}} instances stop. The device keeps it stopped until someone starts it.",
			}),
			who: after(
				t(
					"devices:action.service.stop.who",
					"The service stops answering. Requests in flight are cut off.",
				),
				stopSchedules(c),
				stopBots(c),
			),
			stays: t(
				"devices:action.service.stop.stays",
				"Settings v{{settings}}, app data and certificates stay on the device.",
				c.names,
			),
			when: immediately(t),
			undo: {
				reversible: true,
				text: t(
					"devices:action.service.stop.undo",
					"Start runs the same settings again.",
				),
			},
		},
		strength: "none",
		tone: "danger",
	};
}

function restartCopy(c: CopyInput): Copy {
	const { t } = c;
	return {
		label: t(
			"devices:action.service.restart.label",
			"Restart {{service}}",
			c.names,
		),
		rows: {
			what: t(
				"devices:action.service.restart.what",
				"Each instance stops and starts again with settings v{{settings}}. Restarts don't count toward the crash-loop limit.",
				c.names,
			),
			who: t(
				"devices:action.service.restart.who",
				"Requests in flight are cut off; the service is unavailable for a few seconds.",
			),
			when: immediately(t),
			undo: {
				reversible: null,
				text: t(
					"devices:action.service.restart.undo",
					"Not needed: it comes back on its own.",
				),
			},
		},
		strength: "none",
		tone: "danger",
	};
}

function scaleCopy(c: CopyInput, replicas: number): Copy {
	const { t } = c;
	const down = replicas < c.instances;
	const change = Math.abs(replicas - c.instances);
	return {
		label: t("devices:action.service.scale.label", {
			count: replicas,
			service: c.service,
			defaultValue_one: "Run {{count, number}} instance of {{service}}",
			defaultValue_other: "Run {{count, number}} instances of {{service}}",
		}),
		rows: {
			what: down
				? t("devices:action.service.scale.whatDown", {
						count: change,
						to: replicas,
						defaultValue_one:
							"{{count, number}} instance stops. {{to, number}} keep running.",
						defaultValue_other:
							"{{count, number}} instances stop. {{to, number}} keep running.",
					})
				: t("devices:action.service.scale.whatUp", {
						count: change,
						settings: c.settings,
						defaultValue_one:
							"{{count, number}} more instance starts with settings v{{settings}}.",
						defaultValue_other:
							"{{count, number}} more instances start with settings v{{settings}}.",
					}),
			who: down
				? t(
						"devices:action.service.scale.whoDown",
						"Fewer requests are handled in parallel. Requests on the stopped instances are cut off.",
					)
				: t(
						"devices:action.service.scale.whoUp",
						"More of the app's code runs on {{device}}.",
						c.names,
					),
			when: immediately(t),
			undo: {
				reversible: true,
				text: down
					? t("devices:action.service.scale.undoDown", "Raise the count again.")
					: t("devices:action.service.scale.undoUp", "Lower the count again."),
			},
		},
		strength: "none",
		tone: down ? "danger" : "default",
	};
}

function removeCopy(c: CopyInput): Copy {
	const { t } = c;
	return {
		label: t(
			"devices:action.service.remove.label",
			"Remove {{service}}",
			c.names,
		),
		rows: {
			what: t(
				"devices:action.service.remove.what",
				"{{service}} is removed from {{device}}. Its ID is reserved forever.",
				c.names,
			),
			who: t(
				"devices:action.service.remove.who",
				"Its cloud access and spending limit stay active until you revoke them.",
			),
			stays: t(
				"devices:action.service.remove.stays",
				"Its data stays on the device in placement-data/{{service}}.",
				c.names,
			),
			when: immediately(t),
			undo: {
				reversible: false,
				text: t(
					"devices:action.service.remove.undo",
					"Deploy again under a new ID.",
				),
			},
			first: t(
				"devices:action.service.remove.first",
				"Stop the service before you remove it.",
			),
		},
		strength: "typed",
		typed: c.service,
		tone: "danger",
	};
}

type Expect = (placement: PlacementStatusPlus | undefined) => boolean;

const running: Expect = (placement) => placement?.observed_state === "running";

/** What the next inspection must show before a command counts as done (the device journal only says "accepted"). */
const EXPECT: Record<ServiceCommandId, (replicas: number) => Expect> = {
	start: () => running,
	restart: () => running,
	stop: () => (placement) => placement?.observed_state === "stopped",
	scale: (replicas) => (placement) =>
		!!placement &&
		(placement.desired_replicas ?? 1) === replicas &&
		(placement.running_replicas ?? replicas) === replicas,
	remove_service: () => (placement) => placement === undefined,
};

const WIRE: Record<ServiceCommandId, string> = {
	start: "start",
	stop: "stop",
	restart: "restart",
	scale: "scale",
	remove_service: "remove",
};

/** Resolves once an inspection read after `sinceS` shows what the command asked for. */
function settled(
	workspace: DeviceWorkspace,
	deviceId: string,
	serviceId: string,
	sinceS: number,
	expect: Expect,
): Promise<"done"> {
	return new Promise((resolve) => {
		const watch: { stop?: () => void; done?: boolean } = {};
		const check = () => {
			const inspection = workspace.live.inspection(deviceId);
			if (watch.done || !inspection || inspection.readAt < sinceS) return;
			const placement = inspection.value.placements.find(
				(row) => row.id === serviceId,
			);
			if (!expect(placement)) return;
			watch.done = true;
			watch.stop?.();
			resolve("done");
		};
		watch.stop = workspace.live.subscribe(check);
		check();
		if (watch.done) watch.stop();
	});
}

interface CommandInput {
	actions: DeviceActions;
	workspace: DeviceWorkspace;
	deviceId: string;
	serviceId: string;
	service: ServiceView | undefined;
	deviceLabel: string;
	target: GateTarget;
	resultKey: string;
	gates: Record<ServiceCommandId, GateResult>;
	t: DevicesT;
}

function command(
	input: CommandInput,
	action: ServiceCommandId,
	copy: Copy,
	replicas = 0,
): ServiceCommand {
	const { actions, workspace, deviceId, serviceId, service, t } = input;
	const wire = WIRE[action];
	return {
		action,
		gate: input.gates[action],
		label: copy.label,
		title: t("devices:action.confirm.title", "{{label}}?", {
			label: copy.label,
		}),
		sub: t("devices:action.service.sub", "on {{device}}", {
			device: input.deviceLabel,
		}),
		rows: copy.rows,
		strength: copy.strength,
		...(copy.typed ? { typed: copy.typed } : {}),
		tone: copy.tone,
		pending: actions.pending(input.resultKey),
		run: (options = {}) => {
			/** Hub-corrected seconds when the command left, like `LiveInspection.readAt`. */
			const sent = { atS: 0 };
			const skipSheet = options.confirmed === true && copy.strength === "none";
			return actions.run<ManagementResponse>({
				action,
				deviceId,
				target: input.target,
				label: copy.label,
				...(skipSheet ? {} : { consequence: copy.rows }),
				strength: copy.strength,
				confirm: {
					sub: t("devices:action.service.sub", "on {{device}}", {
						device: input.deviceLabel,
					}),
					tone: copy.tone,
					...(copy.typed ? { typed: copy.typed } : {}),
				},
				resultKey: input.resultKey,
				call: (context) => {
					sent.atS = Math.floor(workspace.clock.now() / 1000);
					return context.request({
						type: wire,
						placement_id: serviceId,
						expected_revision: service?.settings.latest ?? 0,
						...(action === "scale" ? { replicas } : {}),
					});
				},
				activity: {
					kind: "command",
					params: { command: wire },
					deviceName: input.deviceLabel,
					serviceId,
					...(service ? { projectId: service.projectId } : {}),
					href: { screen: "service", deviceId, serviceId, tab: "status" },
					resume: (response) => ({
						type: "operation",
						operationId: response.operation_id,
						command: wire,
						issuedAt: sent.atS,
					}),
					settle: () =>
						settled(
							workspace,
							deviceId,
							serviceId,
							sent.atS,
							EXPECT[action](replicas),
						),
				},
			});
		},
	};
}

/**
 * Start, stop, restart, change instances and remove for one service, shared
 * by N1, N2, N3 and App › Devices: gates, consequence rows (SPEC §6.5), one
 * command each, a tray item that finishes when the device shows the result,
 * and an inline result under `resultKey`.
 */
export function useServiceCommands(
	deviceId: string,
	serviceId: string,
	options: ServiceCommandOptions = {},
): ServiceCommands {
	const { t } = useTranslation("devices");
	const state = useAttentionState();
	const actions = useDeviceAction();
	const { workspace, input } = state;
	const { hasSchedules, hasBots } = options;

	return useMemo(() => {
		const row = input.devices.find((device) => device.device_id === deviceId);
		const services = buildServiceViews(deviceId, input);
		const service = Array.isArray(services)
			? services.find((view) => view.serviceId === serviceId)
			: undefined;
		const hostOperation =
			input.live[deviceId]?.inspection?.value.hostOperation?.state;
		const target: GateTarget = {
			placementId: serviceId,
			...(service ? { projectId: service.projectId } : {}),
			labels: { service: serviceId },
			extra: serviceGateExtra(service, hostOperation),
		};
		const gates = evaluateGates(
			SERVICE_COMMANDS,
			buildGateContext(state, deviceId, target),
		);
		const device = row ? deviceName(row) : deviceId;
		const settings = service?.settings.latest ?? 0;
		// A running service reports its schedules; a stopped one only the screen that reads its app knows about.
		const scheduled =
			hasSchedules ??
			(Array.isArray(service?.schedules) && service.schedules.length > 0);
		const bots =
			hasBots ?? (Array.isArray(service?.bots) && service.bots.length > 0);
		const copy: CopyInput = {
			t,
			service: serviceId,
			device,
			instances: service?.instances.requested ?? 1,
			settings,
			crashLooping: service?.conv === "crash_looping",
			schedules: scheduled
				? service?.source === "offline"
					? "offline"
					: "online"
				: null,
			bots,
			names: { service: serviceId, device, settings },
		};
		const base: CommandInput = {
			actions,
			workspace,
			deviceId,
			serviceId,
			service,
			deviceLabel: copy.device,
			target,
			resultKey: serviceResultKey(deviceId, serviceId),
			gates,
			t,
		};
		return {
			service,
			resultKey: base.resultKey,
			start: command(base, "start", startCopy(copy)),
			stop: command(base, "stop", stopCopy(copy)),
			restart: command(base, "restart", restartCopy(copy)),
			scale: (replicas) =>
				command(base, "scale", scaleCopy(copy, replicas), replicas),
			remove: command(base, "remove_service", removeCopy(copy)),
		};
	}, [
		t,
		state,
		actions,
		workspace,
		input,
		deviceId,
		serviceId,
		hasSchedules,
		hasBots,
	]);
}
