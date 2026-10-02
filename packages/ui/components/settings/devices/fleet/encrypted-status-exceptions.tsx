"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { CircleCheck, Lock, LockKeyhole, LockOpen } from "lucide-react";
import { useMemo } from "react";
import { toast } from "sonner";
import { useUserIdentity } from "../../../../hooks/use-user-lookup";
import { keysLocked } from "../../../../lib/device-management/model/device-view";
import type { DeviceViewModel } from "../../../../lib/device-management/model/types";
import { identityName } from "../access/person-name";
import type { DevicesT } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { Meter } from "../primitives/meter";
import { KeyChip } from "../primitives/status-chip";
import { TONE_SOLID, type Tone, cx } from "../primitives/tone";
import { useRouteLink } from "../routing/use-devices-route";
import { keyChipOf } from "../shell/keys-popover";
import { useCoverage, useKeyChip, useOverlayStore } from "../workspace";
import type { FleetDeviceEntry } from "./fleet-device-row";

/** SPEC §5.1 scale behaviour: five exception rows, then "and N more". */
export const ENCRYPTED_EXCEPTION_CAP = 5;

const LINK = "underline underline-offset-2 hover:text-foreground";

function Swatch({ tone }: Readonly<{ tone?: Tone }>) {
	return (
		<i
			aria-hidden
			className={cx(
				"inline-block size-2 rounded-[2px]",
				tone ? TONE_SOLID[tone] : "bg-muted-foreground",
			)}
		/>
	);
}

/** Why an unlocked device still can't be read (R6: never an empty list). */
function unreadableReason(t: DevicesT, view: DeviceViewModel): string {
	const state = Array.isArray(view.services) ? undefined : view.services.state;
	if (state === "noaccess")
		return view.keys.state === "stale"
			? t("devices:fleet.encrypted.accessEnded", "Your access has ended")
			: t(
					"devices:fleet.encrypted.noStatusAccess",
					"Your access doesn't include View status",
				);
	if (state === "unsupported")
		return t(
			"devices:fleet.encrypted.agentTooOld",
			"The device agent is too old to send status",
		);
	if (state === "error")
		return t("devices:fleet.encrypted.readFailed", "The last read failed");
	return t("devices:fleet.encrypted.waiting", "Waiting for the first status");
}

function OwnerKeys({ view }: Readonly<{ view: DeviceViewModel }>) {
	const { t } = useTranslation("devices");
	const ownerId = view.row.owner_id;
	const owner = identityName(useUserIdentity(ownerId), ownerId);
	return owner
		? t("fleet.encrypted.sharedKeysFrom", "Shared-access keys from {{owner}}", {
				owner,
			})
		: t("fleet.encrypted.sharedKeys", "Shared-access keys");
}

function ExceptionRow({ view, name }: Readonly<FleetDeviceEntry>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const deviceId = view.row.device_id;
	const locked = keysLocked(view.keys);
	const noKeys = view.keys.state === "none";
	let why: React.ReactNode;
	if (noKeys) why = t("fleet.encrypted.noKeys", "No keys on this computer");
	else if (!locked) why = unreadableReason(t, view);
	else if (view.presence.kind === "never")
		why = t("fleet.encrypted.noStatusYet", "no status sent yet");
	else if (view.relationship === "owner")
		why = t("fleet.encrypted.ownerKeys", "Owner keys, closed");
	else why = <OwnerKeys view={view} />;
	return (
		<div
			data-exception={deviceId}
			className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 gap-y-0.5 border-t border-hairline py-2 first:border-t-0"
		>
			<span className="flex min-w-0 flex-wrap items-center gap-1.5">
				<a
					{...link({ screen: "device", deviceId, tab: "overview" })}
					title={name}
					className="truncate font-mono text-ui font-semibold hover:underline"
				>
					{name}
				</a>
				<KeyChip {...keyChipOf(view.keys, view.live)} />
			</span>
			<span className="row-span-2 self-center">
				{view.keys.state === "locked" ? (
					<DvButton
						size="sm"
						icon={LockOpen}
						onClick={() => useOverlayStore.getState().openUnlock(deviceId)}
					>
						{t("fleet.encrypted.unlock", "Unlock…")}
					</DvButton>
				) : noKeys ? (
					<DvButton size="sm" asChild>
						<a {...link({ screen: "keys", focusDeviceId: deviceId })}>
							{t("fleet.encrypted.restore", "Restore keys…")}
						</a>
					</DvButton>
				) : null}
			</span>
			<p className="col-start-1 text-xs text-muted-foreground">{why}</p>
		</div>
	);
}

