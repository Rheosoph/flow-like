"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useMemo, useRef, useState } from "react";
import {
	type DeploymentRolloutStatus,
	cancelDeploymentRollout,
	readDeploymentRollout,
} from "../../../../lib/device-management/deployment";
import type {
	AppServiceRow,
	AppVersionView,
	AppView,
} from "../../../../lib/device-management/model/app-plan";
import type { DeviceWorkspace } from "../../../../lib/device-management/workspace/types";
import { revokeDeviceGrant } from "../../../../lib/device-resources";
import { useConfirm } from "../primitives/confirm-sheet";
import type { ConsequenceRows } from "../primitives/consequence-preview";
import { CheckField } from "../primitives/form-fields";
import type { Gate } from "../primitives/gate-notice";
import {
	type GateTarget,
	serviceGateExtra,
	serviceResultKey,
	useDeviceAction,
	useDeviceWorkspace,
	useGates,
} from "../workspace";
import { useGateText } from "./app-shared";
import { type AppApproval, runsElsewhere, versionName } from "./app-view-local";

function recordRollout(
	workspace: DeviceWorkspace,
	deviceId: string,
	status: DeploymentRolloutStatus,
) {
	const known = workspace.facts.get(deviceId)?.rollouts ?? [];
	workspace.facts.record(deviceId, {
		rollouts: [
			...known.filter((rollout) => rollout.rollout_id !== status.rollout_id),
			status,
		],
	});
}

const targetOf = (row: AppServiceRow): GateTarget => ({
	placementId: row.serviceId,
	projectId: row.view.projectId,
	labels: { service: row.serviceId },
	extra: serviceGateExtra(row.view),
});

const STOP_POLLS = 40;
const STOP_POLL_MS = 500;
const STOPPED: readonly string[] = ["stopped", "failed"];

/**
 * A stop command only records the intent, and the device removes a service
 * only once it reports it stopped: wait for that before sending the removal.
 * After 20 s the removal is sent anyway and the device's answer is the result.
 */
async function stoppedOnDevice(
	workspace: DeviceWorkspace,
	deviceId: string,
	serviceId: string,
): Promise<void> {
	const sinceS = Math.floor(workspace.clock.now() / 1000);
	for (let attempt = 0; attempt < STOP_POLLS; attempt++) {
		await workspace.live.refreshInspection(deviceId).catch(() => undefined);
		const inspection = workspace.live.inspection(deviceId);
		// Only a status read after the stop counts: an older one still shows it running, or stopped by chance.
		if (inspection && inspection.readAt >= sinceS) {
			const status = inspection.value.placements.find(
				(entry) => entry.id === serviceId,
			);
			if (
				!status ||
				(STOPPED.includes(status.observed_state) && !status.running_replicas)
			)
				return;
		}
		await new Promise((resolve) => setTimeout(resolve, STOP_POLL_MS));
	}
}

export interface StagedActions {
	activate(): void;
	discard(): void;
	activateGate: Gate | null;
	discardGate: Gate | null;
	resultKey: string;
}

