"use client";

import { useTranslation } from "@flow-like/locales";
import { Power } from "lucide-react";
import {
	type RefObject,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import { deviceKeys } from "../../../../lib/device-management/hub/queries";
import type { ServiceView } from "../../../../lib/device-management/model/types";
import { revokeDeviceGrant } from "../../../../lib/device-resources";
import { revokeDevice } from "../../../../lib/devices";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import type { ConsequenceRows } from "../primitives/consequence-preview";
import { CheckField } from "../primitives/form-fields";
import {
	type DeviceActionContext,
	type DeviceActionOutcome,
	actionResultKey,
	useAttentionState,
	useDeviceAction,
	useDeviceWorkspace,
	usePolicy,
	useResourceSummary,
} from "../workspace";
import { type DevicePage, sharedPeople } from "./use-device-page";

export interface RevokeOptions {
	stopServices: boolean;
	revokeCloud: boolean;
}

interface RevokeFacts {
	running: ServiceView[];
	/** A live session is open, so the services can be stopped before the hub forgets the device. */
	canStop: boolean;
	approvals: string[];
	billing: string[];
	hasCloud: boolean;
}

/** What the options act on; read while the sheet is open, so late hub answers still show their option. */
function useRevokeFacts(page: DevicePage): RevokeFacts {
	const summary = useResourceSummary();
	return useMemo(() => {
		const running = (page.services ?? []).filter(
			(service) => service.desired === "running",
		);
		const device = summary.data?.devices.find(
			(row) => row.device_id === page.deviceId,
		);
		const approvals = (device?.approvals ?? [])
			.filter((row) => row.status === "active" && row.approver_is_me)
			.map((row) => row.grant_id);
		const billing = (device?.billing ?? [])
			.filter((row) => row.payer_is_me)
			.map((row) => row.billing_grant_id);
		return {
			running,
			canStop: page.liveOpen && running.length > 0 && !page.identity,
			approvals,
			billing,
			hasCloud: approvals.length + billing.length > 0,
		};
	}, [
		summary.data,
		page.services,
		page.deviceId,
		page.liveOpen,
		page.identity,
	]);
}

function OptionFields({
	page,
	options,
}: Readonly<{ page: DevicePage; options: RefObject<RevokeOptions> }>) {
	const { t } = useTranslation("devices");
	const id = useId();
	const facts = useRevokeFacts(page);
	const [state, setState] = useState<RevokeOptions>({ ...options.current });
	const update = (patch: Partial<RevokeOptions>) => {
		const next = { ...state, ...patch };
		options.current = next;
		setState(next);
	};
	if (!facts.canStop && !facts.hasCloud) return null;
	return (
		<div data-revoke-options="" className="flex flex-col gap-2">
			{facts.canStop ? (
				<CheckField
					id={`${id}-stop`}
					checked={state.stopServices}
					onCheckedChange={(stopServices) => update({ stopServices })}
				>
					{t("device.revoke.stopFirst", "Stop all running services first")}
				</CheckField>
			) : null}
			{facts.hasCloud ? (
				<CheckField
					id={`${id}-cloud`}
					checked={state.revokeCloud}
					onCheckedChange={(revokeCloud) => update({ revokeCloud })}
				>
					{t(
						"device.revoke.alsoCloud",
						"Also revoke its cloud access and spending limits",
					)}
				</CheckField>
			) : null}
		</div>
	);
}

/** Who loses access: the people the rules name, or everyone when the rules can't be read here. */
function whoRow(
	t: DevicesT,
	device: string,
	people: number | undefined,
): string {
	if (people === undefined)
		return t(
			"devices:device.revoke.rows.who",
			"Everyone you shared {{device}} with. Their live connections close.",
			{ device },
		);
	if (people === 0)
		return t(
			"devices:device.revoke.rows.whoNone",
			"Only you: {{device}} isn't shared with anyone.",
			{ device },
		);
	return t("devices:device.revoke.rows.whoPeople", {
		count: people,
		device,
		defaultValue_one:
			"You and the {{count, number}} person you shared {{device}} with. Their live connections close.",
		defaultValue_other:
			"You and the {{count, number}} people you shared {{device}} with. Their live connections close.",
	});
}

/** What revoking leaves alone: the services by name when they're known, and the data on the device. */
function staysRow(
	t: DevicesT,
	locale: string,
	running: readonly ServiceView[],
): string {
	const kept = t(
		"devices:device.revoke.rows.data",
		"Nothing on the device is erased. Access tokens deployed to it may still work; rotate them in each app's events.",
	);
	if (!running.length)
		return `${t(
			"devices:device.revoke.rows.stays",
			"Services on it keep running until someone stops them there.",
		)} ${kept}`;
	const services = new Intl.ListFormat(locale, {
		type: "conjunction",
	}).format(running.map((service) => service.serviceId));
	return `${t("devices:device.revoke.rows.staysServices", {
		count: running.length,
		services,
		defaultValue_one:
			"{{services}} keeps running on the device until someone stops it there.",
		defaultValue_other:
			"{{services}} keep running on the device until someone stops them there.",
	})} ${kept}`;
}

function useRevokeRows(
	page: DevicePage,
	running: readonly ServiceView[],
	people: number | undefined,
): ConsequenceRows {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const device = page.name;
	return {
		what: t(
			"device.revoke.rows.what",
			"The hub refuses {{device}} from its next check-in. Nobody can manage it from Flow-Like again, you included. Revoking happens at the hub, so it doesn't need your device password and works even if the device is offline.",
			{ device },
		),
		who: whoRow(t, device, people),
		stays: staysRow(t, time.locale, running),
		when: t(
			"device.revoke.rows.when",
			"{{device}} stays in your list as Revoked, with its cloud approvals visible so you can close them.",
			{ device },
		),
		undo: {
			reversible: false,
			text: t(
				"device.revoke.rows.undo",
				"There's no way to reactivate {{device}}; to reuse this hardware, set it up as a new device.",
				{ device },
			),
		},
		first: t(
			"device.revoke.rows.cloud",
			"Its cloud access and spending limits stay active unless you revoke them. Cloud credentials already issued stay valid for up to 1 hour; relay credentials for up to 4 hours.",
		),
	};
}

/** The chosen extras go first, while the hub still knows the device; billing before the approvals it pays for. */
async function revokeAtHub(
	context: DeviceActionContext,
	deviceId: string,
	chosen: RevokeOptions,
	facts: RevokeFacts,
): Promise<void> {
	const { api, profile } = context.workspace.hub;
	if (chosen.stopServices && facts.canStop)
		for (const service of facts.running)
			await context.request({
				type: "stop",
				placement_id: service.serviceId,
				expected_revision: service.settings.latest,
			});
	if (chosen.revokeCloud) {
		for (const id of facts.billing)
			await revokeDeviceGrant(api, profile, deviceId, "billing", id);
		for (const id of facts.approvals)
			await revokeDeviceGrant(api, profile, deviceId, "resource", id);
	}
	await revokeDevice(api, profile, deviceId);
}

export interface RevokeFlow {
	resultKey: string;
	pending: boolean;
	run(): Promise<DeviceActionOutcome<RevokeOptions>>;
}

/**
 * SPEC §6.5 "Revoke device": review, then the typed name. It runs at the hub,
 * so it needs no keys, no password and no connection to the device.
 */
export function useRevokeFlow(page: DevicePage): RevokeFlow {
	const { t } = useTranslation("devices");
	const actions = useDeviceAction();
	const workspace = useDeviceWorkspace();
	const { input } = useAttentionState();
	const facts = useRevokeFacts(page);
	const policy = usePolicy(
		page.owner && !page.revoked ? page.deviceId : undefined,
	);
	const options = useRef<RevokeOptions>({
		stopServices: false,
		revokeCloud: false,
	});
	const latest = useRef(facts);
	useEffect(() => {
		latest.current = facts;
	});
	const unshared = policy.data?.version === 0 ? 0 : undefined;
	const people = sharedPeople(policy.policy, input.me) ?? unshared;
	const rows = useRevokeRows(page, facts.running, people);
	const resultKey = actionResultKey("revoke_device", page.deviceId);
	const { deviceId, name } = page;
	return {
		resultKey,
		pending: actions.pending(resultKey),
		run: () => {
			options.current = { stopServices: false, revokeCloud: false };
			return actions.run<RevokeOptions>({
				action: "revoke_device",
				deviceId,
				label: t("device.revoke.label", "Revoke {{device}}", { device: name }),
				consequence: rows,
				strength: "review",
				confirm: {
					icon: Power,
					title: t("device.revoke.title", "Revoke {{device}}?", {
						device: name,
					}),
					sub: t("device.revoke.subtitle", "Permanent · happens at the hub"),
					typed: name,
					tone: "danger",
					wide: true,
					whoLabel: "loses",
					rowLabels: {
						what: t("device.revoke.labels.what", "Changes now"),
						stays: t("device.revoke.labels.stays", "Keeps running"),
						when: t("device.revoke.labels.when", "Afterwards"),
						undo: t("device.revoke.labels.undo", "Permanent"),
						first: t("device.revoke.labels.first", "Cloud & credentials"),
					},
					extra: <OptionFields page={page} options={options} />,
				},
				resultKey,
				call: async (context) => {
					const chosen = { ...options.current };
					await revokeAtHub(context, deviceId, chosen, latest.current);
					return chosen;
				},
				invalidate: [
					deviceKeys.list(workspace.scopeKey),
					deviceKeys.resourceSummary(workspace.scopeKey),
					deviceKeys.resources(workspace.scopeKey, deviceId),
				],
			});
		},
	};
}