/** SPEC §5.1 Encrypted status: how much of the fleet this computer can read, and only the devices it can't. */
export function EncryptedStatusExceptions({
	entries,
}: Readonly<{ entries: readonly FleetDeviceEntry[] }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const cover = useCoverage();
	const keyChip = useKeyChip();
	const exceptions = useMemo(
		() =>
			// A device you only approved cloud access for was never yours to read: no row, nothing to unlock.
			entries.filter(
				({ view }) =>
					view.row.status === "active" &&
					view.relationship !== "cloud_approval" &&
					!Array.isArray(view.services),
			),
		[entries],
	);
	const lockedCount = exceptions.filter(
		({ view }) => view.keys.state === "locked",
	).length;
	const other = cover.unknown.length - cover.locked.length;
	const share = (count: number) =>
		cover.total > 0 ? (count / cover.total) * 100 : 0;
	const counts = {
		live: cover.live,
		snapshot: cover.snapshot,
		locked: cover.locked.length,
	};
	const hidden = exceptions.length - ENCRYPTED_EXCEPTION_CAP;

	return (
		<Block
			icon={LockKeyhole}
			title={t("fleet.encrypted.title", "Encrypted status")}
			stamp={
				<FreshnessStamp
					source="snap"
					age="current"
					text={t("fleet.encrypted.stamp", "per device")}
				/>
			}
			foot={
				<>
					{lockedCount > 0 ? (
						<DvButton
							size="sm"
							icon={LockOpen}
							onClick={() => useOverlayStore.getState().openUnlockSeveral()}
						>
							{t("fleet.encrypted.unlockSeveral", {
								count: lockedCount,
								defaultValue_one: "Unlock {{count, number}} device…",
								defaultValue_other: "Unlock {{count, number}} devices…",
							})}
						</DvButton>
					) : null}
					<DvButton
						size="sm"
						variant="ghost"
						icon={Lock}
						onClick={() => {
							keyChip.lockAll();
							toast(
								t(
									"fleet.encrypted.lockedAll",
									"All devices are locked on this computer.",
								),
							);
						}}
					>
						{t("fleet.encrypted.lockAll", "Lock all")}
					</DvButton>
					<button
						type="button"
						className={LINK}
						onClick={() => useOverlayStore.getState().openPlane("status")}
					>
						{t("fleet.encrypted.seeAll", "See all {{count, number}}", {
							count: cover.total,
						})}
					</button>
					<a {...link({ screen: "keys" })} className={LINK}>
						{t("fleet.encrypted.keys", "Keys & recovery")}
					</a>
					<span>
						{t(
							"fleet.encrypted.idle",
							"Unlocked devices lock after 30 min unused.",
						)}
					</span>
				</>
			}
		>
			<p className="text-ui text-ink-2">
				{t(
					"fleet.encrypted.explain",
					"Devices send their status to the hub encrypted. Only keys on this computer can read it, even while a device is offline.",
				)}
			</p>
			<div className="flex flex-col gap-2">
				<p className="text-ui" data-encrypted-summary="">
					<Trans
						t={t}
						i18nKey="fleet.encrypted.summary"
						defaults="<1>{{readable, number}} of {{total, number}}</1> active devices readable · {{live, number}} live · {{snapshot, number}} snapshot · {{locked, number}} locked"
						values={{
							...counts,
							readable: cover.readable,
							total: cover.total,
						}}
						components={{ 1: <b className="font-semibold" /> }}
					/>
				</p>
				<Meter
					label={t(
						"fleet.encrypted.meter",
						"{{live, number}} live, {{snapshot, number}} from snapshots, {{locked, number}} locked",
						counts,
					)}
					segments={[
						{ value: share(cover.live), tone: "good" },
						{ value: share(cover.snapshot), tone: "unknown" },
						{ value: share(cover.locked.length), tone: "locked" },
					]}
				/>
				<p className="flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground">
					<span className="inline-flex items-center gap-1">
						<Swatch tone="good" />
						{t("fleet.encrypted.legendLive", "{{count, number}} live", {
							count: cover.live,
						})}
					</span>
					<span className="inline-flex items-center gap-1">
						<Swatch tone="unknown" />
						{t("fleet.encrypted.legendSnapshot", "{{count, number}} snapshot", {
							count: cover.snapshot,
						})}
					</span>
					<span className="inline-flex items-center gap-1">
						<Swatch tone="locked" />
						{t("fleet.encrypted.legendLocked", "{{count, number}} locked", {
							count: cover.locked.length,
						})}
					</span>
					{other > 0 ? (
						<span className="inline-flex items-center gap-1">
							<Swatch />
							{t(
								"fleet.encrypted.legendOther",
								"{{count, number}} not readable for another reason",
								{ count: other },
							)}
						</span>
					) : null}
				</p>
			</div>
			{exceptions.length ? (
				<div className="flex flex-col">
					{exceptions.slice(0, ENCRYPTED_EXCEPTION_CAP).map((entry) => (
						<ExceptionRow key={entry.view.row.device_id} {...entry} />
					))}
					{hidden > 0 ? (
						<p className="pt-1 text-xs text-muted-foreground">
							{t(
								"fleet.encrypted.more",
								"and {{count, number}} more. They're marked in the device list.",
								{ count: hidden },
							)}
						</p>
					) : null}
				</div>
			) : (
				<p className="flex items-center gap-2 text-ui text-ink-2">
					<CircleCheck aria-hidden className="size-4 text-good" />
					<span>
						{t(
							"fleet.encrypted.allReadable",
							"All {{count, number}} active devices readable.",
							{ count: cover.total },
						)}
					</span>
				</p>
			)}
		</Block>
	);
}