/** Activate… and Discard… of a staged update, with the app-aware rows of APP §2.15. */
export function useStagedActions(
	row: AppServiceRow,
	deviceLabel: string,
	staged: AppVersionView | null,
): StagedActions {
	const { t } = useTranslation("devices");
	const workspace = useDeviceWorkspace();
	const actions = useDeviceAction();
	const gateText = useGateText();
	const { deviceId, serviceId } = row;
	const target = useMemo(() => targetOf(row), [row]);
	const gates = useGates(
		["activate_staged", "discard_staged"],
		deviceId,
		target,
	);
	const rollout = row.view.rollout;
	const resultKey = serviceResultKey(deviceId, serviceId);
	const from = row.version ? versionName(row.version) : null;
	const to = staged ? versionName(staged) : null;
	const sub = t("app.action.on", "on {{device}}", { device: deviceLabel });

	const activate = useCallback(() => {
		if (!rollout) return;
		const scope = {
			rollout_id: rollout.rollout_id,
			placement_id: serviceId,
			project_id: row.view.projectId,
		};
		void actions.run({
			action: "activate_staged",
			deviceId,
			target,
			label: t(
				"app.action.activate.label",
				"Activate the update of {{service}}",
				{
					service: serviceId,
				},
			),
			consequence: {
				what:
					from && to
						? t(
								"app.action.activate.what",
								"{{service}} switches from {{from}} to {{to}} with a safe update.",
								{ service: serviceId, from, to },
							)
						: t(
								"app.action.activate.whatUnknown",
								"{{service}} switches to the staged version with a safe update.",
								{ service: serviceId },
							),
				who: t(
					"app.action.activate.who",
					"The current version keeps serving until the new one is healthy.",
				),
				when: t(
					"app.action.activate.when",
					"Now; the device checks it for up to 2 min.",
				),
				undo: {
					reversible: true,
					text: from
						? t(
								"app.action.activate.undo",
								"If it isn't healthy, the device restores {{from}} on its own.",
								{ from },
							)
						: t(
								"app.action.activate.undoUnknown",
								"If it isn't healthy, the device restores the current version on its own.",
							),
				},
			},
			strength: "none",
			confirm: { sub },
			resultKey,
			call: async (context) => {
				await context.request({
					type: "activate_rollout",
					rollout_id: scope.rollout_id,
				});
				const status = await readDeploymentRollout(context.call, scope).catch(
					(): DeploymentRolloutStatus => ({ ...rollout, state: "validating" }),
				);
				recordRollout(workspace, deviceId, status);
				return status;
			},
			activity: {
				kind: "safe_update",
				deviceName: deviceLabel,
				serviceId,
				projectId: row.view.projectId,
				href: { screen: "service", deviceId, serviceId, tab: "status" },
				resume: () => ({
					type: "rollout",
					rolloutId: scope.rollout_id,
					placementId: serviceId,
					projectId: row.view.projectId,
				}),
			},
		});
	}, [
		actions,
		workspace,
		rollout,
		deviceId,
		serviceId,
		row.view.projectId,
		target,
		resultKey,
		deviceLabel,
		from,
		to,
		sub,
		t,
	]);

	const discard = useCallback(() => {
		if (!rollout) return;
		const scope = {
			rollout_id: rollout.rollout_id,
			placement_id: serviceId,
			project_id: row.view.projectId,
		};
		void actions.run({
			action: "discard_staged",
			deviceId,
			target,
			label: t(
				"app.action.discard.label",
				"Discard the update of {{service}}",
				{
					service: serviceId,
				},
			),
			consequence: {
				what: t(
					"app.action.discard.what",
					"The staged version is discarded. The current version keeps running.",
				),
				who: t(
					"app.action.discard.who",
					"Nobody: {{service}} keeps answering as it does now.",
					{ service: serviceId },
				),
				when: t("app.action.discard.when", "Immediately."),
				undo: {
					reversible: true,
					text: t(
						"app.action.discard.undo",
						"Update {{service}} again from this page.",
						{ service: serviceId },
					),
				},
			},
			strength: "none",
			confirm: { sub, tone: "danger" },
			resultKey,
			call: async (context) => {
				const status = await cancelDeploymentRollout(context.call, scope);
				recordRollout(workspace, deviceId, status);
				return status;
			},
		});
	}, [
		actions,
		workspace,
		rollout,
		deviceId,
		serviceId,
		row.view.projectId,
		target,
		resultKey,
		sub,
		t,
	]);

	return {
		activate,
		discard,
		activateGate: gates.activate_staged.ok
			? null
			: gateText(gates.activate_staged),
		discardGate: gates.discard_staged.ok
			? null
			: gateText(gates.discard_staged),
		resultKey,
	};
}

/* App-aware consequence rows (APP §2.15). */

export interface AwareInput {
	view: AppView;
	row: AppServiceRow;
	deviceLabel: string;
	deviceName(deviceId: string): string;
	/** Names of the events the service serves; null when the status has no event list. */
	events: string[] | null;
	/** "127.0.0.1:8081" when known. */
	address: string | null;
}

/** Who notices when the service stops answering: its events, and where else the app keeps running. */
export function useWhoNotices(): (input: AwareInput) => string {
	const { t, i18n } = useTranslation("devices");
	return useCallback(
		(input) => {
			const { view, row } = input;
			const list = new Intl.ListFormat(i18n?.language ?? "en", {
				type: "conjunction",
			});
			const what = input.events?.length
				? list.format(input.events)
				: row.serviceId;
			const stops = input.address
				? t(
						"app.action.who.stopsAt",
						"{{what}} at {{address}} stops answering on {{device}}.",
						{ what, address: input.address, device: input.deviceLabel },
					)
				: t("app.action.who.stops", "{{what}} stops on {{device}}.", {
						what,
						device: input.deviceLabel,
					});
			const elsewhere = runsElsewhere(view, row).map((other) =>
				t("app.action.who.serviceOn", "{{service}} on {{device}}", {
					service: other.serviceId,
					device: input.deviceName(other.deviceId),
				}),
			);
			const rest = elsewhere.length
				? t(
						"app.action.who.elsewhere",
						"{{app}} keeps running elsewhere: {{services}}.",
						{ app: view.app.name, services: list.format(elsewhere) },
					)
				: t(
						"app.action.who.nowhere",
						"{{app}} then runs nowhere you can see.",
						{ app: view.app.name },
					);
			return `${stops} ${rest}`;
		},
		[t, i18n?.language],
	);
}

