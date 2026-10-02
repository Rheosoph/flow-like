"use client";

import { useTranslation } from "@flow-like/locales";
import {
	FileBadge,
	Fingerprint as FingerprintIcon,
	Laptop,
	Users,
} from "lucide-react";
import type { ReactNode } from "react";
import {
	type PublicCertificateInventory,
	certificateStatus,
} from "../../../../lib/device-management/certificates";
import type {
	DeviceRow,
	MyAccess,
} from "../../../../lib/device-management/model/types";
import type { PolicyView } from "../../../../lib/device-management/types";
import { enumLabel } from "../copy/enum-labels";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import type { ChainLink, ChainState } from "../primitives/trust-chain";
import { useRouteLink } from "../routing/use-devices-route";
import { useDeviceWorkspace } from "../workspace/device-workspace-provider";
import {
	type HubRead,
	type PolicyRead,
	useCertificateInventory,
	useMyAccess,
	usePolicy,
} from "../workspace/use-hub";
import { dayLabel, keyKindLabel, useAppNames } from "./key-parts";
import type { KeyRow } from "./keys-model";
import { Fingerprint } from "./keys-table";

/** Access that ends within six hours is worth a warning tint. */
const ENDING_SOON_S = 6 * 3600;

interface ChainCopy {
	t: DevicesT;
	time: AreaTime;
	day(atS: number): string;
}

/** The app scopes of a shared grant, each linked to its App › Devices page. */
function GrantScopes({
	scopes,
}: Readonly<{ scopes: readonly { kind: string; project_id?: string }[] }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const appName = useAppNames();
	const apps = [
		...new Set(
			scopes.flatMap((scope) => (scope.project_id ? [scope.project_id] : [])),
		),
	];
	if (scopes.some((scope) => scope.kind === "device") || !apps.length)
		return <>{enumLabel(t, "scopeKind", "device")}</>;
	return (
		<>
			{apps.map((appId, index) => (
				<span key={appId}>
					{index > 0 ? ", " : null}
					<a
						{...link(
							{ screen: "app-devices", by: "device" },
							{ scope: { kind: "app", appId } },
						)}
						className="underline decoration-border-strong underline-offset-2 hover:decoration-current"
					>
						{enumLabel(t, "scopeKind", "project", {
							name: appName(appId) ?? appId,
						})}
					</a>
				</span>
			))}
		</>
	);
}

function backupSuffix(t: DevicesT, row: KeyRow): string | undefined {
	const suffixes: Partial<Record<KeyRow["category"], string>> = {
		synced: t(
			"devices:keys.tab.chain.backed",
			"backed up to your account (v{{version}})",
			{ version: row.hubRevision ?? 0 },
		),
		never: t("devices:keys.tab.chain.notBacked", "not backed up"),
		pending: t("devices:keys.tab.chain.pending", "backup upload pending"),
		oldpw: t(
			"devices:keys.tab.chain.oldPassword",
			"account backup opens with the old password",
		),
		hub_newer: t(
			"devices:keys.tab.chain.hubNewer",
			"newer backup on your account",
		),
		out_of_date: t("devices:keys.tab.chain.outOfDate", "backup out of date"),
	};
	return suffixes[row.category];
}

function keysLink({ t }: ChainCopy, row: KeyRow | undefined): ChainLink {
	const base = {
		id: "keys",
		icon: Laptop,
		title: t("devices:keys.tab.chain.keys", "Keys on this computer"),
	};
	if (!row?.vault)
		return {
			...base,
			state: "unknown",
			text: t(
				"devices:keys.tab.chain.keysNone",
				"No keys for this device here",
			),
		};
	const open = row.session?.state === "unlocked";
	const state: ChainState = !open
		? "locked"
		: row.category === "never"
			? "warning"
			: "good";
	const parts = [
		keyKindLabel(t, row.vault.role),
		open
			? t("devices:keys.tab.chain.unlocked", "unlocked")
			: t("devices:keys.tab.chain.locked", "locked"),
		backupSuffix(t, row),
	];
	return { ...base, state, text: parts.filter(Boolean).join(" · ") };
}

