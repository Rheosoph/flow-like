import type { PermissionPreset } from "../../../../lib/device-management/model/permissions";
import type {
	AgeState,
	Convergence,
	HealthLevel,
	HostIsolationMode,
	OnlineAccess,
	PresenceKind,
	Relationship,
	Severity,
} from "../../../../lib/device-management/model/types";
import type { Capability } from "../../../../lib/device-management/types";
import type { KeyState } from "../../../../lib/device-management/workspace/types";
import type { DevicesT } from "../primitives/area-context";
import { ageLabel } from "../primitives/freshness-stamp";
import { severityLabel } from "../primitives/severity-word";
import {
	CONVERGENCE_LOOK,
	convergenceLabel,
	healthLabel,
} from "../primitives/status-chip";

/*
 * SPEC §6.2 / IA §6.6.2–§6.6.3: every enum wire value → label (+ one-line
 * explanation, + risk note for capabilities). Wire values never render (R3).
 * Literal `devices:`-prefixed t() calls so the extractor files them under
 * `devices`; `satisfies` makes every family exhaustive over its union.
 */

export type EnumParams = Record<string, string | number>;

export interface EnumCopy {
	label: string;
	explain?: string;
	/** Capability risk note (IA §6.6.2). */
	note?: string;
}

/** Families and their value unions. Contract unions where the model defines one. */
export interface EnumValues {
	deviceStatus: "active" | "revoked";
	presence: PresenceKind;
	relationship: Relationship;
	health: HealthLevel;
	severity: Severity;
	freshness: AgeState;
	hubSupport: "checking" | "on" | "off" | "unreachable";
	enrollment: "pending" | "expired" | "consumed" | "cancelled";
	connectionStatus:
		| "pending"
		| "enrolled"
		| "connected"
		| "disconnected"
		| "access_denied";
	readinessCheck:
		| "policy"
		| "signing"
		| "api"
		| "signaling"
		| "release"
		| "database";
	target:
		| "x86_64-unknown-linux-gnu"
		| "aarch64-unknown-linux-gnu"
		| "x86_64-apple-darwin"
		| "aarch64-apple-darwin";
	targetShort: EnumValues["target"];
	packageMode: "binary" | "docker" | "both";
	releaseVerification: "verified" | "rejected";
	vaultKind: "owner" | "shared";
	vaultState:
		| "absent"
		| "locked"
		| "unlocked"
		| "restored"
		| "onboarding"
		| "conflict";
	keyState: KeyState;
	lockHolder: "free" | "this_window" | "another_window" | "unsupported";
	storage: "persisted" | "denied" | "unavailable";
	backup:
		| "never"
		| "in_sync"
		| "upload_pending"
		| "local_changes"
		| "hub_newer";
	setupBackup: "saved" | "local_only" | "limit";
	session:
		| "idle"
		| "admitting"
		| "relaying"
		| "webrtc-negotiating"
		| "handshaking"
		| "ready"
		| "closed";
	transport: "webrtc" | "websocket";
	clientError: "not_sent" | "unconfirmed";
	fleetIntegrity:
		| "rollback"
		| "missing_stream"
		| "policy_rollback"
		| "identity_mismatch";
	scopeKind: "device" | "project" | "placement";
	grant: "requested" | "active" | "expired" | "revoked";
	accessRules: "none" | "applied" | "waiting" | "expired";
	capability: Capability;
	preset: PermissionPreset | "custom";
	responseState:
		| "completed"
		| "accepted"
		| "pending"
		| "rejected"
		| "failed"
		| "staging"
		| "draining"
		| "requesting"
		| "requested"
		| "unknown"
		| "rolled_back";
	rejection:
		| "unauthorized"
		| "revision_conflict"
		| "invalid"
		| "host_policy"
		| "unsupported"
		| "limit"
		| "busy"
		| "failed";
	hostOpKind: "reboot" | "update_agent";
	rebootState:
		| "pending"
		| "draining"
		| "requesting"
		| "requested"
		| "completed"
		| "failed"
		| "unknown";
	agentUpdateState:
		| "pending"
		| "staging"
		| "draining"
		| "requesting"
		| "completed"
		| "rolled_back"
		| "failed"
		| "unknown";
	updateJournal:
		| "staged"
		| "armed"
		| "swapped"
		| "completed"
		| "rolled_back"
		| "failed";
	secretWrite: "pending" | "completed" | "failed";
	desired: "running" | "stopped";
	observed:
		| "unknown"
		| "starting"
		| "running"
		| "stopping"
		| "stopped"
		| "backoff"
		| "failed"
		| "removed";
	convergence: Convergence;
	placementSource: "offline" | "online";
	rollout:
		| "staged"
		| "validating"
		| "activating"
		| "healthy"
		| "rolling_back"
		| "rolled_back"
		| "failed"
		| "cancelled";
	failureCode:
		| "discarded"
		| "superseded"
		| "stopped"
		| "staging_timeout"
		| "validation_failed"
		| "validation_timeout"
		| "activation_timeout"
		| "candidate_failed"
		| "rollback_timeout"
		| "rollback_failed";
	isolation: "trusted_process" | "linux_sandbox";
	hostIsolation: HostIsolationMode | "unknown";
	eventType: "http" | "simple_chat" | "page" | "rest" | "mcp" | "daemon";
	readinessKind: "listener" | "explicit" | "unsupported";
	valueType: "Normal" | "Array" | "HashSet" | "HashMap";
	dataType:
		| "String"
		| "PathBuf"
		| "Boolean"
		| "Integer"
		| "Float"
		| "Byte"
		| "Date";
	transfer: "receiving" | "committed" | "aborted" | "expired";
	transferPhase: "manifest" | "files" | "commit";
	tablePurpose: "storage" | "user";
	filePurpose: "files" | "storage" | "user" | "temporary";
	queuedWrite:
		| "pending"
		| "attempting"
		| "applied"
		| "blocked"
		| "conflict"
		| "outcome_unknown"
		| "skipped"
		| "superseded";
	replayStatus:
		| "applied"
		| "conflict"
		| "outcome_unknown"
		| "unsupported"
		| "blocked";
	replayError:
		| "OFFLINE_INVALID"
		| "OFFLINE_LIMIT_EXCEEDED"
		| "OFFLINE_FORBIDDEN"
		| "OFFLINE_PRINCIPAL_UNSUPPORTED"
		| "OFFLINE_SUBJECT_MISMATCH"
		| "OFFLINE_DIGEST_REUSED";
	queueState: "quarantined";
	approvalStatus: "active" | "revoked" | "expired";
	onlineAccess: OnlineAccess | "none";
	usageAdmission: "reserved" | "running" | "settled" | "unknown";
	instancePurpose: "workload" | "rollout_validation";
	instanceStatus: "active" | "validating" | "retired";
	cpuBasis: "one_logical_cpu";
	memoryBasis: "process_rss" | "cgroup_current";
	ioBasis: "cgroup_block_io" | "process_io" | "observed_placement_workers";
	resourceBasis: "all_host_interfaces" | "agent_state_volume";
	usageWindow:
		| "current_process_lifetimes"
		| "retained_observed_process_checkpoints";
	logKind: "log" | "message";
	logStream: "stdout" | "stderr";
	messageKind: "operation" | "replica";
	archiveKind: "logs" | "metrics";
	archiveRoster: "none" | "current" | "expired" | "stale";
	/** Hub history tier: the free tier stores nothing; plan names come from the hub (params). */
	telemetryTier: "free" | "stored";
	mlsResult:
		| "key_package"
		| "joined"
		| "application"
		| "epoch_changed"
		| "removed"
		| "duplicate";
	validity: "not_yet_valid" | "valid" | "expiring" | "expired";
	certMode: "manual" | "csr-issued" | "delegated" | "acme";
	csrPurpose: "service" | "issuer";
	csrState: "pending" | "installed" | "discarded" | "expired" | "stale";
	delegation: "requested" | "active" | "renewing" | "error" | "disabled";
	acme: "scheduled" | "attempting" | "issued" | "backoff" | "disabled";
	acmeEnvironment: "lets_encrypt_staging" | "lets_encrypt_production";
	orgCa: "pending" | "active" | "issuing_expired" | "root_expired" | "restored";
	noticeStage: "week" | "three_days" | "day" | "expired";
	noticeChannel: "push" | "email";
	noticeStatus: "pending" | "sent" | "cancelled";
}