const waitingWrites = (row: AppServiceRow) =>
	typeof row.writes === "object" && row.writes ? row.writes.pending : 0;

/** What stays when a service stops (APP §2.15 Stop row). */
export function useWhatStays(): (row: AppServiceRow) => string {
	const { t } = useTranslation("devices");
	return useCallback(
		(row) =>
			row.mode === "online"
				? t("app.action.stays.online", {
						settings: row.view.settings.latest,
						count: waitingWrites(row),
						defaultValue_one:
							"Settings v{{settings}}, its cloud access and buffered writes stay. Data stays in the cloud; {{count, number}} change is waiting.",
						defaultValue_other:
							"Settings v{{settings}}, its cloud access and buffered writes stay. Data stays in the cloud; {{count, number}} changes are waiting.",
					})
				: t(
						"app.action.stays.offline",
						"Settings v{{settings}} and the data on the device stay.",
						{ settings: row.view.settings.latest },
					),
		[t],
	);
}

export interface RemoveService {
	run(): void;
	gate: Gate | null;
	pending: boolean;
}

function RemoveOptions({
	approval,
	waiting,
	onChange,
}: Readonly<{
	approval: AppApproval | null;
	waiting: number;
	onChange(next: { revoke: boolean; lose: boolean }): void;
}>) {
	const { t } = useTranslation("devices");
	const [state, setState] = useState({ revoke: false, lose: false });
	const set = (patch: Partial<typeof state>) => {
		const next = { ...state, ...patch };
		setState(next);
		onChange(next);
	};
	if (!approval && !waiting) return null;
	return (
		<div data-remove-options="" className="flex flex-col gap-2">
			{approval ? (
				<CheckField
					id="app-remove-revoke"
					checked={state.revoke}
					onCheckedChange={(revoke) => set({ revoke })}
				>
					{t(
						"app.action.remove.revoke",
						"Also revoke its cloud access and spending limit",
					)}
				</CheckField>
			) : null}
			{waiting ? (
				<CheckField
					id="app-remove-lose"
					checked={state.lose}
					onCheckedChange={(lose) => set({ lose })}
				>
					{t("app.action.remove.lose", "Remove anyway and lose them")}
				</CheckField>
			) : null}
		</div>
	);
}

/**
 * Remove service… (typed service id): stops it first when it runs, and
 * revokes its cloud access afterwards when asked.
 */
