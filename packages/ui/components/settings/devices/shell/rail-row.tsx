"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CircleAlert,
	KeyRound,
	Lock,
	LockOpen,
	type LucideIcon,
	OctagonX,
	Radio,
	TriangleAlert,
} from "lucide-react";
import type { MouseEvent } from "react";
import type {
	AttentionKey,
	DevicesRoute,
} from "../../../../lib/device-management/model/types";
import type { DevicesT } from "../primitives/area-context";
import {
	PresenceGlyph,
	type PresenceGlyphKind,
} from "../primitives/presence-glyph";
import { cx } from "../primitives/tone";

export interface NavigateOptions {
	replace?: boolean;
}

/** How every chrome part navigates: the shape of `useDevicesRoute().navigate`. */
export type ChromeNavigate = (
	route: DevicesRoute,
	options?: NavigateOptions,
) => void;

/** The key glyph of a rail row: live connection, open keys, closed keys, or none usable here. */
export type RailKeyState = "live" | "unlocked" | "locked" | "stale" | "none";

export interface RailCounts {
	critical?: number;
	warning?: number;
	notice?: number;
}

export interface RailRowProps {
	name: string;
	href: string;
	/** Router navigation; the row stays a real link for "open in new tab". */
	onSelect?: (event: MouseEvent<HTMLAnchorElement>) => void;
	presence: PresenceGlyphKind;
	/** "Offline since 11:00". */
	presenceLabel?: string;
	/** Open items by severity; only non-zero counts render. */
	counts?: RailCounts;
	keyState?: RailKeyState;
	/** Top reason first, then a short state, then the agent version. */
	sub?: string;
	/** "package" for unused setup packages. */
	tag?: string;
	current?: boolean;
	dim?: boolean;
	className?: string;
}

/** A plain left click goes through the router; modified clicks keep the browser's behaviour (new tab). */
export function plainClick(event: MouseEvent): boolean {
	const modified =
		event.metaKey || event.ctrlKey || event.shiftKey || event.altKey;
	return !event.defaultPrevented && event.button === 0 && !modified;
}

const KEY_LOOK: Record<RailKeyState, { icon: LucideIcon; tone: string }> = {
	live: { icon: Radio, tone: "text-good" },
	unlocked: { icon: LockOpen, tone: "text-info" },
	locked: { icon: Lock, tone: "text-locked" },
	stale: { icon: KeyRound, tone: "text-muted-foreground" },
	none: { icon: KeyRound, tone: "text-muted-foreground" },
};

function keyLabel(t: DevicesT, state: RailKeyState) {
	const labels = {
		live: t("devices:chrome.rail.keyLive", "Live connection"),
		unlocked: t("devices:chrome.rail.keyUnlocked", "Unlocked"),
		locked: t("devices:chrome.rail.keyLocked", "Locked"),
		stale: t("devices:chrome.rail.keyStale", "Unusable keys here"),
		none: t("devices:chrome.rail.keyNone", "No keys here"),
	} satisfies Record<RailKeyState, string>;
	return labels[state];
}

