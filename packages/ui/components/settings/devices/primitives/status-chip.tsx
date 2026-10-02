"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AppWindow,
	CircleCheck,
	CircleDashed,
	CirclePause,
	Cloud,
	CloudDownload,
	CloudOff,
	CloudUpload,
	Fingerprint,
	KeyRound,
	LoaderCircle,
	Lock,
	LockOpen,
	type LucideIcon,
	OctagonX,
	Radio,
	RefreshCw,
	Share2,
	SquareX,
	TriangleAlert,
	User,
	Users,
} from "lucide-react";
import type { ComponentProps, ReactNode } from "react";
import { Badge } from "../../../ui/badge";
import { type DevicesT, useAreaTime } from "./area-context";
import { PresenceGlyph } from "./presence-glyph";
import { type ChipTone, TONE_CHIP, cx } from "./tone";

export interface StatusChipProps
	extends Omit<ComponentProps<"span">, "children"> {
	tone: ChipTone;
	icon?: LucideIcon;
	/** A non-Lucide leading glyph (presence shape). Wins over `icon`. */
	glyph?: ReactNode;
	spin?: boolean;
	/** A long label takes a second line instead of being cut (narrow columns); the chip gets 4 px corners. */
	wrap?: boolean;
	children: ReactNode;
}

/** SPEC §4.2: 22 px pill (critical square-cornered, unknown dashed). Link chips wrap it in an `<a>`. */
export function StatusChip({
	tone,
	icon: Icon,
	glyph,
	spin = false,
	wrap = false,
	className,
	children,
	...props
}: StatusChipProps) {
	return (
		<Badge
			variant="outline"
			data-tone={tone}
			className={cx(
				"h-5.5 max-w-full gap-1.25 border px-2 py-0 align-middle text-xs font-medium [&>svg]:size-3.25",
				TONE_CHIP[tone],
				wrap &&
					"h-auto min-h-5.5 items-start rounded-md py-0.5 whitespace-normal [&>svg]:mt-0.5",
				className,
			)}
			{...props}
		>
			{glyph ??
				(Icon ? (
					<Icon aria-hidden className={spin ? "animate-spin" : undefined} />
				) : null)}
			<span className={wrap ? "min-w-0" : "min-w-0 truncate"}>{children}</span>
		</Badge>
	);
}

interface ChipLook {
	tone: ChipTone;
	icon: LucideIcon;
	spin?: boolean;
}

export type HealthChipLevel =
	| "critical"
	| "attention"
	| "healthy"
	| "unknown"
	| "revoked";

const HEALTH_LOOK: Record<HealthChipLevel, ChipLook> = {
	critical: { tone: "critical", icon: OctagonX },
	attention: { tone: "warning", icon: TriangleAlert },
	healthy: { tone: "good", icon: CircleCheck },
	unknown: { tone: "unknown", icon: CircleDashed },
	revoked: { tone: "outline", icon: SquareX },
};

export function healthLabel(t: DevicesT, level: HealthChipLevel): string {
	const labels = {
		critical: t("devices:enum.health.critical", "Critical"),
		attention: t("devices:enum.health.attention", "Needs attention"),
		healthy: t("devices:enum.health.healthy", "Healthy"),
		unknown: t("devices:enum.health.unknown", "Status unknown"),
		revoked: t("devices:enum.health.revoked", "Revoked"),
	} satisfies Record<HealthChipLevel, string>;
	return labels[level];
}

export function HealthChip({
	level,
	count,
	className,
}: Readonly<{ level: HealthChipLevel; count?: number; className?: string }>) {
	const { t } = useTranslation("devices");
	const look = HEALTH_LOOK[level];
	const label = healthLabel(t, level);
	return (
		<StatusChip
			tone={look.tone}
			icon={look.icon}
			data-health={level}
			className={className}
		>
			{count
				? t("common.chip.withCount", "{{label}} · {{count, number}}", {
						label,
						count,
					})
				: label}
		</StatusChip>
	);
}