export function useRemoveService(
	input: AwareInput,
	approval: AppApproval | null,
): RemoveService {
	const { t } = useTranslation("devices");
	const actions = useDeviceAction();
	const confirm = useConfirm();
	const gateText = useGateText();
	const whoNotices = useWhoNotices();
	const { row, deviceLabel } = input;
	const { deviceId, serviceId } = row;
	const target = useMemo(() => targetOf(row), [row]);
	const gates = useGates(["remove_service", "stop"], deviceId, target);
	const resultKey = serviceResultKey(deviceId, serviceId);
	const choice = useRef({ revoke: false, lose: false });
	const running = row.view.desired !== "stopped";
	const waiting = row.mode === "online" ? waitingWrites(row) : 0;
	// A running service is stopped first, so "stop it first" gates only when Stop itself can't run.
	const stopFirst =
		!gates.remove_service.ok &&
		gates.remove_service.copy.code === "service_must_be_stopped";
	const failing = stopFirst ? gates.stop : gates.remove_service;
	const gate = failing.ok ? null : gateText(failing);

	const run = useCallback(() => {
		choice.current = { revoke: false, lose: false };
		const label = running
			? t("app.action.remove.stopAndRemove", "Stop and remove {{service}}", {
					service: serviceId,
				})
			: t("app.action.remove.label", "Remove {{service}}", {
					service: serviceId,
				});
		const rows: ConsequenceRows = {
			what: t(
				"app.action.remove.what",
				"{{service}} is removed from {{device}}. Its ID can't be used again on this device.",
				{ service: serviceId, device: deviceLabel },
			),
			who: `${whoNotices(input)} ${t(
				"app.action.remove.whoToken",
				"Clients using its access token get no answer.",
			)}`,
			stays:
				row.mode === "online"
					? t(
							"app.action.remove.staysOnline",
							"Data stays in the cloud. Cloud access stays unless you revoke it below.",
						)
					: t(
							"app.action.remove.staysOffline",
							"Its data stays on the device in placement-data/{{service}}; a new service starts from a fresh copy instead.",
							{ service: serviceId },
						),
			when: t("app.action.remove.when", "Immediately."),
			undo: {
				reversible: false,
				text: t("app.action.remove.undo", "Deploy again under a new ID."),
			},
			...(running || waiting
				? {
						first: [
							running
								? t(
										"app.action.remove.firstStop",
										"It is running: it is stopped first, then removed.",
									)
								: "",
							waiting
								? t("app.action.remove.firstWrites", {
										count: waiting,
										defaultValue_one:
											"{{count, number}} buffered change hasn't reached the cloud. Removing discards it.",
										defaultValue_other:
											"{{count, number}} buffered changes haven't reached the cloud. Removing discards them.",
									})
								: "",
						]
							.filter(Boolean)
							.join(" "),
					}
				: {}),
		};
		void confirm({
			title: t("app.action.remove.title", "Remove {{service}}?", {
				service: serviceId,
			}),
			sub: t("app.action.on", "on {{device}}", { device: deviceLabel }),
			rows,
			strength: "typed",
			typed: serviceId,
			confirmLabel: label,
			tone: "danger",
			extra: (
				<RemoveOptions
					approval={approval}
					waiting={waiting}
					onChange={(next) => {
						choice.current = next;
					}}
				/>
			),
			onConfirm: () => {
				if (waiting && !choice.current.lose)
					throw new Error(
						t(
							"app.action.remove.needLose",
							"Tick “Remove anyway and lose them” to remove a service with waiting changes.",
						),
					);
			},
		}).then(async (result) => {
			if (!result.ok) return;
			const { revoke } = choice.current;
			const outcome = await actions.run({
				action: "remove_service",
				deviceId,
				target: {
					...target,
					extra: { ...target.extra, desiredState: "stopped" },
				},
				label,
				resultKey,
				call: async (context) => {
					const expected = row.view.settings.latest;
					if (running)
						await context.request({
							type: "stop",
							placement_id: serviceId,
							expected_revision: expected,
						});
					await stoppedOnDevice(context.workspace, deviceId, serviceId);
					return context.request({
						type: "remove",
						placement_id: serviceId,
						expected_revision: expected,
					});
				},
				activity: {
					kind: "command",
					params: { command: "remove" },
					deviceName: deviceLabel,
					serviceId,
					projectId: row.view.projectId,
					href: { screen: "device", deviceId, tab: "services" },
				},
			});
			if (outcome.status !== "done" || !revoke || !approval) return;
			if (approval.billing?.payerIsMe)
				await actions.run({
					action: "spending_limit_revoke",
					deviceId,
					target: { placementId: serviceId, projectId: row.view.projectId },
					label: t(
						"app.spending.revokeLimitLabel",
						"Revoke the spending limit of {{service}}",
						{ service: serviceId },
					),
					resultKey,
					call: ({ workspace }) =>
						revokeDeviceGrant(
							workspace.hub.api,
							workspace.hub.profile,
							deviceId,
							"billing",
							approval.billing?.id ?? "",
						),
				});
			await actions.run({
				action: "cloud_access_revoke",
				deviceId,
				target: { placementId: serviceId, projectId: row.view.projectId },
				label: t(
					"app.spending.revokeApprovalLabel",
					"Revoke the cloud access of {{service}}",
					{ service: serviceId },
				),
				resultKey,
				call: ({ workspace }) =>
					revokeDeviceGrant(
						workspace.hub.api,
						workspace.hub.profile,
						deviceId,
						"resource",
						approval.grantId,
					),
			});
		});
	}, [
		actions,
		confirm,
		whoNotices,
		input,
		approval,
		row,
		deviceId,
		serviceId,
		deviceLabel,
		target,
		resultKey,
		running,
		waiting,
		t,
	]);

	return { run, gate, pending: actions.pending(resultKey) };
}