/** The top open item in a few words, for the row's reason line and the fleet table (SPEC §3.4, §4.13). */
const REASONS = {
	offline_since: (t) => t("devices:chrome.rail.reason.offlineSince", "Offline"),
	late: (t) => t("devices:chrome.rail.reason.late", "Check-in late"),
	no_heartbeat_since_enrollment: (t) =>
		t(
			"devices:chrome.rail.reason.noHeartbeatSinceEnrollment",
			"Never checked in",
		),
	revoked: (t) => t("devices:chrome.rail.reason.revoked", "Revoked"),
	you_still_pay_for_a_revoked_device: (t) =>
		t(
			"devices:chrome.rail.reason.youStillPayForARevokedDevice",
			"Still billed to you",
		),
	identity_mismatch: (t) =>
		t("devices:chrome.rail.reason.identityMismatch", "Identity changed"),
	snapshot_integrity_error: (t) =>
		t(
			"devices:chrome.rail.reason.snapshotIntegrityError",
			"Status check failed",
		),
	clock_skew: (t) => t("devices:chrome.rail.reason.clockSkew", "Clock is off"),
	access_denied: (t) =>
		t("devices:chrome.rail.reason.accessDenied", "Access denied"),
	status_stale_while_online: (t) =>
		t("devices:chrome.rail.reason.statusStaleWhileOnline", "Status is stale"),
	background_task_failing: (t) =>
		t(
			"devices:chrome.rail.reason.backgroundTaskFailing",
			"Background task failing",
		),
	agent_update_available: (t) =>
		t(
			"devices:chrome.rail.reason.agentUpdateAvailable",
			"Agent update available",
		),
	rebooted_unexpectedly: (t) =>
		t(
			"devices:chrome.rail.reason.rebootedUnexpectedly",
			"Restarted unexpectedly",
		),
	status_subscription_expiring: (t) =>
		t(
			"devices:chrome.rail.reason.statusSubscriptionExpiring",
			"Status subscription ending",
		),
	device_slots_nearly_full: (t) =>
		t(
			"devices:chrome.rail.reason.deviceSlotsNearlyFull",
			"Device slots nearly full",
		),
	pending_setup_waiting: (t) =>
		t("devices:chrome.rail.reason.pendingSetupWaiting", "Setup waiting"),
	pending_setup_expired: (t) =>
		t("devices:chrome.rail.reason.pendingSetupExpired", "Setup expired"),
	hub_not_ready: (t) =>
		t("devices:chrome.rail.reason.hubNotReady", "Hub not ready"),
	release_trust_missing: (t) =>
		t(
			"devices:chrome.rail.reason.releaseTrustMissing",
			"Release check missing",
		),
	keys_missing_here: (t) =>
		t("devices:chrome.rail.reason.keysMissingHere", "No keys here"),
	keys_not_backed_up_to_account: (t) =>
		t(
			"devices:chrome.rail.reason.keysNotBackedUpToAccount",
			"Keys not backed up",
		),
	account_backup_upload_pending: (t) =>
		t(
			"devices:chrome.rail.reason.accountBackupUploadPending",
			"Backup upload pending",
		),
	account_backup_old_password: (t) =>
		t(
			"devices:chrome.rail.reason.accountBackupOldPassword",
			"Backup has old password",
		),
	account_backup_hub_newer: (t) =>
		t(
			"devices:chrome.rail.reason.accountBackupHubNewer",
			"Account backup is newer",
		),
	storage_not_persistent: (t) =>
		t("devices:chrome.rail.reason.storageNotPersistent", "Keys may be deleted"),
	request_keys_unbacked: (t) =>
		t(
			"devices:chrome.rail.reason.requestKeysUnbacked",
			"Request keys not saved",
		),
	backup_slots_nearly_full: (t) =>
		t(
			"devices:chrome.rail.reason.backupSlotsNearlyFull",
			"Backup slots nearly full",
		),
	stale_local_keys: (t) =>
		t("devices:chrome.rail.reason.staleLocalKeys", "Unusable keys"),
	shared_access_expiring: (t) =>
		t(
			"devices:chrome.rail.reason.sharedAccessExpiring",
			"Your access ends soon",
		),
	shared_access_ended: (t) =>
		t("devices:chrome.rail.reason.sharedAccessEnded", "Your access ended"),
	grant_expiring: (t) =>
		t("devices:chrome.rail.reason.grantExpiring", "Shared access ends soon"),
	sharing_policy_waiting_for_device: (t) =>
		t(
			"devices:chrome.rail.reason.sharingPolicyWaitingForDevice",
			"Access change waiting",
		),
	sharing_policy_expiring: (t) =>
		t(
			"devices:chrome.rail.reason.sharingPolicyExpiring",
			"Access rules expiring",
		),
	sharing_policy_expired: (t) =>
		t(
			"devices:chrome.rail.reason.sharingPolicyExpired",
			"Access rules expired",
		),
	access_slots_nearly_full: (t) =>
		t(
			"devices:chrome.rail.reason.accessSlotsNearlyFull",
			"Access slots nearly full",
		),
	access_request_pending: (t) =>
		t(
			"devices:chrome.rail.reason.accessRequestPending",
			"Access request pending",
		),
	code_running_access_without_sandbox: (t) =>
		t(
			"devices:chrome.rail.reason.codeRunningAccessWithoutSandbox",
			"Code access without sandbox",
		),
	service_crash_looping: (t) =>
		t("devices:chrome.rail.reason.serviceCrashLooping", "Crashing"),
	service_not_as_requested: (t) =>
		t("devices:chrome.rail.reason.serviceNotAsRequested", "Not as requested"),
	service_settings_not_applied: (t) =>
		t(
			"devices:chrome.rail.reason.serviceSettingsNotApplied",
			"Settings not applied",
		),
	service_degraded: (t) =>
		t("devices:chrome.rail.reason.serviceDegraded", "Degraded"),
	rollout_in_progress: (t) =>
		t("devices:chrome.rail.reason.rolloutInProgress", "Updating"),
	rollout_failed_service_stopped: (t) =>
		t(
			"devices:chrome.rail.reason.rolloutFailedServiceStopped",
			"Update failed",
		),
	rollout_rolled_back: (t) =>
		t("devices:chrome.rail.reason.rolloutRolledBack", "Update rolled back"),
	rollout_not_applied: (t) =>
		t("devices:chrome.rail.reason.rolloutNotApplied", "Update not applied"),
	rollout_staged_waiting: (t) =>
		t("devices:chrome.rail.reason.rolloutStagedWaiting", "Update staged"),
	secret_write_pending: (t) =>
		t("devices:chrome.rail.reason.secretWritePending", "Secret pending"),
	secret_write_failed: (t) =>
		t("devices:chrome.rail.reason.secretWriteFailed", "Secret not saved"),
	endpoint_unencrypted_exposed: (t) =>
		t(
			"devices:chrome.rail.reason.endpointUnencryptedExposed",
			"Endpoint not encrypted",
		),
	event_tokens_after_revoke: (t) =>
		t(
			"devices:chrome.rail.reason.eventTokensAfterRevoke",
			"Event tokens still valid",
		),
	schedule_held: (t) =>
		t("devices:chrome.rail.reason.scheduleHeld", "Schedule not running here"),
	schedule_failed: (t) =>
		t("devices:chrome.rail.reason.scheduleFailed", "Scheduled run failed"),
	schedule_once_missed: (t) =>
		t(
			"devices:chrome.rail.reason.scheduleOnceMissed",
			"One-time schedule missed",
		),
	bot_held: (t) => t("devices:chrome.rail.reason.botHeld", "Bot not connected"),
	bot_token_refused: (t) =>
		t("devices:chrome.rail.reason.botTokenRefused", "Bot token refused"),
	bot_intents_refused: (t) =>
		t(
			"devices:chrome.rail.reason.botIntentsRefused",
			"Bot permissions refused",
		),
	bot_conflict: (t) =>
		t("devices:chrome.rail.reason.botConflict", "Bot used elsewhere"),
	offline_writes_conflict: (t) =>
		t(
			"devices:chrome.rail.reason.offlineWritesConflict",
			"Buffered changes conflict",
		),
	offline_writes_blocked: (t) =>
		t(
			"devices:chrome.rail.reason.offlineWritesBlocked",
			"Buffered changes blocked",
		),
	offline_writes_outcome_unknown: (t) =>
		t(
			"devices:chrome.rail.reason.offlineWritesOutcomeUnknown",
			"Buffered change unconfirmed",
		),
	offline_writes_quarantined: (t) =>
		t(
			"devices:chrome.rail.reason.offlineWritesQuarantined",
			"Buffered changes paused",
		),
	offline_writes_backlog: (t) =>
		t(
			"devices:chrome.rail.reason.offlineWritesBacklog",
			"Buffered changes piling up",
		),
	offline_mirror_error: (t) =>
		t("devices:chrome.rail.reason.offlineMirrorError", "Offline copy failing"),
	cloud_access_invalid: (t) =>
		t("devices:chrome.rail.reason.cloudAccessInvalid", "Cloud access invalid"),
	cloud_access_ending: (t) =>
		t("devices:chrome.rail.reason.cloudAccessEnding", "Cloud access ending"),
	spending_limit_low: (t) =>
		t("devices:chrome.rail.reason.spendingLimitLow", "Spending limit low"),
	spending_limit_ending: (t) =>
		t(
			"devices:chrome.rail.reason.spendingLimitEnding",
			"Spending limit ending",
		),
	online_files_read_only: (t) =>
		t(
			"devices:chrome.rail.reason.onlineFilesReadOnly",
			"Online files read-only",
		),
	certificate_expired: (t) =>
		t("devices:chrome.rail.reason.certificateExpired", "Certificate expired"),
	certificate_expiring: (t) =>
		t("devices:chrome.rail.reason.certificateExpiring", "Certificate expiring"),
	certificate_not_yet_valid: (t) =>
		t(
			"devices:chrome.rail.reason.certificateNotYetValid",
			"Certificate not valid yet",
		),
	renewal_delegation_error: (t) =>
		t("devices:chrome.rail.reason.renewalDelegationError", "Renewal failing"),
	renewal_authority_expiring: (t) =>
		t(
			"devices:chrome.rail.reason.renewalAuthorityExpiring",
			"Renewal authority expiring",
		),
	acme_error: (t) =>
		t("devices:chrome.rail.reason.acmeError", "Let's Encrypt renewal failing"),
	acme_staging_in_use: (t) =>
		t("devices:chrome.rail.reason.acmeStagingInUse", "Test certificate in use"),
	signing_request_attention: (t) =>
		t(
			"devices:chrome.rail.reason.signingRequestAttention",
			"Signing request open",
		),
	certificate_inventory_stale: (t) =>
		t(
			"devices:chrome.rail.reason.certificateInventoryStale",
			"Certificate report stale",
		),
	certificate_slots_nearly_full: (t) =>
		t(
			"devices:chrome.rail.reason.certificateSlotsNearlyFull",
			"Certificate slots nearly full",
		),
	org_ca_signing_key_expiring: (t) =>
		t(
			"devices:chrome.rail.reason.orgCaSigningKeyExpiring",
			"Authority key expiring",
		),
	org_ca_root_expiring: (t) =>
		t(
			"devices:chrome.rail.reason.orgCaRootExpiring",
			"Authority root expiring",
		),
	history_paused_readers_expired: (t) =>
		t(
			"devices:chrome.rail.reason.historyPausedReadersExpired",
			"History paused",
		),
	history_paused_access_changed: (t) =>
		t(
			"devices:chrome.rail.reason.historyPausedAccessChanged",
			"History paused",
		),
	history_not_stored_by_plan: (t) =>
		t(
			"devices:chrome.rail.reason.historyNotStoredByPlan",
			"History not stored",
		),
	history_storage_nearly_full: (t) =>
		t(
			"devices:chrome.rail.reason.historyStorageNearlyFull",
			"History storage nearly full",
		),
	metric_readers_expiring: (t) =>
		t(
			"devices:chrome.rail.reason.metricReadersExpiring",
			"Shared metrics ending",
		),
	unconfirmed_command: (t) =>
		t("devices:chrome.rail.reason.unconfirmedCommand", "Command unconfirmed"),
	device_operation_failed: (t) =>
		t(
			"devices:chrome.rail.reason.deviceOperationFailed",
			"Device operation failed",
		),
	device_operation_unknown: (t) =>
		t(
			"devices:chrome.rail.reason.deviceOperationUnknown",
			"Device operation unconfirmed",
		),
	upload_paused: (t) =>
		t("devices:chrome.rail.reason.uploadPaused", "Upload paused"),
} satisfies Record<AttentionKey, (t: DevicesT) => string>;

