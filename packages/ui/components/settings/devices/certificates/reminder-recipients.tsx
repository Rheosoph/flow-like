"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import { Users } from "lucide-react";
import { useState } from "react";
import { queries } from "../../../../lib/device-management/hub/queries";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import {
	presence,
	relationshipOf,
} from "../../../../lib/device-management/model/presence";
import type { DeviceRow } from "../../../../lib/device-management/model/types";
import type { ManagementGrant } from "../../../../lib/device-management/types";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { CellSub, DvTable, Td, Th, Tr } from "../primitives/dv-table";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { PersonChip } from "../primitives/person-chip";
import { StateView } from "../primitives/state-view";
import { useRouteLink } from "../routing/use-devices-route";
import { stampOf } from "../shell/attention-popover";
import { useDeviceWorkspace } from "../workspace/device-workspace-provider";
import { useOverlayStore } from "../workspace/overlay-store";
import {
	useAttentionInput,
	useAttentionState,
} from "../workspace/use-attention";
import { useDeviceRows, useMyAccess } from "../workspace/use-hub";
import { type CertificateFleet, LIST_CAP } from "./certificates-model";
import { DeviceCell, LINK, LINK_BUTTON, TABLE_RESET } from "./parts";
import { type CertificateFleetRead, usePerson } from "./use-certificates";

/** Reminders go to the owner and to people whose whole-device access includes one of these. */
const RECEIVES = ["status", "manage_certificates"] as const;

function receives(grant: Pick<ManagementGrant, "scope" | "capabilities">) {
	return (
		grant.scope.kind === "device" &&
		grant.capabilities.some((capability) =>
			(RECEIVES as readonly string[]).includes(capability),
		)
	);
}

interface RecipientLabels {
	device: string;
	receive: string;
	note: string;
}

function Person({
	userId,
	you = false,
	meta,
}: Readonly<{ userId: string; you?: boolean; meta: string }>) {
	const { t } = useTranslation("devices");
	const person = usePerson(userId);
	const fallback = you
		? t("certificates.recipients.you", "You")
		: t("certificates.recipients.someone", "Unknown person");
	return (
		<span className="inline-flex flex-wrap items-center gap-x-1.5 gap-y-0.5">
			<PersonChip
				name={person.name ?? fallback}
				avatarUrl={person.avatarUrl}
				you={you}
			/>
			<span className="text-xs text-muted-foreground">{meta}</span>
		</span>
	);
}

function NoteCell({
	label,
	note,
}: Readonly<{ label: string; note: string | null }>) {
	return (
		<Td label={label}>
			{note ? (
				<span className="text-xs text-muted-foreground">{note}</span>
			) : (
				<span className="text-muted-foreground">–</span>
			)}
		</Td>
	);
}

/** Why a device has nothing to remind about yet, when that is the case. */
function reportNote(
	t: DevicesT,
	fleet: CertificateFleet,
	device: DeviceRow,
): string | null {
	const silent = fleet.silent.find(
		(entry) => entry.device.device_id === device.device_id,
	);
	if (silent?.state === "none")
		return t(
			"devices:certificates.recipients.noCertificates",
			"No certificates, so no reminders.",
		);
	if (silent?.state !== "never") return null;
	return silent.presence.kind === "never"
		? t(
				"devices:certificates.recipients.neverCheckedIn",
				"It hasn't checked in yet, so there's nothing to remind about.",
			)
		: t(
				"devices:certificates.recipients.notReported",
				"Nothing to remind about until it reports certificates.",
			);
}

function accessNote(
	t: DevicesT,
	narrower: number,
	recipients: number,
): string | null {
	if (narrower)
		return t("devices:certificates.recipients.narrower", {
			count: narrower,
			defaultValue_one:
				"{{count, number}} person with narrower access isn't included.",
			defaultValue_other:
				"{{count, number}} people with narrower access aren't included.",
		});
	return recipients
		? t(
				"devices:certificates.recipients.untilAccessEnds",
				"People stop receiving them when their access ends.",
			)
		: null;
}

