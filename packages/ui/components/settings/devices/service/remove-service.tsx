"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { Trash2 } from "lucide-react";
import {
	type ReactNode,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import { deviceKeys } from "../../../../lib/device-management/hub/queries";
import type { GateResult } from "../../../../lib/device-management/model/types";
import type { ManagementResponse } from "../../../../lib/device-management/types";
import type { DeviceWorkspace } from "../../../../lib/device-management/workspace/types";
import { revokeDeviceGrant } from "../../../../lib/device-resources";
import { gateCopy } from "../copy/gate-copy";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import { Block } from "../primitives/block";
import { useConfirm } from "../primitives/confirm-sheet";
import type { ConsequenceRows } from "../primitives/consequence-preview";
import { DvButton } from "../primitives/dv-button";
import { CheckField } from "../primitives/form-fields";
import { type Gate, GateInline } from "../primitives/gate-notice";
import { useDevicesRoute } from "../routing/use-devices-route";
import {
	type DeviceActionOutcome,
	type DeviceActions,
	type GateTarget,
	type ServiceCommands,
	useAttentionState,
	useDeviceAction,
	useDeviceResources,
	useGates,
	useServiceCommands,
} from "../workspace";
import { hostingOf } from "./config-model";
import { ActionResults, Mono, type Note, gateLine } from "./config-parts";
import {
	type ServiceCloudAccess,
	readyToRemove,
	serviceCloudAccess,
	waitingChanges,
} from "./remove-model";
import type { ServiceConfigRead } from "./use-service-config";

/* Danger zone of N3 › Configuration (SPEC §5.3, §6.5): remove a service, stopping it first and ending its cloud access when asked. */

/** The device gives a service 15 s to stop and 2 s more before it ends it by force. */
const STOP_POLLS = 30;
const STOP_POLL_MS = 1_000;

interface RemoveChoice {
	revoke: boolean;
	lose: boolean;
}

/* The confirm's rows. */

/** "Cloud access stays unless you revoke it." with the box that ends it too. */
function RevokeOffer({
	serviceId,
	cloud,
	gate,
	onRevoke,
}: Readonly<{
	serviceId: string;
	cloud: ServiceCloudAccess;
	/** Why the viewer can't revoke right now; the box stays, disabled (R7). */
	gate: ReactNode;
	onRevoke(revoke: boolean): void;
}>) {
	const { t } = useTranslation("devices");
	const [revoke, setRevoke] = useState(false);
	const service = { service: serviceId };
	const both = cloud.limits.length
		? t(
				"serviceConfig.remove.revokeBoth",
				"Also revoke {{service}}'s cloud access and spending limit",
				service,
			)
		: t(
				"serviceConfig.remove.revokeAccess",
				"Also revoke {{service}}'s cloud access",
				service,
			);
	return (
		<>
			{t(
				"serviceConfig.remove.cloudStays",
				"Cloud access stays unless you revoke it.",
			)}
			<div className="mt-1.5">
				<CheckField
					id="svc-remove-revoke"
					checked={revoke}
					disabled={!!gate}
					onCheckedChange={(next) => {
						setRevoke(next);
						onRevoke(next);
					}}
				>
					{cloud.approvals.length
						? both
						: t(
								"serviceConfig.remove.revokeLimit",
								"Also revoke the spending limit you pay for {{service}}",
								service,
							)}
				</CheckField>
			</div>
			{gate ? (
				<span className="mt-1 block text-xs text-muted-foreground">{gate}</span>
			) : null}
		</>
	);
}

/** What happens to the service's cloud access when the service goes. */
function CloudAccessLine({
	serviceId,
	cloud,
	gate,
	hosted,
	unknown,
	onRevoke,
}: Readonly<{
	serviceId: string;
	cloud: ServiceCloudAccess;
	gate: ReactNode;
	/** Another sentence stands before this one. */
	hosted: boolean;
	/** The sentence for when the hub's list wasn't read. */
	unknown: ReactNode;
	onRevoke(revoke: boolean): void;
}>) {
	const { t } = useTranslation("devices");
	if (cloud.state === "unknown") return <>{unknown}</>;
	if (cloud.state === "none")
		return hosted
			? t("serviceConfig.remove.cloudNoneHosted", "It has no cloud access.")
			: t(
					"serviceConfig.remove.cloudNone",
					"Nothing else: it has no cloud access.",
				);
	if (cloud.state === "hidden")
		return t(
			"serviceConfig.remove.cloudHidden",
			"If someone approved cloud access for it, that stays until they or the device owner revoke it.",
		);
	if (!cloud.approvals.length && !cloud.limits.length)
		return t(
			"serviceConfig.remove.cloudForeign",
			"Cloud access stays. Only the device owner or the person who approved it can revoke it.",
		);
	return (
		<RevokeOffer
			serviceId={serviceId}
			cloud={cloud}
			gate={gate}
			onRevoke={onRevoke}
		/>
	);
}

/** What has to happen before the removal: the stop, and buffered changes that would be lost. */
function BeforeRemoval({
	stopFirst,
	waiting,
	onLose,
}: Readonly<{
	stopFirst: boolean;
	waiting: number | "unknown";
	onLose(lose: boolean): void;
}>) {
	const { t } = useTranslation("devices");
	const [lose, setLose] = useState(false);
	const stop = stopFirst
		? t(
				"serviceConfig.remove.firstStop",
				"It's running. Confirming stops it first (Stop and remove).",
			)
		: null;
	if (!waiting) return <>{stop}</>;
	if (waiting === "unknown")
		return (
			<>
				{stop}
				{stop ? " " : null}
				{t(
					"serviceConfig.remove.firstWaitingUnknown",
					"If buffered changes haven't reached the cloud yet, removing discards them. Write buffering shows how many wait.",
				)}
			</>
		);
	return (
		<>
			{stop}
			{stop ? " " : null}
			{t("serviceConfig.remove.firstWaiting", {
				count: waiting,
				defaultValue_one:
					"{{count, number}} buffered change hasn't reached the cloud. Removing discards it.",
				defaultValue_other:
					"{{count, number}} buffered changes haven't reached the cloud. Removing discards them.",
			})}
			<div className="mt-1.5">
				<CheckField
					id="svc-remove-lose"
					checked={lose}
					onCheckedChange={(next) => {
						setLose(next);
						onLose(next);
					}}
				>
					{t("serviceConfig.remove.lose", {
						count: waiting,
						defaultValue_one: "Remove anyway and lose it",
						defaultValue_other: "Remove anyway and lose them",
					})}
				</CheckField>
			</div>
		</>
	);
}

/* The sequence: stop and wait, revoke, remove. Every step that doesn't finish leaves its sentence in the zone. */

interface Flow {
	t: DevicesT;
	time: AreaTime;
	actions: DeviceActions;
	workspace: DeviceWorkspace;
	commands: ServiceCommands;
	deviceId: string;
	serviceId: string;
	deviceLabel: string;
	projectId: string | undefined;
	resultKey: string;
	cloud: ServiceCloudAccess;
	/** The service runs and this person may stop it. */
	stopFirst: boolean;
	say(note: Note | null): void;
}

const namesOf = (flow: Flow) => ({
	service: flow.serviceId,
	device: flow.deviceLabel,
});

function targetOf(flow: Flow): GateTarget {
	return {
		placementId: flow.serviceId,
		...(flow.projectId ? { projectId: flow.projectId } : {}),
		labels: { service: flow.serviceId },
	};
}

/** The lists hold only what this account may end, so the gates check the hub side alone. */
const REVOCABLE = { delegatorIsMe: true, payerIsMe: true };

function sendCommand(flow: Flow, command: "stop" | "remove") {
	const { deviceId, serviceId, commands, projectId } = flow;
	const stop = command === "stop";
	return flow.actions.run<ManagementResponse>({
		action: stop ? "stop" : "remove_service",
		deviceId,
		target: {
			...targetOf(flow),
			...(stop ? {} : { extra: { desiredState: "stopped" } }),
		},
		label: stop ? commands.stop.label : commands.remove.label,
		resultKey: flow.resultKey,
		call: (context) =>
			context.request({
				type: command,
				placement_id: serviceId,
				expected_revision: commands.service?.settings.latest ?? 0,
			}),
		activity: {
			kind: "command",
			params: { command },
			deviceName: flow.deviceLabel,
			serviceId,
			...(projectId ? { projectId } : {}),
			href: stop
				? { screen: "service", deviceId, serviceId, tab: "status" }
				: { screen: "device", deviceId, tab: "services" },
		},
	});
}

function revokeGrant(flow: Flow, kind: "resource" | "billing", id: string) {
	const { t, deviceId, projectId } = flow;
	const scope = flow.workspace.scopeKey;
	return flow.actions.run<void>({
		action:
			kind === "resource" ? "cloud_access_revoke" : "spending_limit_revoke",
		deviceId,
		target: { ...targetOf(flow), extra: REVOCABLE },
		label:
			kind === "resource"
				? t(
						"devices:serviceConfig.remove.revokeAccessLabel",
						"Revoke the cloud access of {{service}}",
						namesOf(flow),
					)
				: t(
						"devices:serviceConfig.remove.revokeLimitLabel",
						"Revoke the spending limit of {{service}}",
						namesOf(flow),
					),
		resultKey: flow.resultKey,
		invalidate: [
			deviceKeys.resources(scope, deviceId),
			deviceKeys.resourceSummary(scope),
			...(projectId ? [deviceKeys.appPlacements(scope, projectId)] : []),
		],
		call: ({ workspace }) =>
			revokeDeviceGrant(
				workspace.hub.api,
				workspace.hub.profile,
				deviceId,
				kind,
				id,
			),
	});
}

/** A failing gate sends nothing and leaves no result line, so its reason goes in front of what it means here. */
function explain(
	flow: Flow,
	outcome: DeviceActionOutcome<unknown>,
	then: string,
) {
	const reason =
		outcome.status === "gated"
			? gateCopy(flow.t, outcome.gate, flow.time).inline
			: null;
	flow.say({ tone: "warning", text: reason ? `${reason} ${then}` : then });
}

async function waitStopped(flow: Flow, sinceS: number) {
	const { workspace, deviceId, serviceId } = flow;
	for (let attempt = 0; attempt < STOP_POLLS; attempt++) {
		await workspace.live.refreshInspection(deviceId);
		const inspection = workspace.live.inspection(deviceId);
		const fresh = !!inspection && inspection.readAt >= sinceS;
		const row = inspection?.value.placements.find(
			(entry) => entry.id === serviceId,
		);
		if (fresh && readyToRemove(row)) return true;
		await new Promise((resolve) => setTimeout(resolve, STOP_POLL_MS));
	}
	return false;
}

async function stopAndWait(flow: Flow) {
	const { t } = flow;
	const names = namesOf(flow);
	const sinceS = Math.floor(flow.workspace.clock.now() / 1000);
	const halted = await sendCommand(flow, "stop");
	if (halted.status === "gated")
		explain(
			flow,
			halted,
			t(
				"devices:serviceConfig.remove.notStopped",
				"{{service}} wasn't stopped or removed.",
				names,
			),
		);
	if (halted.status !== "done") return false;
	flow.say({
		tone: "info",
		text: t(
			"devices:serviceConfig.remove.stopping",
			"{{service}} is stopping. It's removed as soon as {{device}} reports it stopped.",
			names,
		),
	});
	const stopped = await waitStopped(flow, sinceS);
	flow.say(
		stopped
			? null
			: {
					tone: "warning",
					text: t(
						"devices:serviceConfig.remove.stillStopping",
						"{{device}} didn't report {{service}} as stopped within {{seconds}} s, so it wasn't removed. Remove it once its status shows Stopped.",
						{
							service: flow.serviceId,
							device: flow.deviceLabel,
							seconds: (STOP_POLLS * STOP_POLL_MS) / 1000,
						},
					),
				},
	);
	return stopped;
}

async function revokeAll(flow: Flow) {
	const { t, cloud } = flow;
	const steps = [
		...cloud.limits.map((id) => ({ kind: "billing" as const, id })),
		...cloud.approvals.map((id) => ({ kind: "resource" as const, id })),
	];
	for (const step of steps) {
		const outcome = await revokeGrant(flow, step.kind, step.id);
		if (outcome.status === "done") continue;
		explain(
			flow,
			outcome,
			flow.stopFirst
				? t(
						"devices:serviceConfig.remove.notRevokedStopped",
						"Its cloud access wasn't revoked, so {{service}} wasn't removed. It stays stopped. Try again, or remove it without revoking.",
						namesOf(flow),
					)
				: t(
						"devices:serviceConfig.remove.notRevoked",
						"Its cloud access wasn't revoked, so {{service}} wasn't removed. Try again, or remove it without revoking.",
						namesOf(flow),
					),
		);
		return false;
	}
	return true;
}

/** True once the device removed the service. */
async function removeService(flow: Flow, revoking: boolean) {
	const { t } = flow;
	if (flow.stopFirst && !(await stopAndWait(flow))) return false;
	if (revoking && !(await revokeAll(flow))) return false;
	const removed = await sendCommand(flow, "remove");
	if (removed.status === "done") return true;
	if (revoking)
		explain(
			flow,
			removed,
			t(
				"devices:serviceConfig.remove.revokedNotRemoved",
				"{{service}} wasn't removed. Its cloud access is already revoked; approve it again under Cloud access if you keep the service.",
				namesOf(flow),
			),
		);
	else if (removed.status === "gated")
		explain(
			flow,
			removed,
			t(
				"devices:serviceConfig.remove.notRemoved",
				"{{service}} wasn't removed.",
				namesOf(flow),
			),
		);
	return false;
}

/* What the zone knows about the service. */

function useMounted() {
	const mounted = useRef(true);
	useEffect(() => {
		mounted.current = true;
		return () => {
			mounted.current = false;
		};
	}, []);
	return mounted;
}

/** The service's cloud access as the hub lists it, and why this account can't end it right now. */
function useCloudAccess(
	deviceId: string,
	serviceId: string,
	read: ServiceConfigRead,
) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { input } = useAttentionState();
	const resources = useDeviceResources(deviceId);
	const owner = read.device?.relationship === "owner";
	const projectId = read.service?.projectId;
	// Whether an approval has ended is judged once a minute, not on every tick of the clock.
	const minuteS = Math.floor(time.nowS / 60) * 60;
	const cloud = useMemo(
		() =>
			serviceCloudAccess(
				resources.data,
				serviceId,
				{ id: input.me, owner },
				minuteS,
			),
		[resources.data, serviceId, input.me, owner, minuteS],
	);
	const target = useMemo<GateTarget>(
		() => ({
			placementId: serviceId,
			...(projectId ? { projectId } : {}),
			labels: { service: serviceId },
			extra: REVOCABLE,
		}),
		[serviceId, projectId],
	);
	const gates = useGates(
		["cloud_access_revoke", "spending_limit_revoke"],
		deviceId,
		target,
	);
	const inline = (gate: GateResult) =>
		gate.ok ? null : gateCopy(t, gate, time).inline;
	const gate =
		(cloud.approvals.length ? inline(gates.cloud_access_revoke) : null) ??
		(cloud.limits.length ? inline(gates.spending_limit_revoke) : null);
	return { cloud, gate };
}