export type EnumFamily = keyof EnumValues;

type Table<F extends EnumFamily> = Record<EnumValues[F], EnumCopy>;
type Builder<F extends EnumFamily> = (t: DevicesT, p: EnumParams) => Table<F>;

const row = (label: string, explain?: string, note?: string): EnumCopy => ({
	label,
	explain,
	note,
});

const CONVERGENCES = Object.keys(CONVERGENCE_LOOK) as Convergence[];

type Rows<F extends EnumFamily, K extends EnumValues[F]> = Pick<Table<F>, K>;

function observeCapabilities(
	t: DevicesT,
): Rows<"capability", "status" | "logs" | "metrics"> {
	return {
		status: row(
			t("devices:enum.capability.status", "View status"),
			t(
				"devices:enum.capability.statusExplain",
				"See services, their state, versions and certificates in scope.",
			),
		),
		logs: row(
			t("devices:enum.capability.logs", "Read logs"),
			t(
				"devices:enum.capability.logsExplain",
				"Read logs, activity and retained log history.",
			),
			t("devices:enum.capability.logsNote", "Logs may contain data."),
		),
		metrics: row(
			t("devices:enum.capability.metrics", "Read metrics"),
			t(
				"devices:enum.capability.metricsExplain",
				"Read resource use and usage, live and retained.",
			),
		),
	};
}

function serviceCapabilities(
	t: DevicesT,
): Rows<
	"capability",
	| "deploy"
	| "start"
	| "stop"
	| "restart"
	| "remove"
	| "scale"
	| "service_connect"
> {
	const runsCode = t("devices:enum.capability.noteRunsCode", "Runs code.");
	return {
		service_connect: row(
			t("devices:enum.capability.serviceConnect", "Connect to services"),
			t(
				"devices:enum.capability.serviceConnectExplain",
				"Open encrypted connections to configured service listeners in scope.",
			),
			t(
				"devices:enum.capability.serviceConnectNote",
				"Can call service APIs. The service still checks its own credentials.",
			),
		),
		deploy: row(
			t("devices:enum.capability.deploy", "Deploy & configure"),
			t(
				"devices:enum.capability.deployExplain",
				"Upload app versions, change settings and secrets, create cloud access, handle queued changes.",
			),
			t(
				"devices:enum.capability.deployNote",
				"Runs code; reads full service settings.",
			),
		),
		start: row(
			t("devices:enum.capability.start", "Start services"),
			t(
				"devices:enum.capability.startExplain",
				"Start stopped services, activate updates, and run a service's quick actions and forms.",
			),
			runsCode,
		),
		stop: row(
			t("devices:enum.capability.stop", "Stop services"),
			t("devices:enum.capability.stopExplain", "Stop running services."),
			t("devices:enum.capability.stopNote", "Interrupts service."),
		),
		restart: row(
			t("devices:enum.capability.restart", "Restart services"),
			t("devices:enum.capability.restartExplain", "Restart running services."),
			runsCode,
		),
		remove: row(
			t("devices:enum.capability.remove", "Remove services"),
			t(
				"devices:enum.capability.removeExplain",
				"Remove stopped services for good.",
			),
			t("devices:enum.capability.removeNote", "Permanent."),
		),
		scale: row(
			t("devices:enum.capability.scale", "Change instance count"),
			t("devices:enum.capability.scaleExplain", "Set how many instances run."),
			runsCode,
		),
	};
}

function deviceCapabilities(
	t: DevicesT,
): Rows<"capability", "update_agent" | "reboot" | "manage_certificates"> {
	return {
		update_agent: row(
			t("devices:enum.capability.updateAgent", "Update the agent"),
			t(
				"devices:enum.capability.updateAgentExplain",
				"Install a verified agent release (Linux). Whole device only.",
			),
			t("devices:enum.capability.updateAgentNote", "Stops services briefly."),
		),
		reboot: row(
			t("devices:enum.capability.reboot", "Reboot the device"),
			t(
				"devices:enum.capability.rebootExplain",
				"Restart the device (Linux). Whole device only.",
			),
			t("devices:enum.capability.rebootNote", "Stops services."),
		),
		manage_certificates: row(
			t("devices:enum.capability.manageCertificates", "Manage certificates"),
			t(
				"devices:enum.capability.manageCertificatesExplain",
				"Add, replace, delete and assign certificates. Whole device only.",
			),
			t(
				"devices:enum.capability.manageCertificatesNote",
				"Can assign private keys to services.",
			),
		),
	};
}

function modelCapabilities(
	t: DevicesT,
): Rows<"capability", "model_use" | "model_manage"> {
	return {
		model_use: row(
			t("devices:enum.capability.modelUse", "Use models"),
			t(
				"devices:enum.capability.modelUseExplain",
				"Call the models this device hosts from apps and the playground. Whole device only.",
			),
		),
		model_manage: row(
			t("devices:enum.capability.modelManage", "Manage models"),
			t(
				"devices:enum.capability.modelManageExplain",
				"Install, configure, load and remove models and model runtimes. Whole device only.",
			),
			t(
				"devices:enum.capability.modelManageNote",
				"Uses disk, memory and GPU; can unload models others use.",
			),
		),
	};
}

function failuresBeforeSwitch(
	t: DevicesT,
): Rows<
	"failureCode",
	| "discarded"
	| "superseded"
	| "stopped"
	| "staging_timeout"
	| "validation_failed"
	| "validation_timeout"
> {
	const stillRunning = t(
		"devices:enum.failureCode.stillRunning",
		"The current version is still running.",
	);
	return {
		discarded: row(
			t("devices:enum.failureCode.discarded", "Discarded"),
			t(
				"devices:enum.failureCode.discardedExplain",
				"You discarded the update.",
			),
		),
		superseded: row(
			t("devices:enum.failureCode.superseded", "Replaced by a newer change"),
		),
		stopped: row(t("devices:enum.failureCode.stopped", "Service was stopped")),
		staging_timeout: row(
			t("devices:enum.failureCode.stagingTimeout", "Not activated within 24 h"),
			stillRunning,
		),
		validation_failed: row(
			t(
				"devices:enum.failureCode.validationFailed",
				"New version failed its checks",
			),
			stillRunning,
		),
		validation_timeout: row(
			t(
				"devices:enum.failureCode.validationTimeout",
				"New version's checks took too long",
			),
			stillRunning,
		),
	};
}

function failuresAfterSwitch(
	t: DevicesT,
): Rows<
	"failureCode",
	| "activation_timeout"
	| "candidate_failed"
	| "rollback_timeout"
	| "rollback_failed"