/** The viewer's own device: the owner, plus the people the access rules give whole-device access (read with the device's open keys). */
function OwnedRecipients({
	device,
	fleet,
	labels,
}: Readonly<{
	device: DeviceRow;
	fleet: CertificateFleet;
	labels: RecipientLabels;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const openUnlock = useOverlayStore((store) => store.openUnlock);
	const { hub } = useDeviceWorkspace();
	const { verifyPolicy, input } = useAttentionState();
	// The hub's device row says when nothing is shared: no rules to read then.
	const shared = device.access_rules_expire_at !== null;
	const query = useQuery({
		...queries.policy(hub, device.device_id),
		refetchInterval: false,
		enabled: shared,
	});
	const policy = query.data
		? verifyPolicy(device.device_id, query.data)
		: undefined;
	const rulesKnown =
		!shared || policy !== undefined || query.data?.policy_jws === null;
	const others = (policy?.grants ?? []).filter(
		(grant) => grant.user_id !== input.me && grant.expires_at > time.nowS,
	);
	const recipients = others.filter(receives);
	const narrower = new Set(
		others.filter((grant) => !receives(grant)).map((grant) => grant.user_id),
	).size;
	return (
		<>
			<Td label={labels.receive}>
				<span className="flex flex-wrap gap-x-4 gap-y-1.5">
					<Person
						userId={input.me}
						you
						meta={t("certificates.recipients.owner", "owner")}
					/>
					{recipients.map((grant) => (
						<Person
							key={grant.grant_id}
							userId={grant.user_id}
							meta={t(
								"certificates.recipients.wholeDevice",
								"whole-device access · until {{date}}",
								{ date: time.at(grant.expires_at) },
							)}
						/>
					))}
				</span>
				{rulesKnown ? null : (
					<CellSub>
						{query.error ? (
							t(
								"certificates.recipients.rulesUnread",
								"Couldn't read who else has access.",
							)
						) : (
							<button
								type="button"
								onClick={() => openUnlock(device.device_id)}
								className={LINK_BUTTON}
							>
								{t(
									"certificates.recipients.unlock",
									"Unlock to see who else receives them",
								)}
							</button>
						)}
					</CellSub>
				)}
			</Td>
			<NoteCell
				label={labels.note}
				note={
					reportNote(t, fleet, device) ??
					accessNote(t, narrower, recipients.length)
				}
			/>
		</>
	);
}

/** A device shared with the viewer: its owner, and the viewer when their access covers the whole device. */
function SharedRecipients({
	device,
	labels,
	cloudOnly,
}: Readonly<{
	device: DeviceRow;
	labels: RecipientLabels;
	cloudOnly: boolean;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const me = useAttentionInput().me;
	const access = useMyAccess(cloudOnly ? undefined : device.device_id);
	const mine = access.data?.grants.find(
		(grant) => grant.expires_at > time.nowS && receives(grant),
	);
	let note: string | null = null;
	if (cloudOnly)
		note = t(
			"certificates.recipients.cloudOnly",
			"You don't receive them: you only approved cloud access for one of its services.",
		);
	else if (mine)
		note = t(
			"certificates.recipients.youToo",
			"You stop receiving them when your access ends.",
		);
	else if (access.data)
		note = t(
			"certificates.recipients.notYou",
			"You don't receive them: your access covers part of the device only.",
		);
	return (
		<>
			<Td label={labels.receive}>
				<span className="flex flex-wrap gap-x-4 gap-y-1.5">
					<Person
						userId={device.owner_id}
						meta={t("certificates.recipients.owner", "owner")}
					/>
					{mine ? (
						<Person
							userId={me}
							you
							meta={t(
								"certificates.recipients.wholeDevice",
								"whole-device access · until {{date}}",
								{ date: time.at(mine.expires_at) },
							)}
						/>
					) : null}
				</span>
			</Td>
			<NoteCell label={labels.note} note={note} />
		</>
	);
}

/** Who the hub reminds for each device: the owner and people with whole-device View status or Manage certificates. */
export function RecipientsBlock({
	read,
}: Readonly<{ read: CertificateFleetRead }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const list = useDeviceRows();
	const me = useAttentionInput().me;
	const [all, setAll] = useState(false);
	const { fleet } = read;
	const devices = fleet.devices;
	const shown = all ? devices : devices.slice(0, LIST_CAP);
	const labels: RecipientLabels = {
		device: t("certificates.recipients.column.device", "Device"),
		receive: t("certificates.recipients.column.receive", "Receive reminders"),
		note: t("certificates.recipients.column.note", "Note"),
	};
	return (
		<Block
			icon={Users}
			title={t("certificates.recipients.title", "Recipients by device")}
			count={read.loaded ? devices.length : undefined}
			stamp={<FreshnessStamp {...stampOf(list.freshness)} />}
			foot={
				<span>
					<Trans
						t={t}
						i18nKey="certificates.recipients.foot"
						defaults="Change who receives them by changing whole-device access on the <1>Access page</1>."
						components={{
							1: (
								<a
									{...link({ screen: "access", tab: "people" })}
									className={LINK}
								/>
							),
						}}
					/>
				</span>
			}
			flush
		>
			{!read.loaded ? (
				<div className="p-4">
					<StateView kind="loading" rows={2} />
				</div>
			) : devices.length ? (
				<DvTable
					cols={["22%", "46%", "32%"]}
					className={TABLE_RESET}
					label={t(
						"certificates.recipients.caption",
						"Who receives expiry reminders for each device",
					)}
					head={
						<tr>
							<Th>{labels.device}</Th>
							<Th>{labels.receive}</Th>
							<Th>{labels.note}</Th>
						</tr>
					}
				>
					{shown.map((device) => {
						const relationship = relationshipOf(device, me);
						return (
							<Tr key={device.device_id}>
								<Td label={labels.device} kind="name">
									<DeviceCell
										deviceId={device.device_id}
										name={deviceName(device)}
										presence={presence(device, read.now)}
										tab="access"
									>
										{relationship === "owner"
											? t("certificates.recipients.yours", "Yours")
											: t("certificates.recipients.shared", "Shared with you")}
									</DeviceCell>
								</Td>
								{relationship === "owner" ? (
									<OwnedRecipients
										device={device}
										fleet={fleet}
										labels={labels}
									/>
								) : (
									<SharedRecipients
										device={device}
										labels={labels}
										cloudOnly={relationship === "cloud_approval"}
									/>
								)}
							</Tr>
						);
					})}
				</DvTable>
			) : (
				<div className="p-4">
					<StateView
						kind="empty"
						icon={Users}
						title={t("certificates.recipients.empty", "No devices yet")}
						text={t(
							"certificates.recipients.emptyText",
							"Reminders start once a device reports a certificate.",
						)}
					/>
				</div>
			)}
			{devices.length > shown.length ? (
				<div className="flex flex-wrap items-center justify-between gap-2 border-t border-hairline px-4 py-2.5 text-xs text-muted-foreground">
					<span>
						{t(
							"certificates.recipients.showing",
							"Showing {{shown, number}} of {{count, number}} devices",
							{ shown: shown.length, count: devices.length },
						)}
					</span>
					<DvButton size="sm" onClick={() => setAll(true)}>
						{t("certificates.recipients.more", "Show {{count, number}} more", {
							count: devices.length - shown.length,
						})}
					</DvButton>
				</div>
			) : null}
		</Block>
	);
}