function identityLink(
	{ t }: ChainCopy,
	check: "match" | "mismatch" | "unpinned",
	fingerprint: string | undefined,
): ChainLink {
	const base = {
		id: "identity",
		icon: FingerprintIcon,
		title: t("devices:keys.tab.chain.identity", "Device identity"),
	};
	if (check === "mismatch")
		return {
			...base,
			state: "critical",
			text: t(
				"devices:keys.tab.chain.identityMismatch",
				"Doesn't match the hub · management is blocked",
			),
		};
	if (check === "unpinned")
		return {
			...base,
			state: "unknown",
			text: t(
				"devices:keys.tab.chain.identityUnpinned",
				"Not checked yet · this computer checks it at the first unlock",
			),
		};
	return {
		...base,
		state: "good",
		text: (
			<>
				{t("devices:keys.tab.chain.identityMatch", "Matches the hub")}
				{fingerprint ? (
					<>
						{" · "}
						<Fingerprint value={fingerprint} />
					</>
				) : null}
			</>
		),
	};
}

function rulesText(
	{ t, day }: ChainCopy,
	view: PolicyView,
	people: number | undefined,
	expiresAt: number | null | undefined,
): string {
	if (view.version === 0)
		return t("devices:keys.tab.chain.notShared", "Not shared");
	if (view.applied_version !== view.version)
		return t(
			"devices:keys.tab.chain.rulesWaiting",
			"Saved v{{saved}} · device still uses v{{applied}}",
			{ saved: view.version, applied: view.applied_version },
		);
	const parts = [
		t("devices:keys.tab.chain.rulesActive", "Active on device"),
		people === undefined
			? undefined
			: t("devices:keys.tab.chain.rulesPeople", {
					count: people,
					defaultValue_one: "{{count, number}} person",
					defaultValue_other: "{{count, number}} people",
				}),
		expiresAt
			? t("devices:keys.tab.chain.rulesExpire", "expire {{date}}", {
					date: day(expiresAt),
				})
			: undefined,
	];
	return parts.filter(Boolean).join(" · ");
}

function ownerRulesLink(
	copy: ChainCopy,
	policy: PolicyRead,
	device: DeviceRow,
): ChainLink {
	const { t } = copy;
	const view = policy.data;
	const base = { id: "rules", icon: Users };
	if (!view)
		return {
			...base,
			title: t("devices:keys.tab.chain.rulesNone", "Access rules"),
			state: "unknown",
			text: t("devices:keys.tab.chain.notLoaded", "Not loaded yet"),
		};
	const people = policy.policy
		? new Set(policy.policy.grants.map((grant) => grant.user_id)).size
		: undefined;
	return {
		...base,
		title:
			view.version > 0
				? t("devices:keys.tab.chain.rules", "Access rules v{{version}}", {
						version: view.version,
					})
				: t("devices:keys.tab.chain.rulesNone", "Access rules"),
		state: view.applied_version === view.version ? "good" : "warning",
		text: rulesText(copy, view, people, device.access_rules_expire_at),
	};
}

function accessText(
	{ t, time }: ChainCopy,
	access: HubRead<MyAccess>,
): ReactNode {
	const mine = access.data;
	if (mine?.grants.length)
		return (
			<>
				<GrantScopes scopes={mine.grants.map((grant) => grant.scope)} />
				{" · "}
				{t("devices:keys.tab.chain.accessEnds", "ends {{in}}", {
					in: time.ago(
						Math.min(...mine.grants.map((grant) => grant.expires_at)),
						"long",
					),
				})}
			</>
		);
	if (access.missingOnHub)
		return t(
			"devices:keys.tab.chain.accessUnlock",
			"Unlock to check your permissions",
		);
	return mine || accessRefused(access)
		? t("devices:keys.tab.chain.accessNone", "No active permissions")
		: t("devices:keys.tab.chain.notLoaded", "Not loaded yet");
}

/** The hub answers every ended, removed or lapsed access with one refusal. */
function accessRefused(access: HubRead<MyAccess>): boolean {
	return access.error?.code === "forbidden";
}

function accessState(access: HubRead<MyAccess>, nowS: number): ChainState {
	const mine = access.data;
	if (!mine) return accessRefused(access) ? "warning" : "unknown";
	const endingSoon = mine.grants.some(
		(grant) => grant.expires_at - nowS < ENDING_SOON_S,
	);
	return endingSoon || !mine.grants.length ? "warning" : "good";
}

function accessLink(copy: ChainCopy, access: HubRead<MyAccess>): ChainLink {
	const text = accessText(copy, access);
	return {
		id: "rules",
		icon: Users,
		title: copy.t("devices:keys.tab.chain.access", "Your access"),
		state: accessState(access, copy.time.nowS),
		text,
	};
}

type CertificateState = ReturnType<typeof certificateStatus>;