> {
	const rolledBack = t(
		"devices:enum.failureCode.rolledBack",
		"The previous version is running again.",
	);
	const stopped = t(
		"devices:enum.failureCode.stoppedExplain",
		"The service is stopped.",
	);
	return {
		activation_timeout: row(
			t(
				"devices:enum.failureCode.activationTimeout",
				"New version didn't become healthy in time",
			),
			rolledBack,
		),
		candidate_failed: row(
			t(
				"devices:enum.failureCode.candidateFailed",
				"New version crashed after switching",
			),
			rolledBack,
		),
		rollback_timeout: row(
			t("devices:enum.failureCode.rollbackTimeout", "Rollback took too long"),
			stopped,
		),
		rollback_failed: row(
			t("devices:enum.failureCode.rollbackFailed", "Rollback failed"),
			stopped,
		),
	};
}

/** Failure codes after which the service is stopped; every other failed update left the current version running. */
const SERVICE_STOPPED_CODES: readonly (string | number | undefined)[] = [
	"rollback_timeout",
	"rollback_failed",
];

function failedRollout(t: DevicesT, p: EnumParams): EnumCopy {
	if (SERVICE_STOPPED_CODES.includes(p.failureCode)) {
		return row(
			t("devices:enum.rollout.failedStopped", "Failed, service stopped"),
			t("devices:enum.failureCode.stoppedExplain", "The service is stopped."),
		);
	}
	return row(
		t("devices:enum.rollout.failed", "Not applied"),
		t(
			"devices:enum.rollout.failedExplain",
			"The failure reason says whether the service is still running.",
		),
	);
}

function storedTierLabel(t: DevicesT, p: EnumParams): string {
	if (p.plan === undefined)
		return t("devices:enum.telemetryTier.stored", "Stored in the cloud");
	if (p.retention === undefined || p.size === undefined) return String(p.plan);
	return t(
		"devices:enum.telemetryTier.storedPlan",
		"{{plan}} · keeps {{retention}}, up to {{size}}",
		p,
	);
}

