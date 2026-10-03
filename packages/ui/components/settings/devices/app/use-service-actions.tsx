"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
import { schedulesClaimedBy } from "../../../../lib/device-management/model/schedule-where";
import type { DeviceWorkspace } from "../../../../lib/device-management/workspace/types";
import {
	type HeldEvents,
	botCutOffText,
	splitHeld,
} from "../cloud/held-events";
import { useDeployRole } from "../deploy/use-deploy-reads";
import { useAreaTime } from "../primitives/area-context";
import { useConfirm } from "../primitives/confirm-sheet";
import type { ConsequenceRows } from "../primitives/consequence-preview";
import { CheckField } from "../primitives/form-fields";
import type { Gate } from "../primitives/gate-notice";
import type { Note } from "../service/config-parts";
import {
	FRESH_CHOICE,
	type RemovalFlow,
	type RemoveChoice,
	ScheduleGiveBack,
	giveBackLabel,
	removeService,
} from "../service/remove-service";
import {
	type GateTarget,
	type ServiceCommands,
	serviceGateExtra,
	serviceResultKey,
	useAppPlacements,
	useDeviceAction,
	useDeviceWorkspace,
	useGates,
	useScheduleMoves,
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
	/** The sentence of a step that didn't finish (stop, give back, revoke, remove). */
	note: Note | null;
	dismiss(): void;
}

/** The schedules and bots a service took off the hub, with their event ids. */
type Held = HeldEvents & { ids: string[] };

function RemoveOptions({
	approval,
	waiting,
	held,
	mayGiveBack,
	deviceLabel,
	onChange,
}: Readonly<{
	approval: AppApproval | null;
	waiting: number;
	held: Held | null;
	mayGiveBack: boolean;
	deviceLabel: string;
	onChange(patch: Partial<RemoveChoice>): void;
}>) {
	const { t } = useTranslation("devices");
	const [state, setState] = useState({ revoke: false, lose: false });
	const set = (patch: Partial<typeof state>) => {
		setState({ ...state, ...patch });
		onChange(patch);
	};
	if (!approval && !waiting && !held) return null;
	return (
		<div data-remove-options="" className="flex flex-col gap-2">
			{held ? (
				<ScheduleGiveBack
					id="app-remove-give-back"
					held={held}
					allowed={mayGiveBack}
					onGiveBack={(giveBack) => onChange({ giveBack })}
				/>
			) : null}
			{held?.bots ? (
				<span data-remove-bots="">
					{botCutOffText(t, {
						device: deviceLabel,
						bots: held.bots,
						date: undefined,
						removing: true,
					})}
				</span>
			) : null}
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
 * Remove service… (typed service id), with the sequence of the service page:
 * stop it when it runs, hand its schedules and bots back to the hub, revoke
 * its cloud access when asked, then remove it.
 */
export function useRemoveService(
	input: AwareInput,
	approval: AppApproval | null,
	commands: ServiceCommands,
): RemoveService {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const workspace = useDeviceWorkspace();
	const actions = useDeviceAction();
	const confirm = useConfirm();
	const gateText = useGateText();
	const whoNotices = useWhoNotices();
	const { view, row, deviceLabel } = input;
	const { deviceId, serviceId } = row;
	const appId = row.view.projectId;
	const target = useMemo(() => targetOf(row), [row]);
	const gates = useGates(["remove_service", "stop"], deviceId, target);
	const resultKey = serviceResultKey(deviceId, serviceId);
	const choice = useRef<RemoveChoice>({ ...FRESH_CHOICE });
	const [busy, setBusy] = useState(false);
	const [note, setNote] = useState<Note | null>(null);
	const mounted = useRef(true);
	useEffect(() => {
		mounted.current = true;
		return () => {
			mounted.current = false;
		};
	}, []);
	const placements = useAppPlacements(appId);
	const moves = useScheduleMoves(appId ?? "");
	const { canEditEvents } = useDeployRole(appId);
	const held = useMemo<Held | null>(() => {
		const ids = schedulesClaimedBy(
			placements.data?.schedules,
			deviceId,
			serviceId,
		);
		if (!ids.length) return null;
		const known = new Map(
			[...view.events.rows, ...view.events.ineligible].map((event) => [
				event.eventId,
				{ name: event.name, eventType: event.eventType },
			]),
		);
		return { ids, ...splitHeld(ids, known, time.locale) };
	}, [placements.data, view, deviceId, serviceId, time.locale]);
	const mayGiveBack = canEditEvents !== false;
	const running = row.view.desired !== "stopped";
	const waiting = row.mode === "online" ? waitingWrites(row) : 0;
	// A running service is stopped first, so "stop it first" gates only when Stop itself can't run.
	const stopFirst =
		!gates.remove_service.ok &&
		gates.remove_service.copy.code === "service_must_be_stopped";
	const failing = stopFirst ? gates.stop : gates.remove_service;
	const gate = failing.ok ? null : gateText(failing);

	const run = useCallback(() => {
		choice.current = { ...FRESH_CHOICE };
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
					held={held}
					mayGiveBack={mayGiveBack}
					deviceLabel={deviceLabel}
					onChange={(patch) => {
						choice.current = { ...choice.current, ...patch };
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
			const say = (next: Note | null) => {
				if (mounted.current) setNote(next);
			};
			say(null);
			setBusy(true);
			const flow: RemovalFlow = {
				t,
				time,
				actions,
				workspace,
				commands,
				deviceId,
				serviceId,
				deviceLabel,
				projectId: appId,
				resultKey,
				cloud: {
					state: approval ? "listed" : "unknown",
					approvals: approval ? [approval.grantId] : [],
					limits: approval?.billing?.payerIsMe ? [approval.billing.id] : [],
				},
				stopFirst: stopFirst && gates.stop.ok,
				schedules:
					held && mayGiveBack
						? {
								ids: held.ids,
								giveBack: moves.giveBack,
								...(held.bots ? { box: giveBackLabel(t, held) } : {}),
							}
						: null,
				say,
			};
			try {
				if (await removeService(flow, choice.current))
					void workspace.live.refreshInspection(deviceId);
			} catch {
				say({
					tone: "critical",
					text: t(
						"serviceConfig.remove.interrupted",
						"Removing {{service}} was interrupted before {{device}} confirmed it. Check its status, then try again.",
						{ service: serviceId, device: deviceLabel },
					),
				});
			} finally {
				if (mounted.current) setBusy(false);
			}
		});
	}, [
		actions,
		confirm,
		whoNotices,
		workspace,
		commands,
		moves,
		time,
		input,
		approval,
		held,
		mayGiveBack,
		appId,
		stopFirst,
		gates.stop.ok,
		row,
		deviceId,
		serviceId,
		deviceLabel,
		resultKey,
		running,
		waiting,
		t,
	]);

	return {
		run,
		gate,
		pending: busy || actions.pending(resultKey),
		note,
		dismiss: () => setNote(null),
	};
}