export function attentionShort(t: DevicesT, key: AttentionKey): string {
	return REASONS[key](t);
}

interface GlyphProps {
	icon: LucideIcon;
	tone: string;
	label: string;
	count?: number;
}

function Glyph({ icon: Icon, tone, label, count }: Readonly<GlyphProps>) {
	return (
		<span
			title={label}
			className={cx(
				"inline-flex items-center gap-0.5 font-mono text-xs font-medium tabular-nums",
				tone,
			)}
		>
			<Icon aria-hidden className="size-3" />
			{count === undefined ? null : <span aria-hidden>{count}</span>}
			<span className="sr-only">{label}</span>
		</span>
	);
}

interface RailGlyphsProps {
	counts?: RailCounts;
	keyState?: RailKeyState;
}

function RailGlyphs({ counts, keyState }: Readonly<RailGlyphsProps>) {
	const { t } = useTranslation("devices");
	const critical = counts?.critical ?? 0;
	const warning = counts?.warning ?? 0;
	const notice = counts?.notice ?? 0;
	const key = keyState ? KEY_LOOK[keyState] : null;
	return (
		<span data-rail-glyphs="" className="flex items-center gap-1.5">
			{critical > 0 ? (
				<Glyph
					icon={OctagonX}
					tone="text-critical"
					count={critical}
					label={t("chrome.rail.critical", "{{count, number}} critical", {
						count: critical,
					})}
				/>
			) : null}
			{warning > 0 ? (
				<Glyph
					icon={TriangleAlert}
					tone="text-warning"
					count={warning}
					label={t("chrome.rail.warning", "{{count, number}} warning", {
						count: warning,
					})}
				/>
			) : null}
			{notice > 0 ? (
				<Glyph
					icon={CircleAlert}
					tone="text-info"
					count={notice}
					label={t("chrome.rail.notice", "{{count, number}} notice", {
						count: notice,
					})}
				/>
			) : null}
			{key && keyState ? (
				<Glyph icon={key.icon} tone={key.tone} label={keyLabel(t, keyState)} />
			) : null}
		</span>
	);
}