export type PresenceChipKind =
	| "online"
	| "late"
	| "offline"
	| "never"
	| "revoked";

const PRESENCE_TONE: Record<PresenceChipKind, ChipTone> = {
	online: "good",
	late: "warning",
	offline: "critical",
	never: "unknown",
	revoked: "outline",
};

export function PresenceChip({
	kind,
	since,
	short = false,
	className,
}: Readonly<{
	kind: PresenceChipKind;
	/** Unix seconds of the last check-in. */
	since?: number;
	/** "Online · 41 s ago" instead of "Online · checked in 41 s ago". */
	short?: boolean;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const ago = since === undefined ? "" : time.ago(Math.min(since, time.nowS));
	const text: Record<PresenceChipKind, string> = {
		online:
			since === undefined
				? t("enum.presence.online", "Online")
				: short
					? t("common.presence.onlineShort", "Online · {{ago}}", { ago })
					: t("common.presence.onlineLong", "Online · checked in {{ago}}", {
							ago,
						}),
		late:
			since === undefined
				? t("enum.presence.late", "Late")
				: t("common.presence.lateAgo", "Late · {{ago}}", { ago }),
		offline:
			since === undefined
				? t("enum.presence.offline", "Offline")
				: t("common.presence.offlineSince", "Offline since {{time}}", {
						time: time.at(since),
					}),
		never: t("enum.presence.never", "Never checked in"),
		revoked: t("enum.deviceStatus.revoked", "Revoked"),
	};
	const title =
		since === undefined || kind === "never" || kind === "revoked"
			? undefined
			: t(
					"common.presence.title",
					"Last check-in {{at}}. A check-in is a signed “I'm here” the device sends about once a minute. It doesn't prove a live connection works.",
					{ at: time.abs(since) },
				);
	return (
		<StatusChip
			tone={PRESENCE_TONE[kind]}
			glyph={<PresenceGlyph kind={kind} decorative />}
			title={title}
			data-presence={kind}
			className={className}
		>
			{text[kind]}
		</StatusChip>
	);
}

/** Superset of the key session states plus the live connection on top of an unlocked session. */
export type KeyChipState =
	| "live"
	| "reconnecting"
	| "unlocking"
	| "unlocked"
	| "held_elsewhere"
	| "blocked"
	| "locked"
	| "stale"
	| "none";

const KEY_LOOK: Record<KeyChipState, ChipLook> = {
	live: { tone: "good", icon: Radio },
	reconnecting: { tone: "info", icon: LoaderCircle, spin: true },
	unlocking: { tone: "info", icon: LoaderCircle, spin: true },
	unlocked: { tone: "info", icon: LockOpen },
	held_elsewhere: { tone: "locked", icon: AppWindow },
	blocked: { tone: "critical", icon: Fingerprint },
	locked: { tone: "locked", icon: Lock },
	stale: { tone: "outline", icon: KeyRound },
	none: { tone: "outline", icon: KeyRound },
};

export function KeyChip({
	state,
	transport,
	renewsAt,
	className,
}: Readonly<{
	state: KeyChipState;
	transport?: "direct" | "relayed";
	/** Unix seconds of the next connection renewal (direct only). */
	renewsAt?: number;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const relayed = transport === "relayed";
	const live = relayed
		? t("common.key.liveRelayed", "Live · relayed")
		: renewsAt === undefined
			? t("common.key.liveDirect", "Live · direct")
			: t("common.key.liveDirectRenews", "Live · direct · renews in {{in}}", {
					in: time.countdown(renewsAt),
				});
	const copy: Record<KeyChipState, { text: string; title?: string }> = {
		live: {
			text: live,
			title: relayed
				? t(
						"common.key.liveRelayedTitle",
						"Relayed through the hub. Both connection kinds are end-to-end encrypted. The connection renews itself every 5 minutes while the keys stay unlocked.",
					)
				: t(
						"common.key.liveDirectTitle",
						"Direct connection. Both connection kinds are end-to-end encrypted. The connection renews itself every 5 minutes while the keys stay unlocked.",
					),
		},
		reconnecting: {
			text: t("common.key.reconnecting", "Reconnecting… keys still unlocked"),
		},
		unlocking: { text: t("common.key.unlocking", "Unlocking…") },
		unlocked: {
			text: t("enum.vaultState.unlocked", "Unlocked"),
			title: t(
				"common.key.unlockedTitle",
				"Keys are open on this computer. Encrypted status is readable; there is no live connection.",
			),
		},
		held_elsewhere: {
			text: t("enum.lockHolder.anotherWindow", "Unlocked in another window"),
		},
		// The key manager blocks a session only when the hub reports another identity than the trusted one.
		blocked: {
			text: t("common.key.identityChanged", "Identity changed"),
			title: t(
				"common.key.identityChangedTitle",
				"The hub reports other keys than the ones trusted on this computer. The keys here stay closed until you confirm the identity.",
			),
		},
		locked: {
			text: t("enum.vaultState.locked", "Locked"),
			title: t(
				"common.key.lockedTitle",
				"Keys for this device are on this computer but closed. Unlock with the device password.",
			),
		},
		stale: {
			text: t("common.key.stale", "Unusable keys here"),
			title: t(
				"common.key.staleTitle",
				"The keys stored here belong to a revoked device and can't be used.",
			),
		},
		none: {
			text: t("enum.vaultState.absent", "No keys here"),
			title: t(
				"common.key.noneTitle",
				"This computer has no keys for this device.",
			),
		},
	};
	const look = KEY_LOOK[state];
	return (
		<StatusChip
			tone={look.tone}
			icon={look.icon}
			spin={look.spin}
			title={copy[state].title}
			data-key-state={state}
			className={className}
		>
			{copy[state].text}
		</StatusChip>
	);
}

export type BackupChipState =
	| "never"
	| "in_sync"
	| "upload_pending"
	| "local_changes"
	| "hub_newer";

const BACKUP_LOOK: Record<BackupChipState, ChipLook> = {
	never: { tone: "warning", icon: CloudOff },
	in_sync: { tone: "outline", icon: CloudUpload },
	upload_pending: { tone: "info", icon: LoaderCircle },
	local_changes: { tone: "warning", icon: CloudOff },
	hub_newer: { tone: "info", icon: CloudDownload },
};

/** Keys safety: storage risk first, then the account backup state. */
export function SafetyChip({
	backup,
	revision,
	atRisk = false,
	className,
}: Readonly<{
	backup?: BackupChipState;
	/** Backup revision on the account. */
	revision?: number;
	/** The browser hasn't granted persistent storage. */
	atRisk?: boolean;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	if (atRisk) {
		return (
			<StatusChip
				tone="warning"
				icon={TriangleAlert}
				data-safety="at_risk"
				title={t(
					"common.safety.atRiskTitle",
					"This browser hasn't granted persistent storage. Back up the keys or use the desktop app.",
				)}
				className={className}
			>
				{t("enum.storage.denied", "Browser may delete keys")}
			</StatusChip>
		);
	}
	if (!backup) return null;
	const copy: Record<BackupChipState, { text: string; title?: string }> = {
		never: {
			text: t("enum.backup.never", "Not backed up"),
			title: t(
				"common.safety.neverTitle",
				"The keys exist only on this computer.",
			),
		},
		in_sync: {
			text:
				revision === undefined
					? t("common.safety.backedUp", "Backed up")
					: t("enum.backup.inSync", "Backed up (v{{revision}})", { revision }),
			title: t(
				"common.safety.inSyncTitle",
				"The hub stores it encrypted. You still need the device password to open it.",
			),
		},
		upload_pending: { text: t("enum.backup.uploadPending", "Upload pending") },
		local_changes: {
			text: t("enum.backup.localChanges", "Backup out of date"),
		},
		hub_newer: {
			text: t("enum.backup.hubNewer", "Newer backup on your account"),
		},
	};
	const look = BACKUP_LOOK[backup];
	return (
		<StatusChip
			tone={look.tone}
			icon={look.icon}
			data-safety={backup}
			title={copy[backup].title}
			className={className}
		>
			{copy[backup].text}
		</StatusChip>
	);
}

export type RelationshipChipKind =
	| "owner"
	| "shared"
	| "cloud_approval"
	| "unknown"
	| "none";

/** `none` renders nothing; `unknown` is the older-hub interim (IA BG1). */
export function RelationshipChip({
	relationship,
	ownerName,
	endsAt,
	className,
}: Readonly<{
	relationship: RelationshipChipKind;
	ownerName?: string;
	/** Unix seconds when shared access ends. */
	endsAt?: number;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	if (relationship === "none") return null;
	const shared = ownerName
		? endsAt === undefined
			? t("enum.relationship.sharedBy", "Shared by {{owner}}", {
					owner: ownerName,
				})
			: t(
					"common.relationship.sharedByEnds",
					"Shared by {{owner}} · ends {{ago}}",
					{
						owner: ownerName,
						ago: time.ago(endsAt),
					},
				)
		: t("common.relationship.sharedWithYou", "Shared with you");
	const look: Record<
		Exclude<RelationshipChipKind, "none">,
		{ icon: LucideIcon; text: string; title?: string }
	> = {
		owner: { icon: User, text: t("enum.relationship.owner", "Yours") },
		shared: { icon: Share2, text: shared },
		cloud_approval: {
			icon: Cloud,
			text: t("enum.relationship.cloudApproval", "Cloud approvals only"),
			title: ownerName
				? t(
						"common.relationship.cloudApprovalTitle",
						"Owned by {{owner}}. You see it because you approve or pay for its cloud access.",
						{ owner: ownerName },
					)
				: undefined,
		},
		unknown: {
			icon: Users,
			text: t("enum.relationship.unknown", "Shared or cloud approvals"),
		},
	};
	const row = look[relationship];
	return (
		<StatusChip
			tone="outline"
			icon={row.icon}
			title={row.title}
			data-relationship={relationship}
			className={className}
		>
			{row.text}
		</StatusChip>
	);
}

export type ConvergenceChipKind =
	| "converged"
	| "converging"
	| "crash_looping"
	| "stopped_by_user"
	| "update_in_progress"
	| "failed_stopped"
	| "unknown";

export const CONVERGENCE_LOOK: Record<ConvergenceChipKind, ChipLook> = {
	converged: { tone: "good", icon: CircleCheck },
	converging: { tone: "info", icon: LoaderCircle },
	crash_looping: { tone: "critical", icon: OctagonX },
	stopped_by_user: { tone: "paused", icon: CirclePause },
	update_in_progress: { tone: "info", icon: RefreshCw },
	failed_stopped: { tone: "critical", icon: OctagonX },
	unknown: { tone: "unknown", icon: CircleDashed },
};

export function convergenceLabel(
	t: DevicesT,
	conv: ConvergenceChipKind,
): string {
	const labels = {
		converged: t("devices:enum.convergence.converged", "As requested"),
		converging: t("devices:enum.convergence.converging", "Applying changes"),
		crash_looping: t("devices:enum.convergence.crash_looping", "Crashing"),
		stopped_by_user: t("devices:enum.convergence.stopped_by_user", "Stopped"),
		update_in_progress: t(
			"devices:enum.convergence.update_in_progress",
			"Updating",
		),
		failed_stopped: t(
			"devices:enum.convergence.failed_stopped",
			"Stopped after a failed update",
		),
		unknown: t("devices:enum.convergence.unknown", "Unknown"),
	} satisfies Record<ConvergenceChipKind, string>;
	return labels[conv];
}

export function ConvergenceChip({
	conv,
	className,
}: Readonly<{ conv: ConvergenceChipKind; className?: string }>) {
	const { t } = useTranslation("devices");
	const look = CONVERGENCE_LOOK[conv];
	return (
		<StatusChip
			tone={look.tone}
			icon={look.icon}
			data-conv={conv}
			className={className}
		>
			{convergenceLabel(t, conv)}
		</StatusChip>
	);
}