const BUILDERS: { [F in EnumFamily]: Builder<F> } = {
	deviceStatus: (t) => ({
		active: row(
			t("devices:enum.deviceStatus.active", "Registered"),
			t(
				"devices:enum.deviceStatus.activeExplain",
				"Finished setup and can check in.",
			),
		),
		revoked: row(
			t("devices:enum.deviceStatus.revoked", "Revoked"),
			t(
				"devices:enum.deviceStatus.revokedExplain",
				"Cut off for good; set up again to reuse the hardware.",
			),
		),
	}),
	presence: (t) => ({
		online: row(
			t("devices:enum.presence.online", "Online"),
			t(
				"devices:enum.presence.onlineExplain",
				"Checked in within the last 2 minutes.",
			),
		),
		late: row(
			t("devices:enum.presence.late", "Late"),
			t(
				"devices:enum.presence.lateExplain",
				"Last check-in 2 to 10 minutes ago.",
			),
		),
		offline: row(
			t("devices:enum.presence.offline", "Offline"),
			t(
				"devices:enum.presence.offlineExplain",
				"No check-in for more than 10 minutes.",
			),
		),
		never: row(
			t("devices:enum.presence.never", "Never checked in"),
			t(
				"devices:enum.presence.neverExplain",
				"The device hasn't checked in yet.",
			),
		),
		revoked: row(t("devices:enum.deviceStatus.revoked", "Revoked")),
	}),
	relationship: (t, p) => ({
		owner: row(
			t("devices:enum.relationship.owner", "Yours"),
			t("devices:enum.relationship.ownerExplain", "You set it up."),
		),
		shared: row(
			p.owner === undefined
				? t("devices:common.relationship.sharedWithYou", "Shared with you")
				: t("devices:enum.relationship.sharedBy", "Shared by {{owner}}", p),
			t(
				"devices:enum.relationship.sharedExplain",
				"The owner gave you access.",
			),
		),
		cloud_approval: row(
			t("devices:enum.relationship.cloudApproval", "Cloud approvals only"),
			t(
				"devices:enum.relationship.cloudApprovalExplain",
				"You only approve or pay for its cloud access.",
			),
		),
		unknown: row(
			t("devices:enum.relationship.unknown", "Shared or cloud approvals"),
			t(
				"devices:enum.relationship.unknownExplain",
				"This hub doesn't say whether it is shared with you or you hold its cloud approvals.",
			),
		),
		none: row(t("devices:enum.relationship.none", "No access")),
	}),
	health: (t) => ({
		critical: row(healthLabel(t, "critical")),
		attention: row(healthLabel(t, "attention")),
		healthy: row(healthLabel(t, "healthy")),
		revoked: row(healthLabel(t, "revoked")),
		unknown: row(healthLabel(t, "unknown")),
	}),
	severity: (t) => ({
		critical: row(severityLabel(t, "critical")),
		warning: row(severityLabel(t, "warning")),
		notice: row(severityLabel(t, "notice")),
		info: row(severityLabel(t, "info")),
	}),
	freshness: (t) => ({
		live: row(ageLabel(t, "live")),
		current: row(ageLabel(t, "current")),
		delayed: row(ageLabel(t, "delayed")),
		lastknown: row(ageLabel(t, "lastknown")),
		snapshot: row(ageLabel(t, "snapshot")),
		locked: row(ageLabel(t, "locked")),
		notloaded: row(ageLabel(t, "notloaded")),
		noaccess: row(ageLabel(t, "noaccess")),
		unsupported: row(ageLabel(t, "unsupported")),
		error: row(ageLabel(t, "error")),
	}),
	hubSupport: (t) => ({
		checking: row(t("devices:enum.hubSupport.checking", "Checking…")),
		on: row(t("devices:enum.hubSupport.on", "On")),
		off: row(t("devices:enum.hubSupport.off", "Off on this hub")),
		unreachable: row(
			t("devices:enum.hubSupport.unreachable", "Hub unreachable"),
		),
	}),
	enrollment: (t) => ({
		pending: row(
			t("devices:enum.enrollment.pending", "Waiting for the device"),
			t(
				"devices:enum.enrollment.pendingExplain",
				"Start the package before it expires.",
			),
		),
		expired: row(
			t("devices:enum.enrollment.expired", "Expired"),
			t("devices:enum.enrollment.expiredExplain", "It wasn't used in time."),
		),
		consumed: row(
			t("devices:enum.enrollment.consumed", "Completed"),
			t("devices:enum.enrollment.consumedExplain", "The device registered."),
		),
		cancelled: row(
			t("devices:enum.enrollment.cancelled", "Cancelled"),
			t(
				"devices:enum.enrollment.cancelledExplain",
				"It can't be used anymore.",
			),
		),
	}),
	connectionStatus: (t) => ({
		pending: row(t("devices:enum.connectionStatus.pending", "Not set up")),
		enrolled: row(t("devices:enum.connectionStatus.enrolled", "Registered")),
		connected: row(t("devices:enum.connectionStatus.connected", "Checking in")),
		disconnected: row(
			t("devices:enum.connectionStatus.disconnected", "Can't reach hub"),
		),
		access_denied: row(
			t(
				"devices:enum.connectionStatus.accessDenied",
				"Hub refused this device",
			),
			t(
				"devices:enum.connectionStatus.accessDeniedExplain",
				"The agent stopped trying; run recover-enrollment on the device.",
			),
		),
	}),
	readinessCheck: (t) => {
		const explain = t(
			"devices:enum.readinessCheck.explain",
			"Each failing check is fixed by the hub operator.",
		);
		return {
			policy: row(
				t("devices:enum.readinessCheck.policy", "Device limits"),
				explain,
			),
			signing: row(
				t("devices:enum.readinessCheck.signing", "Hub signing key"),
				explain,
			),
			api: row(
				t("devices:enum.readinessCheck.api", "Public API address"),
				explain,
			),
			signaling: row(
				t("devices:enum.readinessCheck.signaling", "Connection service"),
				explain,
			),
			release: row(
				t("devices:enum.readinessCheck.release", "Release settings"),
				explain,
			),
			database: row(
				t("devices:enum.readinessCheck.database", "Database migrations"),
				explain,
			),
		};
	},
	target: (t) => ({
		"x86_64-unknown-linux-gnu": row(
			t("devices:enum.target.linuxX64", "Linux (Intel/AMD 64-bit)"),
		),
		"aarch64-unknown-linux-gnu": row(
			t("devices:enum.target.linuxArm64", "Linux (ARM 64-bit)"),
		),
		"x86_64-apple-darwin": row(
			t("devices:enum.target.macIntel", "Mac (Intel)"),
		),
		"aarch64-apple-darwin": row(
			t("devices:enum.target.macAppleSilicon", "Mac (Apple silicon)"),
		),
	}),
	targetShort: (t) => ({
		"x86_64-unknown-linux-gnu": row(
			t("devices:enum.targetShort.linuxX64", "Linux x64"),
		),
		"aarch64-unknown-linux-gnu": row(
			t("devices:enum.targetShort.linuxArm64", "Linux ARM64"),
		),
		"x86_64-apple-darwin": row(
			t("devices:enum.targetShort.macIntel", "Mac Intel"),
		),
		"aarch64-apple-darwin": row(
			t("devices:enum.targetShort.macAppleSilicon", "Mac Apple silicon"),
		),
	}),
	packageMode: (t) => ({
		binary: row(t("devices:enum.packageMode.binary", "Run directly")),
		docker: row(
			t("devices:enum.packageMode.docker", "Docker Compose"),
			t("devices:enum.packageMode.dockerExplain", "Docker only on Linux."),
		),
		both: row(
			t("devices:enum.packageMode.both", "Both"),
			t("devices:enum.packageMode.bothExplain", "Includes both start scripts."),
		),
	}),
	releaseVerification: (t, p) => ({
		verified: row(t("devices:enum.releaseVerification.verified", "Verified")),
		rejected: row(
			p.reason === undefined
				? t("devices:enum.releaseVerification.rejected", "Can't be verified")
				: t(
						"devices:enum.releaseVerification.rejectedReason",
						"Can't be verified: {{reason}}",
						p,
					),
		),
	}),
	vaultKind: (t) => ({
		owner: row(t("devices:enum.vaultKind.owner", "Owner keys")),
		shared: row(t("devices:enum.vaultKind.shared", "Shared-access keys")),
	}),
	vaultState: (t) => ({
		absent: row(t("devices:enum.vaultState.absent", "No keys here")),
		locked: row(t("devices:enum.vaultState.locked", "Locked")),
		unlocked: row(t("devices:enum.vaultState.unlocked", "Unlocked")),
		restored: row(
			t("devices:enum.vaultState.restored", "Restored"),
			t(
				"devices:enum.vaultState.restoredExplain",
				"The first unlock refreshes the metric-group identity.",
			),
		),
		onboarding: row(t("devices:enum.vaultState.onboarding", "Setting up…")),
		conflict: row(
			t("devices:enum.vaultState.conflict", "Other keys already here"),
		),
	}),
	keyState: (t) => ({
		none: row(t("devices:enum.vaultState.absent", "No keys here")),
		stale: row(t("devices:common.key.stale", "Unusable keys here")),
		locked: row(t("devices:enum.vaultState.locked", "Locked")),
		unlocking: row(t("devices:common.key.unlocking", "Unlocking…")),
		unlocked: row(t("devices:enum.vaultState.unlocked", "Unlocked")),
		held_elsewhere: row(
			t("devices:enum.lockHolder.anotherWindow", "Unlocked in another window"),
		),
		blocked: row(t("devices:common.key.identityChanged", "Identity changed")),
	}),
	lockHolder: (t) => ({
		free: row(t("devices:enum.lockHolder.free", "Not in use")),
		this_window: row(t("devices:enum.vaultState.unlocked", "Unlocked")),
		another_window: row(
			t("devices:enum.lockHolder.anotherWindow", "Unlocked in another window"),
		),
		unsupported: row(
			t("devices:enum.lockHolder.unsupported", "Browser can't protect keys"),
		),
	}),
	storage: (t) => ({
		persisted: row(t("devices:enum.storage.persisted", "Kept safely")),
		denied: row(t("devices:enum.storage.denied", "Browser may delete keys")),
		unavailable: row(
			t("devices:enum.storage.unavailable", "Browser can't guarantee storage"),
		),
	}),
	backup: (t, p) => ({
		never: row(t("devices:enum.backup.never", "Not backed up")),
		in_sync: row(
			p.revision === undefined
				? t("devices:common.safety.backedUp", "Backed up")
				: t("devices:enum.backup.inSync", "Backed up (v{{revision}})", p),
		),
		upload_pending: row(
			t("devices:enum.backup.uploadPending", "Upload pending"),
		),
		local_changes: row(
			t("devices:enum.backup.localChanges", "Backup out of date"),
		),
		hub_newer: row(
			t("devices:enum.backup.hubNewer", "Newer backup on your account"),
		),
	}),
	setupBackup: (t) => ({
		saved: row(t("devices:enum.setupBackup.saved", "Saved to your account")),
		local_only: row(
			t("devices:enum.setupBackup.localOnly", "Only on this computer"),
		),
		limit: row(
			t("devices:enum.setupBackup.limit", "Account backup limit reached"),
		),
	}),
	session: (t) => ({
		idle: row(t("devices:enum.session.idle", "Not connected")),
		admitting: row(
			t("devices:enum.session.admitting", "Getting a connection pass"),
		),
		relaying: row(t("devices:enum.session.relaying", "Reaching the device")),
		"webrtc-negotiating": row(
			t("devices:enum.session.negotiating", "Trying direct connection"),
		),
		handshaking: row(
			t("devices:enum.session.handshaking", "Securing the connection"),
		),
		ready: row(t("devices:enum.session.ready", "Live")),
		closed: row(t("devices:enum.session.closed", "Disconnected")),
	}),
	transport: (t) => {
		const explain = t(
			"devices:enum.transport.explain",
			"Both are end-to-end encrypted.",
		);
		return {
			webrtc: row(t("devices:enum.transport.webrtc", "Direct"), explain),
			websocket: row(
				t("devices:enum.transport.websocket", "Relayed through the hub"),
				explain,
			),
		};
	},
	clientError: (t) => ({
		not_sent: row(
			t("devices:enum.clientError.notSent", "Not sent (safe to retry)"),
		),
		unconfirmed: row(
			t("devices:enum.clientError.unconfirmed", "No reply (it may have run)"),
		),
	}),
	fleetIntegrity: (t) => {
		const explain = t(
			"devices:enum.fleetIntegrity.explain",
			"The hub served data the app refused.",
		);
		return {
			rollback: row(
				t("devices:enum.fleetIntegrity.rollback", "Older data than before"),
				explain,
			),
			missing_stream: row(
				t("devices:enum.fleetIntegrity.missingStream", "Status missing"),
				explain,
			),
			policy_rollback: row(
				t(
					"devices:enum.fleetIntegrity.policyRollback",
					"Access rules went backwards",
				),
				explain,
			),
			identity_mismatch: row(
				t(
					"devices:enum.fleetIntegrity.identityMismatch",
					"Device keys changed",
				),
				explain,
			),
		};
	},
	scopeKind: (t, p) => ({
		device: row(t("devices:enum.scopeKind.device", "Whole device")),
		project: row(
			p.name === undefined
				? t("devices:enum.scopeKind.app", "App")
				: t("devices:enum.scopeKind.appNamed", "App {{name}}", p),
		),
		placement: row(
			p.name === undefined
				? t("devices:enum.scopeKind.service", "Service")
				: t("devices:enum.scopeKind.serviceNamed", "Service {{name}}", p),
		),
	}),
	grant: (t, p) => ({
		requested: row(t("devices:enum.grant.requested", "Waiting for approval")),
		active: row(
			p.time === undefined
				? t("devices:enum.grant.active", "Active")
				: t("devices:enum.grant.activeUntil", "Active until {{time}}", p),
		),
		expired: row(t("devices:enum.grant.expired", "Expired")),
		revoked: row(t("devices:enum.grant.revoked", "Removed")),
	}),
	accessRules: (t, p) => ({
		none: row(t("devices:enum.accessRules.none", "Not shared")),
		applied: row(
			p.n === undefined
				? t("devices:enum.accessRules.applied", "Active on device")
				: t(
						"devices:enum.accessRules.appliedVersion",
						"Active on device (v{{n}})",
						p,
					),
		),
		waiting: row(
			p.time === undefined
				? t("devices:enum.accessRules.waiting", "Waiting for device")
				: t(
						"devices:enum.accessRules.waitingSince",
						"Waiting for device since {{time}}",
						p,
					),
		),
		expired: row(
			t("devices:enum.accessRules.expired", "Expired"),
			t(
				"devices:enum.accessRules.expiredExplain",
				"When the rules expire, shared access ends and retained history pauses for everyone, you included.",
			),
		),
	}),
	capability: (t) => ({
		...observeCapabilities(t),
		...serviceCapabilities(t),
		...deviceCapabilities(t),
		...modelCapabilities(t),
	}),
	preset: (t) => ({
		viewer: row(
			t("devices:enum.preset.viewer", "Viewer"),
			t(
				"devices:enum.preset.viewerExplain",
				"View status, Read logs, Read metrics.",
			),
		),
		operator: row(
			t("devices:enum.preset.operator", "Operator"),
			t(
				"devices:enum.preset.operatorExplain",
				"Viewer plus Start, Stop, Restart and Change instance count.",
			),
		),
		deployer: row(
			t("devices:enum.preset.deployer", "Deployer"),
			t(
				"devices:enum.preset.deployerExplain",
				"Operator plus Deploy & configure and Remove services.",
			),
		),
		device_admin: row(
			t("devices:enum.preset.deviceAdmin", "Device admin"),
			t(
				"devices:enum.preset.deviceAdminExplain",
				"Every permission. Whole device only.",
			),
		),
		model_user: row(
			t("devices:enum.preset.modelUser", "Model user"),
			t(
				"devices:enum.preset.modelUserExplain",
				"Use models on this device. Whole device only.",
			),
		),
		custom: row(t("devices:enum.preset.custom", "Custom")),
	}),
	responseState: (t) => ({
		completed: row(t("devices:enum.responseState.completed", "Done")),
		accepted: row(
			t("devices:enum.responseState.accepted", "Requested"),
			t(
				"devices:enum.responseState.acceptedExplain",
				"The device recorded it; watch the service state for the result.",
			),
		),
		pending: row(t("devices:enum.responseState.pending", "Queued")),
		rejected: row(t("devices:enum.responseState.rejected", "Refused")),
		failed: row(t("devices:enum.responseState.failed", "Failed")),
		staging: row(t("devices:enum.responseState.staging", "Preparing")),
		draining: row(
			t("devices:enum.responseState.draining", "Stopping services"),
		),
		requesting: row(
			t("devices:enum.responseState.requesting", "Handing over to the system"),
		),
		requested: row(
			t("devices:enum.responseState.requested", "Waiting for restart"),
		),
		unknown: row(t("devices:enum.responseState.unknown", "Outcome unknown")),
		rolled_back: row(t("devices:enum.responseState.rolledBack", "Rolled back")),
	}),
	rejection: (t) => ({
		unauthorized: row(
			t("devices:enum.rejection.unauthorized", "Not allowed"),
			t(
				"devices:enum.rejection.unauthorizedExplain",
				"Your access doesn't include this action.",
			),
		),
		revision_conflict: row(
			t("devices:enum.rejection.revisionConflict", "Changed meanwhile"),
			t(
				"devices:enum.rejection.revisionConflictExplain",
				"Reload and try again.",
			),
		),
		invalid: row(
			t("devices:enum.rejection.invalid", "Invalid request"),
			t(
				"devices:enum.rejection.invalidExplain",
				"The device rejected the values.",
			),
		),
		host_policy: row(
			t("devices:enum.rejection.hostPolicy", "Blocked by device policy"),
			t(
				"devices:enum.rejection.hostPolicyExplain",
				"This device requires sandboxed services.",
			),
		),
		unsupported: row(
			t("devices:enum.rejection.unsupported", "Not supported"),
			t(
				"devices:enum.rejection.unsupportedExplain",
				"Update the agent, or this needs Linux.",
			),
		),
		limit: row(
			t("devices:enum.rejection.limit", "Device limit reached"),
			t(
				"devices:enum.rejection.limitExplain",
				"For example storage, services or the journal.",
			),
		),
		busy: row(
			t("devices:enum.rejection.busy", "Device busy, retrying"),
			t("devices:enum.rejection.busyExplain", "Retried automatically."),
		),
		failed: row(
			t("devices:enum.rejection.failed", "Couldn't complete"),
			t("devices:enum.rejection.failedExplain", "Try again; see details."),
		),
	}),
	hostOpKind: (t) => ({
		reboot: row(t("devices:enum.hostOpKind.reboot", "Reboot")),
		update_agent: row(t("devices:enum.hostOpKind.updateAgent", "Agent update")),
	}),
	rebootState: (t, p) => ({
		pending: row(t("devices:enum.responseState.pending", "Queued")),
		draining: row(
			t("devices:enum.responseState.draining", "Stopping services"),
		),
		requesting: row(t("devices:enum.rebootState.requesting", "Handing over")),
		requested: row(
			t("devices:enum.responseState.requested", "Waiting for restart"),
		),
		completed: row(t("devices:enum.responseState.completed", "Done")),
		failed: row(
			p.detail === undefined
				? t("devices:enum.responseState.failed", "Failed")
				: t("devices:enum.rebootState.failedDetail", "Failed ({{detail}})", p),
		),
		unknown: row(t("devices:enum.responseState.unknown", "Outcome unknown")),
	}),
	agentUpdateState: (t) => ({
		pending: row(t("devices:enum.responseState.pending", "Queued")),
		staging: row(
			t("devices:enum.agentUpdateState.staging", "Verifying release"),
		),
		draining: row(
			t("devices:enum.responseState.draining", "Stopping services"),
		),
		requesting: row(t("devices:enum.agentUpdateState.requesting", "Switching")),
		completed: row(t("devices:enum.responseState.completed", "Done")),
		rolled_back: row(t("devices:enum.responseState.rolledBack", "Rolled back")),
		failed: row(t("devices:enum.responseState.failed", "Failed")),
		unknown: row(t("devices:enum.responseState.unknown", "Outcome unknown")),
	}),
	updateJournal: (t) => ({
		staged: row(t("devices:enum.updateJournal.staged", "Verified")),
		armed: row(t("devices:enum.updateJournal.armed", "Ready to switch")),
		swapped: row(t("devices:enum.updateJournal.swapped", "Switched")),
		completed: row(t("devices:enum.responseState.completed", "Done")),
		rolled_back: row(t("devices:enum.responseState.rolledBack", "Rolled back")),
		failed: row(t("devices:enum.responseState.failed", "Failed")),
	}),
	secretWrite: (t) => ({
		pending: row(t("devices:enum.secretWrite.pending", "Waiting for device")),
		completed: row(t("devices:enum.secretWrite.completed", "Saved")),
		failed: row(t("devices:enum.responseState.failed", "Failed")),
	}),
	desired: (t) => ({
		running: row(t("devices:enum.desired.running", "Running")),
		stopped: row(t("devices:enum.desired.stopped", "Stopped")),
	}),
	observed: (t) => ({
		unknown: row(
			t("devices:enum.observed.unknown", "Unknown"),
			t(
				"devices:enum.observed.unknownExplain",
				"No report since the last change.",
			),
		),
		starting: row(t("devices:enum.observed.starting", "Starting")),
		running: row(t("devices:enum.observed.running", "Running")),
		stopping: row(t("devices:enum.observed.stopping", "Stopping")),
		stopped: row(t("devices:enum.observed.stopped", "Stopped")),
		backoff: row(
			t("devices:enum.observed.backoff", "Restarting after a crash"),
			t(
				"devices:enum.observed.backoffExplain",
				"Waiting before the next attempt.",
			),
		),
		failed: row(
			t("devices:enum.observed.failed", "Crashed, gave up"),
			t(
				"devices:enum.observed.failedExplain",
				"Stopped retrying after repeated crashes; Start clears this.",
			),
		),
		removed: row(t("devices:enum.observed.removed", "Removed")),
	}),
	convergence: (t) =>
		Object.fromEntries(
			CONVERGENCES.map((conv) => [conv, row(convergenceLabel(t, conv))]),
		) as Table<"convergence">,
	placementSource: (t) => ({
		offline: row(
			t("devices:enum.placementSource.offline", "Offline copy"),
			t(
				"devices:enum.placementSource.offlineExplain",
				"Data copied to the device.",
			),
		),
		online: row(
			t("devices:enum.placementSource.online", "Runs online"),
			t(
				"devices:enum.placementSource.onlineExplain",
				"Data stays in the cloud.",
			),
		),
	}),
	rollout: (t, p) => ({
		staged: row(
			t("devices:enum.rollout.staged", "Update ready"),
			t(
				"devices:enum.rollout.stagedExplain",
				"The current version keeps running until the update is activated.",
			),
		),
		validating: row(
			t("devices:enum.rollout.validating", "Checking new version"),
		),
		activating: row(t("devices:enum.rollout.activating", "Switching over")),
		healthy: row(t("devices:enum.rollout.healthy", "Updated")),
		rolling_back: row(t("devices:enum.rollout.rollingBack", "Rolling back")),
		rolled_back: row(
			t("devices:enum.rollout.rolledBack", "Rolled back"),
			t(
				"devices:enum.rollout.rolledBackExplain",
				"The previous version is running again.",
			),
		),
		failed: failedRollout(t, p),
		cancelled: row(t("devices:enum.rollout.cancelled", "Cancelled")),
	}),
	failureCode: (t) => ({
		...failuresBeforeSwitch(t),
		...failuresAfterSwitch(t),
	}),
	isolation: (t) => ({
		trusted_process: row(
			t("devices:enum.isolation.trustedProcess", "Runs as the agent"),
			t("devices:enum.isolation.trustedProcessExplain", "Full device access."),
		),
		linux_sandbox: row(
			t("devices:enum.isolation.linuxSandbox", "Sandboxed"),
			t(
				"devices:enum.isolation.linuxSandboxExplain",
				"CPU, memory, process and disk limits.",
			),
		),
	}),
	hostIsolation: (t) => ({
		required: row(t("devices:enum.hostIsolation.required", "Sandbox required")),
		optional: row(
			t("devices:enum.hostIsolation.optional", "Sandbox available"),
		),
		none: row(t("devices:enum.hostIsolation.none", "No sandbox")),
		unknown: row(
			t("devices:enum.hostIsolation.unknown", "Unknown"),
			t(
				"devices:enum.hostIsolation.unknownExplain",
				"Needs whole-device View status.",
			),
		),
	}),
	eventType: (t) => {
		const served = t(
			"devices:enum.eventType.servedExplain",
			"Served by the device's web server.",
		);
		return {
			http: row(t("devices:enum.eventType.http", "Endpoint"), served),
			simple_chat: row(t("devices:enum.eventType.simpleChat", "Chat"), served),
			page: row(t("devices:enum.eventType.page", "Page"), served),
			rest: row(t("devices:enum.eventType.rest", "REST")),
			mcp: row(t("devices:enum.eventType.mcp", "MCP")),
			daemon: row(t("devices:enum.eventType.daemon", "Background")),
		};
	},
	readinessKind: (t) => ({
		listener: row(
			t("devices:enum.readinessKind.listener", "Checks its web server"),
		),
		explicit: row(
			t(
				"devices:enum.readinessKind.explicit",
				"Waits for the flow to report ready",
			),
		),
		unsupported: row(
			t("devices:enum.readinessKind.unsupported", "Can't be checked"),
			t(
				"devices:enum.readinessKind.unsupportedExplain",
				"This rules out safe updates.",
			),
		),
	}),
	valueType: (t) => ({
		Normal: row(t("devices:enum.valueType.normal", "Single value")),
		Array: row(t("devices:enum.valueType.array", "List")),
		HashSet: row(t("devices:enum.valueType.hashSet", "Set")),
		HashMap: row(t("devices:enum.valueType.hashMap", "Map")),
	}),
	dataType: (t) => {
		const number = t("devices:enum.dataType.number", "Number");
		return {
			String: row(t("devices:enum.dataType.string", "Text")),
			PathBuf: row(t("devices:enum.dataType.path", "Path")),
			Boolean: row(t("devices:enum.dataType.boolean", "Yes/No")),
			Integer: row(number),
			Float: row(number),
			Byte: row(t("devices:enum.dataType.byte", "Byte")),
			Date: row(t("devices:enum.dataType.date", "Date")),
		};
	},
	transfer: (t) => ({
		receiving: row(t("devices:enum.transfer.receiving", "Uploading")),
		committed: row(t("devices:enum.transfer.committed", "Finished")),
		aborted: row(t("devices:enum.transfer.aborted", "Aborted")),
		expired: row(t("devices:enum.transfer.expired", "Expired")),
	}),
	transferPhase: (t) => ({
		manifest: row(t("devices:enum.transferPhase.manifest", "Checking files")),
		files: row(t("devices:enum.transferPhase.files", "Sending files")),
		commit: row(t("devices:enum.transferPhase.commit", "Verifying")),
	}),
	tablePurpose: (t) => ({
		storage: row(t("devices:enum.purpose.storage", "Project storage")),
		user: row(t("devices:enum.tablePurpose.user", "User data")),
	}),
	filePurpose: (t) => ({
		files: row(t("devices:enum.filePurpose.files", "Project files")),
		storage: row(t("devices:enum.purpose.storage", "Project storage")),
		user: row(t("devices:enum.filePurpose.user", "User files")),
		temporary: row(t("devices:enum.filePurpose.temporary", "Temporary files")),
	}),
	queuedWrite: (t) => ({
		pending: row(
			t("devices:enum.queuedWrite.pending", "Waiting"),
			t(
				"devices:enum.queuedWrite.pendingExplain",
				"Will be sent automatically.",
			),
		),
		attempting: row(t("devices:enum.queuedWrite.attempting", "Sending")),
		applied: row(t("devices:enum.queuedWrite.applied", "Saved to the cloud")),
		blocked: row(
			t("devices:enum.queuedWrite.blocked", "Stuck"),
			t("devices:enum.queuedWrite.blockedExplain", "Needs you; see the error."),
		),
		conflict: row(
			t("devices:enum.queuedWrite.conflict", "Conflicts with newer cloud data"),
			t("devices:enum.queuedWrite.conflictExplain", "Needs you."),
		),
		outcome_unknown: row(
			t("devices:enum.queuedWrite.outcomeUnknown", "Might already be saved"),
			t(
				"devices:enum.queuedWrite.outcomeUnknownExplain",
				"Needs you; check before discarding.",
			),
		),
		skipped: row(t("devices:enum.queuedWrite.skipped", "Discarded")),
		superseded: row(
			t("devices:enum.queuedWrite.superseded", "Replaced by a later change"),
		),
	}),
	replayStatus: (t) => ({
		applied: row(t("devices:enum.replayStatus.applied", "Saved")),
		conflict: row(t("devices:enum.replayStatus.conflict", "Conflict")),
		outcome_unknown: row(
			t("devices:enum.queuedWrite.outcomeUnknown", "Might already be saved"),
		),
		unsupported: row(
			t(
				"devices:enum.replayStatus.unsupported",
				"Not supported by this storage",
			),
		),
		blocked: row(t("devices:enum.queuedWrite.blocked", "Stuck")),
	}),
	replayError: (t) => ({
		OFFLINE_INVALID: row(
			t("devices:enum.replayError.invalid", "Invalid change"),
		),
		OFFLINE_LIMIT_EXCEEDED: row(
			t("devices:enum.replayError.limitExceeded", "Too large or too many"),
		),
		OFFLINE_FORBIDDEN: row(
			t("devices:enum.replayError.forbidden", "Not allowed by cloud access"),
		),
		OFFLINE_PRINCIPAL_UNSUPPORTED: row(
			t(
				"devices:enum.replayError.principalUnsupported",
				"Approver can't replay",
			),
		),
		OFFLINE_SUBJECT_MISMATCH: row(
			t("devices:enum.replayError.subjectMismatch", "Belongs to another user"),
		),
		OFFLINE_DIGEST_REUSED: row(
			t("devices:enum.replayError.digestReused", "Duplicate change"),
		),
	}),
	queueState: (t) => ({
		quarantined: row(
			t("devices:enum.queueState.quarantined", "Paused: cloud access changed"),
			t("devices:enum.queueState.quarantinedExplain", "Changes are kept."),
		),
	}),
	approvalStatus: (t) => ({
		active: row(t("devices:enum.approvalStatus.active", "Active")),
		revoked: row(t("devices:enum.approvalStatus.revoked", "Revoked")),
		expired: row(t("devices:enum.approvalStatus.expired", "Expired")),
	}),
	onlineAccess: (t) => ({
		none: row(t("devices:enum.onlineAccess.none", "No project files")),
		read_only: row(
			t("devices:enum.onlineAccess.readOnly", "Read project files"),
		),
		read_write: row(
			t("devices:enum.onlineAccess.readWrite", "Read & write project files"),
			t(
				"devices:enum.onlineAccess.readWriteExplain",
				"Drops to read-only when storage is full.",
			),
		),
	}),
	usageAdmission: (t) => ({
		reserved: row(t("devices:enum.usageAdmission.reserved", "Reserved")),
		running: row(t("devices:enum.usageAdmission.running", "In progress")),
		settled: row(t("devices:enum.usageAdmission.settled", "Charged")),
		unknown: row(t("devices:enum.usageAdmission.unknown", "Unknown")),
	}),
	instancePurpose: (t) => ({
		workload: row(
			t("devices:enum.instancePurpose.workload", "Service instance"),
		),
		rollout_validation: row(
			t("devices:enum.instancePurpose.rolloutValidation", "Update check"),
		),
	}),
	instanceStatus: (t) => ({
		active: row(t("devices:enum.instanceStatus.active", "Active")),
		validating: row(
			t("devices:enum.instanceStatus.validating", "Checking update"),
		),
		retired: row(t("devices:enum.instanceStatus.retired", "Ended")),
	}),
	cpuBasis: (t) => ({
		one_logical_cpu: row(
			t("devices:enum.cpuBasis.oneLogicalCpu", "% of one CPU"),
			t(
				"devices:enum.cpuBasis.oneLogicalCpuExplain",
				"Can exceed 100 % on multi-core devices.",
			),
		),
	}),
	memoryBasis: (t) => ({
		process_rss: row(
			t("devices:enum.memoryBasis.processRss", "Process memory"),
		),
		cgroup_current: row(
			t("devices:enum.memoryBasis.cgroupCurrent", "Sandbox memory"),
		),
	}),
	ioBasis: (t) => ({
		cgroup_block_io: row(
			t("devices:enum.ioBasis.cgroupBlockIo", "Sandbox disk I/O"),
		),
		process_io: row(t("devices:enum.ioBasis.processIo", "Process disk I/O")),
		observed_placement_workers: row(
			t(
				"devices:enum.ioBasis.observedPlacementWorkers",
				"Sum of the app's services",
			),
		),
	}),
	resourceBasis: (t) => ({
		all_host_interfaces: row(
			t(
				"devices:enum.resourceBasis.allHostInterfaces",
				"All network interfaces",
			),
		),
		agent_state_volume: row(
			t("devices:enum.resourceBasis.agentStateVolume", "Agent's data disk"),
		),
	}),
	usageWindow: (t, p) => ({
		current_process_lifetimes: row(
			t("devices:enum.usageWindow.current", "Since instances started"),
			t("devices:enum.usageWindow.explain", "Not a billing record."),
		),
		retained_observed_process_checkpoints: row(
			p.date === undefined
				? t("devices:enum.usageWindow.retained", "Retained history")
				: t("devices:enum.usageWindow.retainedSince", "Since {{date}}", p),
			t("devices:enum.usageWindow.explain", "Not a billing record."),
		),
	}),
	logKind: (t) => ({
		log: row(t("devices:enum.logKind.log", "Log")),
		message: row(t("devices:enum.logKind.message", "Activity")),
	}),
	logStream: (t) => ({
		stdout: row(t("devices:enum.logStream.stdout", "Output")),
		stderr: row(t("devices:enum.logStream.stderr", "Errors")),
	}),
	messageKind: (t) => ({
		operation: row(t("devices:enum.messageKind.operation", "Command")),
		replica: row(t("devices:enum.messageKind.replica", "Instance")),
	}),
	archiveKind: (t) => ({
		logs: row(t("devices:enum.archiveKind.logs", "Logs")),
		metrics: row(t("devices:enum.archiveKind.metrics", "Metrics")),
	}),
	archiveRoster: (t) => ({
		none: row(t("devices:enum.archiveRoster.none", "Not set up")),
		current: row(t("devices:enum.archiveRoster.current", "Recording")),
		expired: row(
			t("devices:enum.archiveRoster.expired", "Paused: readers list expired"),
		),
		stale: row(t("devices:enum.archiveRoster.stale", "Paused: access changed")),
	}),
	telemetryTier: (t, p) => ({
		free: row(t("devices:enum.telemetryTier.free", "Not stored in the cloud")),
		stored: row(storedTierLabel(t, p)),
	}),
	mlsResult: (t) => ({
		key_package: row(t("devices:enum.mlsResult.keyPackage", "Request created")),
		joined: row(t("devices:enum.mlsResult.joined", "Joined")),
		application: row(t("devices:enum.mlsResult.application", "New sample")),
		epoch_changed: row(
			t("devices:enum.mlsResult.epochChanged", "Readers changed"),
		),
		removed: row(t("devices:enum.mlsResult.removed", "Removed from readers")),
		duplicate: row(t("devices:enum.mlsResult.duplicate", "Already read")),
	}),
	validity: (t, p) => ({
		not_yet_valid: row(t("devices:enum.validity.notYetValid", "Not valid yet")),
		valid: row(t("devices:enum.validity.valid", "Valid")),
		expiring: row(
			p.count === undefined
				? t("devices:enum.validity.expiring", "Expiring")
				: t("devices:enum.validity.expiresIn", "Expires in {{count}} d", p),
			t("devices:enum.validity.expiringExplain", "Within 7 days."),
		),
		expired: row(t("devices:enum.validity.expired", "Expired")),
	}),
	certMode: (t) => ({
		manual: row(t("devices:enum.certMode.manual", "Manual")),
		"csr-issued": row(
			t("devices:enum.certMode.csrIssued", "Signed from a request"),
		),
		delegated: row(
			t("devices:enum.certMode.delegated", "Automatic (your authority)"),
		),
		acme: row(t("devices:enum.certMode.acme", "Automatic (Let's Encrypt)")),
	}),
	csrPurpose: (t) => ({
		service: row(t("devices:enum.csrPurpose.service", "Certificate request")),
		issuer: row(
			t("devices:enum.csrPurpose.issuer", "Renewal authority request"),
		),
	}),
	csrState: (t) => ({
		pending: row(
			t("devices:enum.csrState.pending", "Waiting for signed certificate"),
		),
		installed: row(t("devices:enum.csrState.installed", "Installed")),
		discarded: row(t("devices:enum.csrState.discarded", "Discarded")),
		expired: row(t("devices:enum.csrState.expired", "Expired")),
		stale: row(
			t("devices:enum.csrState.stale", "Out of date"),
			t(
				"devices:enum.csrState.staleExplain",
				"The certificate changed meanwhile.",
			),
		),
	}),
	delegation: (t) => ({
		requested: row(t("devices:enum.grant.requested", "Waiting for approval")),
		active: row(t("devices:enum.delegation.active", "On")),
		renewing: row(t("devices:enum.delegation.renewing", "Renewing")),
		error: row(t("devices:enum.delegation.error", "Failing")),
		disabled: row(t("devices:enum.delegation.disabled", "Off")),
	}),
	acme: (t) => ({
		scheduled: row(t("devices:enum.acme.scheduled", "Scheduled")),
		attempting: row(t("devices:enum.acme.attempting", "Requesting")),
		issued: row(t("devices:enum.acme.issued", "Issued")),
		backoff: row(t("devices:enum.acme.backoff", "Retrying later")),
		disabled: row(t("devices:enum.delegation.disabled", "Off")),
	}),
	acmeEnvironment: (t) => ({
		lets_encrypt_staging: row(
			t(
				"devices:enum.acmeEnvironment.staging",
				"Test (not trusted by browsers)",
			),
		),
		lets_encrypt_production: row(
			t("devices:enum.acmeEnvironment.production", "Production"),
		),
	}),
	orgCa: (t) => ({
		pending: row(t("devices:enum.orgCa.pending", "Not saved yet")),
		active: row(t("devices:enum.orgCa.active", "Active")),
		issuing_expired: row(
			t("devices:enum.orgCa.issuingExpired", "Signing key expired"),
		),
		root_expired: row(t("devices:enum.orgCa.rootExpired", "Root expired")),
		restored: row(t("devices:enum.orgCa.restored", "Restored")),
	}),
	noticeStage: (t) => ({
		week: row(t("devices:enum.noticeStage.week", "7 days before")),
		three_days: row(t("devices:enum.noticeStage.threeDays", "3 days before")),
		day: row(t("devices:enum.noticeStage.day", "1 day before")),
		expired: row(t("devices:enum.noticeStage.expired", "On expiry")),
	}),
	noticeChannel: (t) => ({
		push: row(t("devices:enum.noticeChannel.push", "Push")),
		email: row(t("devices:enum.noticeChannel.email", "Email")),
	}),
	noticeStatus: (t) => ({
		pending: row(t("devices:enum.noticeStatus.pending", "Scheduled")),
		sent: row(t("devices:enum.noticeStatus.sent", "Sent")),
		cancelled: row(t("devices:enum.noticeStatus.cancelled", "Cancelled")),
	}),
};