/** SPEC §3.4 rail row: presence shape, mono name, severity counts, key glyph, one reason line. */
export function RailRow(props: Readonly<RailRowProps>) {
	const { name, sub, tag } = props;
	return (
		<a
			href={props.href}
			onClick={props.onSelect}
			aria-current={props.current ? "page" : undefined}
			data-rail-row=""
			data-dim={props.dim ? "" : undefined}
			className={cx(
				"grid h-11 grid-cols-[14px_minmax(0,1fr)_auto] content-center items-center gap-x-2 rounded-lg px-2 text-ui text-foreground no-underline hover:bg-row-hover focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring",
				props.current && "bg-row-selected",
				props.dim && "opacity-55",
				props.className,
			)}
		>
			<PresenceGlyph kind={props.presence} label={props.presenceLabel} />
			<span className="flex min-w-0 items-center gap-1">
				<span
					title={name}
					className="truncate font-mono text-[12.5px]/[18px] font-semibold"
				>
					{name}
				</span>
				{tag ? (
					<span className="shrink-0 rounded-sm border border-dashed border-border-strong px-1 font-mono text-label font-medium text-muted-foreground">
						{tag}
					</span>
				) : null}
			</span>
			<RailGlyphs counts={props.counts} keyState={props.keyState} />
			{sub ? (
				<span
					title={sub}
					className="col-span-2 col-start-2 truncate text-xs text-muted-foreground"
				>
					{sub}
				</span>
			) : null}
		</a>
	);
}