interface Removal {
	/** Why the button can't be used; the reason shows under it (R7). */
	blocked: Gate | null;
	busy: boolean;
	note: Note | null;
	resultKey: string;
	open(): void;
	dismiss(): void;
}

function useRemoval(
	deviceId: string,
	serviceId: string,
	read: ServiceConfigRead,
): Removal {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const confirm = useConfirm();
	const actions = useDeviceAction();
	const { workspace } = useAttentionState();
	const { navigate } = useDevicesRoute();
	const commands = useServiceCommands(deviceId, serviceId);
	const { cloud, gate } = useCloudAccess(deviceId, serviceId, read);
	const mounted = useMounted();
	const [busy, setBusy] = useState(false);
	const [note, setNote] = useState<Note | null>(null);
	const choice = useRef<RemoveChoice>({ revoke: false, lose: false });
	const resultKey = `service-remove:${deviceId}/${serviceId}`;
	const { service, deviceLabel, configuration } = read;

	const removeGate = commands.remove.gate;
	// A running service is stopped first when the person may stop it; the confirm says so.
	const stopFirst =
		!removeGate.ok &&
		removeGate.copy.code === "service_must_be_stopped" &&
		commands.stop.gate.ok;
	const blocked = removeGate.ok || stopFirst ? null : removeGate;
	const online = (configuration?.config.source ?? service?.source) === "online";
	const waiting = waitingChanges(
		online,
		configuration ? !!configuration.config.offline_writes : true,
		service?.offlineWrites,
	);
	const hosting =
		configuration && service?.desired === "running"
			? hostingOf(configuration.config)
			: null;
	const authority = hosting
		? `${hosting.host.includes(":") ? `[${hosting.host}]` : hosting.host}:${hosting.port}`
		: null;

	const rows: ConsequenceRows = {
		...commands.remove.rows,
		who: (
			<>
				{authority ? (
					<>
						<Trans
							t={t}
							i18nKey="serviceConfig.remove.whoPage"
							defaults="Its service page at <1/> stops answering."
							components={{ 1: <Mono>{authority}</Mono> }}
						/>{" "}
					</>
				) : null}
				<CloudAccessLine
					serviceId={serviceId}
					cloud={cloud}
					gate={gate}
					hosted={authority !== null}
					unknown={commands.remove.rows.who}
					onRevoke={(revoke) => {
						choice.current.revoke = revoke;
					}}
				/>
			</>
		),
		...(online
			? {
					stays: t(
						"serviceConfig.remove.staysOnline",
						"Its data is in the cloud and isn't touched; other services of the app keep using it.",
					),
				}
			: {}),
		when: stopFirst
			? t("serviceConfig.remove.whenStop", "It stops first, then it's removed.")
			: commands.remove.rows.when,
		first:
			stopFirst || waiting ? (
				<BeforeRemoval
					stopFirst={stopFirst}
					waiting={waiting}
					onLose={(lose) => {
						choice.current.lose = lose;
					}}
				/>
			) : undefined,
	};

	const flow: Flow = {
		t,
		time,
		actions,
		workspace,
		commands,
		deviceId,
		serviceId,
		deviceLabel,
		projectId: service?.projectId,
		resultKey,
		cloud,
		stopFirst,
		say: (next) => {
			if (mounted.current) setNote(next);
		},
	};

	const open = async () => {
		if (busy || blocked) return;
		choice.current = { revoke: false, lose: false };
		const answer = await confirm({
			icon: Trash2,
			title: commands.remove.title,
			sub: commands.remove.sub,
			rows,
			strength: "typed",
			typed: serviceId,
			tone: "danger",
			confirmLabel: stopFirst
				? t(
						"serviceConfig.remove.stopAndRemove",
						"Stop and remove {{service}}",
						{ service: serviceId },
					)
				: commands.remove.label,
			onConfirm: () => {
				if (typeof waiting === "number" && waiting > 0 && !choice.current.lose)
					throw new Error(
						t(
							"serviceConfig.remove.needLose",
							"Tick the box under “Do this first” to remove a service whose buffered changes haven't reached the cloud.",
						),
					);
			},
		});
		if (!answer.ok) return;
		setNote(null);
		setBusy(true);
		let removed = false;
		try {
			removed = await removeService(flow, choice.current.revoke);
		} catch {
			// A step that broke outside the action layer still ends in a sentence and a free button.
			flow.say({
				tone: "critical",
				text: t(
					"serviceConfig.remove.interrupted",
					"Removing {{service}} was interrupted before {{device}} confirmed it. Check its status, then try again.",
					{ service: serviceId, device: deviceLabel },
				),
			});
		}
		if (removed) void workspace.live.refreshInspection(deviceId);
		// Someone who left the page meanwhile stays where they went.
		if (!mounted.current) return;
		setBusy(false);
		if (removed) navigate({ screen: "device", deviceId, tab: "services" });
	};

	return {
		blocked: gateLine(t, time, blocked),
		busy: busy || actions.pending(resultKey),
		note,
		resultKey,
		open: () => void open(),
		dismiss: () => setNote(null),
	};
}