export const ENUM_FAMILIES = Object.keys(BUILDERS) as EnumFamily[];

/*
 * Tables without params are built once per `t`: a table costs one t() call
 * per string of the family, and rows ask for a label on every render.
 * react-i18next hands out a new `t` when the language or its resources
 * change, which drops the cached tables with the old one.
 */
const TABLES = new WeakMap<DevicesT, Map<EnumFamily, unknown>>();

/** All values of a family with their copy (pickers, legends, exhaustive tests). Read-only: tables without params are shared. */
export function enumTable<F extends EnumFamily>(
	t: DevicesT,
	family: F,
	params?: EnumParams,
): Table<F> {
	const build = BUILDERS[family] as Builder<F>;
	if (params) return build(t, params);
	let tables = TABLES.get(t);
	if (!tables) {
		tables = new Map();
		TABLES.set(t, tables);
	}
	let table = tables.get(family) as Table<F> | undefined;
	if (!table) {
		table = build(t, {});
		tables.set(family, table);
	}
	return table;
}

/**
 * The copy of one wire value. A value this client doesn't know (a newer agent
 * or hub) reads "Unknown": never the raw value (R3), never a crash.
 */
export function enumCopy<F extends EnumFamily>(
	t: DevicesT,
	family: F,
	value: EnumValues[F],
	params?: EnumParams,
): EnumCopy {
	const table: Partial<Record<string, EnumCopy>> = enumTable(t, family, params);
	const known = Object.hasOwn(table, value) ? table[value] : undefined;
	return known ?? { label: t("devices:enum.unknownValue", "Unknown") };
}

/** The UI label of a wire value (never the value itself, R3). */
export function enumLabel<F extends EnumFamily>(
	t: DevicesT,
	family: F,
	value: EnumValues[F],
	params?: EnumParams,
): string {
	return enumCopy(t, family, value, params).label;
}

/** The one-line explanation, when the copy guide has one. */
export function enumExplain<F extends EnumFamily>(
	t: DevicesT,
	family: F,
	value: EnumValues[F],
	params?: EnumParams,
): string | undefined {
	return enumCopy(t, family, value, params).explain;
}