function certificatesText(
	copy: ChainCopy,
	read: HubRead<PublicCertificateInventory>,
	next: { not_after: number } | undefined,
	status: CertificateState | undefined,
): string {
	const { t, time, day } = copy;
	const inventory = read.data;
	if (!inventory)
		return read.error
			? t(
					"devices:keys.tab.chain.certsNoAccess",
					"Not readable · needs whole-device View status",
				)
			: t("devices:keys.tab.chain.notLoaded", "Not loaded yet");
	if (inventory.updated_at === null)
		return t("devices:keys.tab.chain.certsNever", "Not reported yet");
	if (!next) return t("devices:keys.tab.chain.certsNone", "None on the device");
	const count = inventory.certificates.length;
	if (status === "expired")
		return t(
			"devices:keys.tab.chain.certsExpired",
			"{{count, number}} · one expired {{date}}",
			{ count, date: day(next.not_after) },
		);
	if (status === "expiring")
		return t(
			"devices:keys.tab.chain.certsExpiring",
			"{{count, number}} · the next expires {{in}}",
			{ count, in: time.ago(next.not_after, "long") },
		);
	return t(
		"devices:keys.tab.chain.certsValid",
		"{{count, number}} · all valid · next {{date}}",
		{ count, date: day(next.not_after) },
	);
}

function certificatesLink(
	copy: ChainCopy,
	read: HubRead<PublicCertificateInventory>,
): ChainLink {
	const list = read.data?.certificates ?? [];
	const next = list.length
		? list.reduce((soonest, entry) =>
				entry.not_after < soonest.not_after ? entry : soonest,
			)
		: undefined;
	const status = next ? certificateStatus(next, copy.time.nowS) : undefined;
	const state: ChainState =
		status === "expired" || status === "expiring"
			? "warning"
			: status === "valid"
				? "good"
				: "unknown";
	const text = certificatesText(copy, read, next, status);
	return {
		id: "certificates",
		icon: FileBadge,
		title: copy.t("devices:keys.tab.chain.certs", "Certificates"),
		state,
		text,
	};
}

function identityJoin(
	{ t, day }: ChainCopy,
	pinnedAt: number | undefined,
): string {
	return pinnedAt === undefined
		? t(
				"devices:keys.tab.chain.joinUnpinned",
				"this computer checks the device's keys the first time you unlock",
			)
		: t(
				"devices:keys.tab.chain.joinPinned",
				"you trusted this device's keys on {{date}}",
				{ date: day(Math.floor(pinnedAt / 1000)) },
			);
}

function rulesJoin(
	{ t }: ChainCopy,
	owner: boolean,
	view: PolicyView | undefined,
): string {
	if (!owner)
		return t(
			"devices:keys.tab.chain.joinShared",
			"enforces access rules signed with the owner's key",
		);
	return view && view.version > 0
		? t(
				"devices:keys.tab.chain.joinRules",
				"enforces access rules v{{version}}, signed with your owner key",
				{ version: view.applied_version },
			)
		: t(
				"devices:keys.tab.chain.joinNoRules",
				"has no access rules: nobody else can manage it",
			);
}

/** SPEC §4.28: keys → identity → access rules → certificates, each joined by why it trusts the next. */
export function useTrustChain(
	device: DeviceRow,
	row: KeyRow | undefined,
): ChainLink[] {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const workspace = useDeviceWorkspace();
	const deviceId = device.device_id;
	const owner = row?.relationship === "owner";
	const policy = usePolicy(owner ? deviceId : undefined);
	const access = useMyAccess(row && !owner ? deviceId : undefined);
	const certificates = useCertificateInventory(deviceId);
	const copy: ChainCopy = {
		t,
		time,
		day: (atS) => dayLabel(atS, time.now, time.locale),
	};
	const vault = row?.vault;
	const check = workspace.local.identityCheck(deviceId, device.identity);
	return [
		{
			...keysLink(copy, row),
			join: identityJoin(copy, vault?.identityPinnedAt),
		},
		{
			...identityLink(copy, check, vault?.identityFingerprint),
			join: rulesJoin(copy, owner, policy.data),
		},
		{
			...(owner
				? ownerRulesLink(copy, policy, device)
				: accessLink(copy, access)),
			join: owner
				? t(
						"keys.tab.chain.joinCertsOwner",
						"certificates on the device are managed with whole-device access",
					)
				: t(
						"keys.tab.chain.joinCertsShared",
						"certificate details need whole-device View status",
					),
		},
		certificatesLink(copy, certificates),
	];
}