/** Remove service: needs a stopped service ("Stop and remove" offered) and the typed service ID. */
export function RemoveServiceZone({
	deviceId,
	serviceId,
	read,
}: Readonly<{ deviceId: string; serviceId: string; read: ServiceConfigRead }>) {
	const { t } = useTranslation("devices");
	const reasonId = useId();
	const removal = useRemoval(deviceId, serviceId, read);
	const { service, deviceLabel, configuration } = read;
	if (!service) return null;
	const online = (configuration?.config.source ?? service.source) === "online";
	const { blocked } = removal;
	return (
		<Block
			id="svc-danger"
			danger
			icon={Trash2}
			title={t("serviceConfig.remove.zone", "Danger zone")}
		>
			<p className="max-w-[92ch] text-ui">
				<b className="font-semibold">
					{t("serviceConfig.remove.lead", "Remove this service.")}
				</b>{" "}
				{t(
					"serviceConfig.remove.text",
					"The service is removed from {{device}} and its ID {{service}} is reserved forever.",
					{ device: deviceLabel, service: serviceId },
				)}{" "}
				{online
					? t(
							"serviceConfig.remove.dataOnline",
							"Its data is in the cloud and isn't touched.",
						)
					: t(
							"serviceConfig.remove.dataDevice",
							"Its data stays on the device.",
						)}{" "}
				{service.desired === "running"
					? t(
							"serviceConfig.remove.running",
							"It has to stop first; the next step offers Stop and remove.",
						)
					: null}
			</p>
			<div className="flex flex-col items-start gap-1">
				<DvButton
					variant="danger-ghost"
					icon={Trash2}
					busy={removal.busy}
					aria-disabled={blocked ? true : undefined}
					aria-describedby={blocked ? reasonId : undefined}
					data-gated={blocked?.kind}
					data-act="service-remove"
					onClick={removal.open}
				>
					{t("serviceConfig.remove.button", "Remove service…")}
				</DvButton>
				{blocked ? (
					<GateInline kind={blocked.kind} id={reasonId}>
						{blocked.reason}
					</GateInline>
				) : null}
			</div>
			<ActionResults
				resultKey={removal.resultKey}
				note={removal.note}
				onDismiss={removal.dismiss}
			/>
		</Block>
	);
}
