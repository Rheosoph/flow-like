"use client";

import { useTranslation } from "@flow-like/locales";
import {
	FileBadge,
	Fingerprint,
	Laptop,
	ShieldCheck,
	Users,
} from "lucide-react";
import {
	groupFingerprint,
	identityFingerprint,
} from "../../../../lib/device-management/fingerprint";
import { DAY_S } from "../../../../lib/device-management/model/device-view";
import type {
	DeviceRow,
	DeviceTab,
} from "../../../../lib/device-management/model/types";
import { enumLabel } from "../copy/enum-labels";
import { useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { type ChainLink, TrustChain } from "../primitives/trust-chain";
import { useAttentionState, useLocalSummary, usePolicy } from "../workspace";
import { type DevicePage, sharedPeople } from "./use-device-page";

const EXPIRING_SOON_S = 7 * DAY_S;

/** The 16 characters `flow-like-standalone status` prints; undefined when the hub row holds no usable keys. */
export function fingerprintOf(
	row: Pick<DeviceRow, "identity">,
): string | undefined {
	try {
		return identityFingerprint(row.identity);
	} catch {
		return undefined;
	}
}

function useKeysLink(page: DevicePage): ChainLink {
	const { t } = useTranslation("devices");
	const { input } = useAttentionState();
	const { keys } = page.view;
	const kind = enumLabel(t, "vaultKind", keys.role);
	const state = enumLabel(t, "keyState", keys.state);
	const revision = input.accountBackups[page.deviceId]?.revision ?? 0;
	const notBackedUp = page.attention.some(
		(item) => item.key === "keys_not_backed_up_to_account",
	);
	if (keys.state === "none")
		return {
			id: "keys",
			icon: Laptop,
			state: "unknown",
			title: t("device.trust.keys", "Keys on this computer"),
			text: t(
				"device.trust.keysNone",
				"None. This computer can't read or manage this device.",
			),
		};
	if (keys.state === "stale")
		return {
			id: "keys",
			icon: Laptop,
			state: "warning",
			title: t("device.trust.keys", "Keys on this computer"),
			text: t(
				"device.trust.keysStale",
				"Left over from access that ended. They can't be unlocked any more.",
			),
		};
	const backup = notBackedUp
		? t("device.trust.notBackedUp", "not backed up")
		: revision > 0
			? t(
					"device.trust.backedUp",
					"backed up to your account (v{{revision, number}})",
					{ revision },
				)
			: undefined;
	return {
		id: "keys",
		icon: Laptop,
		state:
			keys.state === "unlocked" ? (notBackedUp ? "warning" : "good") : "locked",
		title: t("device.trust.keys", "Keys on this computer"),
		text: [kind, state.toLowerCase(), backup].filter(Boolean).join(" · "),
		join: t(
			"device.trust.keysJoin",
			"open the vault that holds this device's keys",
		),
	};
}

function useIdentityLink(page: DevicePage): ChainLink {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const local = useLocalSummary();
	const vault = local.vaults.find((entry) => entry.deviceId === page.deviceId);
	const reported = fingerprintOf(page.view.row);
	const title = t("device.trust.identity", "Device identity");
	if (page.identity)
		return {
			id: "identity",
			icon: Fingerprint,
			state: "critical",
			title,
			text: t(
				"device.trust.identityMismatch",
				"Doesn't match the keys you trusted. Management is blocked.",
			),
		};
	if (page.revoked)
		return {
			id: "identity",
			icon: Fingerprint,
			state: "unknown",
			title,
			text: t(
				"device.trust.identityRevoked",
				"No longer checked: the hub refuses this device.",
			),
		};
	if (!vault)
		return {
			id: "identity",
			icon: Fingerprint,
			state: "unknown",
			title,
			text: reported ? (
				<>
					{t(
						"device.trust.identityHubOnly",
						"Reported by the hub, not checked here ·",
					)}{" "}
					<span className="font-mono">{groupFingerprint(reported)}</span>
				</>
			) : (
				t("device.trust.identityUnknown", "Not known")
			),
		};
	const pinned = vault.identityFingerprint;
	if (!pinned)
		return {
			id: "identity",
			icon: Fingerprint,
			state: "unknown",
			title,
			text: t(
				"device.trust.identityUnpinned",
				"Not trusted on this computer yet. The next unlock checks it.",
			),
		};
	return {
		id: "identity",
		icon: Fingerprint,
		state: "good",
		title,
		text: (
			<>
				{t("device.trust.identityMatches", "Matches the hub ·")}{" "}
				<span className="font-mono">{groupFingerprint(pinned)}</span>
			</>
		),
		...(vault.identityPinnedAt === undefined
			? {}
			: {
					join: t(
						"device.trust.identityJoin",
						"you trusted this device's keys on {{date}}",
						{ date: time.at(vault.identityPinnedAt) },
					),
				}),
	};
}

function useRulesLink(page: DevicePage): ChainLink {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { input } = useAttentionState();
	const read = usePolicy(
		page.consentOnly || page.revoked ? undefined : page.deviceId,
	);
	const view = read.data;
	const expires = page.view.row.access_rules_expire_at ?? undefined;
	const base = { id: "rules", icon: Users } as const;
	if (page.revoked)
		return {
			...base,
			state: "unknown",
			title: t("device.trust.rules", "Access rules"),
			text: t(
				"device.trust.rulesRevoked",
				"No longer enforced: nobody can reach a revoked device.",
			),
		};
	if (!view)
		return {
			...base,
			state: "unknown",
			title: t("device.trust.rules", "Access rules"),
			text: read.error
				? t("device.trust.rulesError", "Couldn't be read from the hub")
				: t("device.trust.rulesLoading", "Not loaded yet"),
		};
	if (view.version === 0)
		return {
			...base,
			state: "good",
			title: t("device.trust.rules", "Access rules"),
			text: page.owner
				? t(
						"device.trust.rulesNone",
						"Not shared. Only you can reach this device.",
					)
				: t(
						"device.trust.rulesNoneShared",
						"The owner hasn't saved access rules.",
					),
		};
	const title = t(
		"device.trust.rulesVersion",
		"Access rules v{{version, number}}",
		{
			version: view.version,
		},
	);
	const people = sharedPeople(read.policy, input.me);
	const expired = expires !== undefined && expires <= time.nowS;
	const parts = [
		view.applied_version === view.version
			? t("device.trust.rulesActive", "Active on device")
			: t(
					"device.trust.rulesWaiting",
					"Device still has v{{version, number}}",
					{ version: view.applied_version },
				),
		people === undefined
			? undefined
			: t("device.trust.rulesPeople", {
					count: people,
					defaultValue_one: "{{count, number}} person",
					defaultValue_other: "{{count, number}} people",
				}),
		expires === undefined
			? undefined
			: expired
				? t("device.trust.rulesExpired", "expired {{date}}", {
						date: time.at(expires),
					})
				: t("device.trust.rulesExpire", "expire {{date}}", {
						date: time.at(expires),
					}),
	].filter(Boolean);
	return {
		...base,
		state: expired
			? "critical"
			: view.applied_version === view.version
				? "good"
				: "warning",
		title,
		text: parts.join(" · "),
		join: page.owner
			? t(
					"device.trust.rulesJoin",
					"enforces access rules v{{version, number}}, signed with your owner key",
					{ version: view.version },
				)
			: t(
					"device.trust.rulesJoinShared",
					"enforces access rules v{{version, number}}, signed by the owner",
					{ version: view.version },
				),
	};
}

function useCertificatesLink(page: DevicePage): ChainLink {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const inventory = page.view.certificates;
	const base = {
		id: "certificates",
		icon: FileBadge,
		title: t("device.trust.certificates", "Certificates"),
	} as const;
	if (!inventory || inventory.updated_at === null)
		return {
			...base,
			state: "unknown",
			text: t(
				"device.trust.certificatesNever",
				"The device hasn't reported certificates",
			),
		};
	const rows = inventory.certificates;
	if (!rows.length)
		return {
			...base,
			state: "good",
			text: t("device.trust.certificatesNone", "None on this device"),
		};
	const next = Math.min(...rows.map((row) => row.not_after));
	const expired = rows.filter((row) => row.not_after <= time.nowS).length;
	if (expired)
		return {
			...base,
			state: "critical",
			text: t("device.trust.certificatesExpired", {
				count: rows.length,
				expired,
				defaultValue_one: "{{count, number}} · {{expired, number}} expired",
				defaultValue_other: "{{count, number}} · {{expired, number}} expired",
			}),
		};
	return {
		...base,
		state: next - time.nowS <= EXPIRING_SOON_S ? "warning" : "good",
		text: t("device.trust.certificatesNext", {
			count: rows.length,
			ago: time.ago(next, "long"),
			defaultValue_one: "{{count, number}} · expires {{ago}}",
			defaultValue_other: "{{count, number}} · the next one expires {{ago}}",
		}),
	};
}

/** SPEC §4.28 as it stands on this device: keys here → device identity → access rules → certificates. */
export function useTrustChain(page: DevicePage): ChainLink[] {
	const keys = useKeysLink(page);
	const identity = useIdentityLink(page);
	const rules = useRulesLink(page);
	const certificates = useCertificatesLink(page);
	return [keys, identity, rules, certificates];
}

/** Overview block 4: the compact chain; the Keys tab shows the full one. */
export function TrustPanel({
	page,
	onTab,
}: Readonly<{ page: DevicePage; onTab(tab: DeviceTab): void }>) {
	const { t } = useTranslation("devices");
	const links = useTrustChain(page);
	return (
		<Block
			id="device-trust"
			icon={ShieldCheck}
			title={t("device.trust.title", "Trust")}
			stamp={
				<FreshnessStamp
					source="local"
					age="current"
					text={t("device.trust.stamp", "checked on this computer")}
				/>
			}
			foot={
				page.tabs.includes("keys") ? (
					<DvButton variant="link" onClick={() => onTab("keys")}>
						{t("device.trust.details", "Keys and trust details")}
					</DvButton>
				) : null
			}
		>
			<TrustChain links={links} compact />
		</Block>
	);
}
