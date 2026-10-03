import { formatMoment } from "../../../../lib/date";
import type {
	CopyParams,
	FixAction,
	GateFailure,
	GateReason,
} from "../../../../lib/device-management/model/types";
import type { Capability } from "../../../../lib/device-management/types";
import { humanFileSize } from "../../../../lib/utils";
import type { DevicesT } from "../primitives/area-context";
import { enumLabel } from "./enum-labels";

/** `useAreaTime()` satisfies it; without one, times use the default locale. */
export interface CopyFormat {
	/** "11:00" today, "29 Sept, 11:00" otherwise, for unix seconds. */
	at(atS: number): string;
	locale?: string;
}

const DEFAULT_FORMAT: CopyFormat = { at: (atS) => formatMoment(atS * 1000) };

export interface CopyContext {
	t: DevicesT;
	p: CopyParams;
	/** The device's name, or "this device". */
	device: string;
	/** The service's name, or "this service". */
	service: string;
	/** A unix-seconds param, formatted; undefined when absent. */
	at(key: string): string | undefined;
	list(
		capabilities: readonly Capability[],
		type: "conjunction" | "disjunction",
	): string;
}

export function copyContext(
	t: DevicesT,
	p: CopyParams = {},
	fmt: CopyFormat = DEFAULT_FORMAT,
): CopyContext {
	return {
		t,
		p,
		device:
			typeof p.device === "string"
				? p.device
				: t("devices:gate.thisDevice", "this device"),
		service:
			typeof p.service === "string"
				? p.service
				: t("devices:gate.thisService", "this service"),
		at: (key) =>
			typeof p[key] === "number" ? fmt.at(p[key] as number) : undefined,
		list: (capabilities, type) =>
			new Intl.ListFormat(fmt.locale, { type }).format(
				capabilities.map((capability) =>
					enumLabel(t, "capability", capability),
				),
			),
	};
}

type Sentence = string | readonly [title: string, text: string];

interface GateCopyContext extends CopyContext {
	result: GateFailure;
}

const bytes = (value: string | number | undefined) =>
	typeof value === "number" ? humanFileSize(value) : "";

/**
 * "Start needs a live connection." when the gate names what is gated
 * (`params.action`, with `params.actions` = how many it names for "Restart and
 * Stop need …"); undefined when it doesn't.
 */
function actionNeedsLive({ t, p }: CopyContext): string | undefined {
	if (typeof p.action !== "string" || !p.action) return undefined;
	return t("devices:gate.actionNeedsLive", {
		action: p.action,
		count: typeof p.actions === "number" ? p.actions : 1,
		defaultValue_one: "{{action}} needs a live connection.",
		defaultValue_other: "{{action}} need a live connection.",
	});
}

/** One entry per gate reason (SPEC §6.3 gate copy, IA §3.3). */
const GATE_COPY = {
	desktop_only: ({ t }) =>
		t("devices:gate.desktopOnly", "This works only in the desktop app."),
	desktop_only_offline_app: ({ t }) =>
		t(
			"devices:gate.desktopOnlyOfflineApp",
			"Local-only apps deploy from the desktop app.",
		),
	browser_size_limit: ({ t, p }) => [
		t("devices:gate.browserSizeLimit", "Too large to upload from the browser."),
		t(
			"devices:gate.browserSizeLimitText",
			"The browser takes files up to {{fileLimit}} each and {{totalLimit}} in total. Use the desktop app for larger apps.",
			{ fileLimit: bytes(p.fileLimit), totalLimit: bytes(p.totalLimit) },
		),
	],
	sign_in: ({ t }) =>
		t("devices:gate.signIn", "Sign in to manage your devices."),
	hub_checking: ({ t }) =>
		t(
			"devices:gate.hubChecking",
			"Checking whether this hub supports devices…",
		),
	hub_devices_off: ({ t }) =>
		t("devices:gate.hubDevicesOff", "Devices are off on this hub."),
	hub_unreachable: ({ t }) =>
		t("devices:gate.hubUnreachable", "The hub can't be reached right now."),
	token_restricted: ({ t }) =>
		t(
			"devices:gate.tokenRestricted",
			"Your access token is restricted. Use a token with full permissions.",
		),
	not_shared_with_you: ({ t, device }) =>
		t("devices:gate.notSharedWithYou", "{{device}} isn't shared with you.", {
			device,
		}),
	owner_only_device: ({ t, p, device }) =>
		typeof p.owner === "string"
			? t(
					"devices:gate.ownerOnlyNamed",
					"Only the owner, {{owner}}, can do this on {{device}}.",
					{ owner: p.owner, device },
				)
			: t(
					"devices:gate.ownerOnlyDevice",
					"Only the owner of {{device}} can do this.",
					{ device },
				),
	device_revoked: ({ t, device, at }) =>
		at("at")
			? t("devices:gate.deviceRevokedAt", "{{device}} was revoked on {{at}}.", {
					device,
					at: at("at"),
				})
			: t("devices:gate.deviceRevoked", "{{device}} was revoked.", { device }),
	device_not_active: ({ t, device }) =>
		t("devices:gate.deviceNotActive", "{{device}} isn't active.", { device }),
	access_ended: ({ t, device, at }) =>
		at("at")
			? t(
					"devices:gate.accessEndedAt",
					"Your access to {{device}} ended on {{at}}.",
					{ device, at: at("at") },
				)
			: t("devices:gate.accessEnded", "Your access to {{device}} has ended.", {
					device,
				}),
	needs_capability: (c) => needsSentence(c),
	needs_device_scope: (c) => needsSentence(c),
	unlock_to_check_permissions: ({ t, device }) => [
		t(
			"devices:gate.unlockToCheckPermissions",
			"Unlock {{device}} to check your permissions.",
			{ device },
		),
		t(
			"devices:gate.unlockToCheckPermissionsText",
			"Your permissions are in the device's access rules, which only keys on this computer can read.",
		),
	],
	no_keys_here: ({ t, device }) =>
		t("devices:gate.noKeysHere", "This computer has no keys for {{device}}.", {
			device,
		}),
	keys_unusable: ({ t, device }) =>
		t(
			"devices:gate.keysUnusable",
			"The keys for {{device}} on this computer can't be used any more.",
			{ device },
		),
	locked_status: ({ t, device }) => [
		t("devices:gate.lockedStatus", "Unlock {{device}} to see its services.", {
			device,
		}),
		t(
			"devices:gate.lockedText",
			"The hub can't read them; only keys on this computer can.",
		),
	],
	locked_logs: ({ t, device }) =>
		t("devices:gate.lockedLogs", "Unlock {{device}} to read logs.", { device }),
	locked_metrics: ({ t, device }) =>
		t("devices:gate.lockedMetrics", "Unlock {{device}} to read metrics.", {
			device,
		}),
	locked_change: ({ t, device }) =>
		t("devices:gate.lockedChange", "Unlock {{device}} to change settings.", {
			device,
		}),
	locked_lookup: ({ t, device }) =>
		t("devices:gate.lockedLookup", "Unlock {{device}} to look up a command.", {
			device,
		}),
	locked_access: ({ t, device }) =>
		t(
			"devices:gate.lockedAccess",
			"Unlock {{device}} to change who has access.",
			{ device },
		),
	locked_keys: ({ t, device }) =>
		t("devices:gate.lockedKeys", "Unlock {{device}} to use its keys.", {
			device,
		}),
	locked_certificates: ({ t, device }) =>
		t(
			"devices:gate.lockedCertificates",
			"Unlock {{device}} to manage its certificates.",
			{ device },
		),
	locked_run: ({ t, device }) =>
		t(
			"devices:gate.lockedRun",
			"Unlock {{device}} to run its actions and forms.",
			{ device },
		),
	unlocking: ({ t, device }) =>
		t("devices:gate.unlocking", "Unlocking {{device}}…", { device }),
	held_elsewhere: ({ t, device }) =>
		t(
			"devices:gate.heldElsewhere",
			"{{device}} is unlocked in another window.",
			{ device },
		),
	password_required: ({ t, device }) =>
		t(
			"devices:gate.passwordRequired",
			"Enter the device password for {{device}} to continue.",
			{ device },
		),
	lock_unsupported: ({ t }) => [
		t(
			"devices:gate.lockUnsupported",
			"This browser can't protect device keys.",
		),
		t(
			"devices:gate.useDesktopOrCurrentBrowser",
			"Use the desktop app or a current browser.",
		),
	],
	crypto_unavailable: ({ t }) => [
		t(
			"devices:gate.cryptoUnavailable",
			"Device encryption couldn't load in this browser.",
		),
		t(
			"devices:gate.useDesktopOrCurrentBrowser",
			"Use the desktop app or a current browser.",
		),
	],
	identity_blocked: ({ t, device }) => [
		t(
			"devices:gate.identityBlocked",
			"The hub reports different keys for {{device}} than the ones you trusted.",
			{ device },
		),
		t(
			"devices:gate.identityBlockedText",
			"The device may have been set up again, or someone may be impersonating it. Compare the fingerprint on the device before you continue.",
		),
	],
	offline_needs_live: (c) => {
		const { t, device, at } = c;
		const needs = actionNeedsLive(c);
		if (needs)
			return at("since")
				? t(
						"devices:gate.offlineSinceAction",
						"{{device}} has been offline since {{since}}. {{needs}}",
						{ device, since: at("since"), needs },
					)
				: t("devices:gate.offlineAction", "{{device}} is offline. {{needs}}", {
						device,
						needs,
					});
		return at("since")
			? t(
					"devices:gate.offlineNeedsLiveSince",
					"{{device}} has been offline since {{since}}. This needs a live connection.",
					{ device, since: at("since") },
				)
			: t(
					"devices:gate.offlineNeedsLive",
					"{{device}} is offline. This needs a live connection.",
					{ device },
				);
	},
	never_connected_needs_live: (c) => {
		const { t, device } = c;
		const needs = actionNeedsLive(c);
		return needs
			? t(
					"devices:gate.neverConnectedAction",
					"{{device}} hasn't checked in yet. {{needs}}",
					{ device, needs },
				)
			: t(
					"devices:gate.neverConnectedNeedsLive",
					"{{device}} hasn't checked in yet. This needs a live connection.",
					{ device },
				);
	},
	connect_first: ({ t, device }) =>
		t("devices:gate.connectFirst", "Connect live to {{device}} first.", {
			device,
		}),
	connecting: ({ t, device }) =>
		t("devices:gate.connecting", "Connecting to {{device}}…", { device }),
	connection_failed: ({ t, device }) =>
		t(
			"devices:gate.connectionFailed",
			"The live connection to {{device}} failed.",
			{ device },
		),
	connection_not_configured: ({ t }) =>
		t(
			"devices:gate.connectionNotConfigured",
			"Live connections aren't set up on this hub.",
		),
	agent_update_needed: ({ t, p }) =>
		typeof p.version === "string"
			? t(
					"devices:gate.agentUpdateNeededVersion",
					"Needs a newer device agent (running {{version}}).",
					{ version: p.version },
				)
			: t("devices:gate.agentUpdateNeeded", "Needs a newer device agent."),
	agent_features_unknown: ({ t }) =>
		t(
			"devices:gate.agentFeaturesUnknown",
			"The device agent hasn't reported what it supports yet.",
		),
	needs_linux_systemd: ({ t }) =>
		t("devices:gate.needsLinuxSystemd", "Needs Linux with systemd."),
	rollout_source_unsupported: ({ t, p }) =>
		p.source === "online"
			? t(
					"devices:gate.rolloutSourceOnline",
					"This device agent can't update online services with startup checks. Update the agent or use a quick update.",
				)
			: t(
					"devices:gate.rolloutSourceOffline",
					"This device agent can't update offline services with startup checks. Update the agent or use a quick update.",
				),
	sandbox_required: ({ t, device }) =>
		t(
			"devices:gate.sandboxRequired",
			"{{device}} requires sandboxed services.",
			{ device },
		),
	sandbox_unavailable: ({ t, device }) =>
		t(
			"devices:gate.sandboxUnavailable",
			"{{device}} can't run sandboxed services.",
			{ device },
		),
	plan_no_history: ({ t, p }) =>
		typeof p.tier === "string"
			? t(
					"devices:gate.planNoHistoryTier",
					"Your {{tier}} plan doesn't store device history.",
					{ tier: p.tier },
				)
			: t(
					"devices:gate.planNoHistory",
					"Your plan doesn't store device history.",
				),
	role_read_boards: ({ t }) =>
		t("devices:gate.roleReadBoards", "Needs Read boards on the app."),
	role_admin_execute: ({ t }) =>
		t(
			"devices:gate.roleAdminExecute",
			"Needs Admin or Owner on the app with Execute boards.",
		),
	role_app_owner: ({ t }) =>
		t("devices:gate.roleAppOwner", "Only the app owner can do this."),
	owner_only: ({ t, p, device }) =>
		typeof p.owner === "string"
			? t(
					"devices:gate.ownerOnlyNamed",
					"Only the owner, {{owner}}, can do this on {{device}}.",
					{ owner: p.owner, device },
				)
			: t(
					"devices:gate.ownerOnlyDevice",
					"Only the owner of {{device}} can do this.",
					{ device },
				),
	owner_keys_elsewhere: ({ t, device }) =>
		t(
			"devices:gate.ownerKeysElsewhere",
			"Your owner keys for {{device}} aren't on this computer.",
			{ device },
		),
	rollout_in_progress: ({ t }) =>
		t(
			"devices:gate.rolloutInProgress",
			"An update is in progress. Only Stop is allowed.",
		),
	wait_for_rollout: ({ t, service, at }) =>
		at("by")
			? t(
					"devices:gate.waitForRolloutBy",
					"Wait for {{service}}'s update to finish (by {{by}} at the latest).",
					{ service, by: at("by") },
				)
			: t(
					"devices:gate.waitForRollout",
					"Wait for {{service}}'s update to finish.",
					{ service },
				),
	secret_pending: ({ t, service }) =>
		t(
			"devices:gate.secretPending",
			"A new secret value for {{service}} is still being applied.",
			{ service },
		),
	host_operation_running: ({ t, device }) =>
		t(
			"devices:gate.hostOperationRunning",
			"Wait for the reboot or agent update on {{device}} to finish.",
			{ device },
		),
	service_must_be_stopped: ({ t, service }) =>
		t("devices:gate.serviceMustBeStopped", "Stop {{service}} first.", {
			service,
		}),
	service_must_be_running: ({ t, service }) =>
		t("devices:gate.serviceMustBeRunning", "{{service}} needs to be running.", {
			service,
		}),
	single_instance_only: ({ t, service }) =>
		t(
			"devices:gate.singleInstanceOnly",
			"{{service}} can run only one instance.",
			{ service },
		),
	no_staged_update: ({ t, service }) =>
		t("devices:gate.noStagedUpdate", "{{service}} has no staged update.", {
			service,
		}),
	different_source: ({ t }) =>
		t(
			"devices:gate.differentSource",
			"A safe update needs the same kind of app (local-only or online) as the running version.",
		),
	release_trust_missing: ({ t }) =>
		t(
			"devices:gate.releaseTrustMissing",
			"This hub has no trusted agent releases set up.",
		),
	certificate_in_use: ({ t }) =>
		t(
			"devices:gate.certificateInUse",
			"Services still use this certificate. Assign them another one first.",
		),
	certificate_not_valid_now: ({ t }) =>
		t(
			"devices:gate.certificateNotValidNow",
			"This certificate isn't valid right now.",
		),
	config_too_large: ({ t, p }) =>
		t(
			"devices:gate.configTooLarge",
			"These settings are too large ({{bytes, number}} of {{max, number}} bytes).",
			{ bytes: p.bytes, max: p.max },
		),
	queue_quarantined: ({ t, service }) =>
		t(
			"devices:gate.queueQuarantined",
			"Changes from {{service}} are paused because its cloud access changed.",
			{ service },
		),
	cloud_approval_required: ({ t }) =>
		t(
			"devices:gate.cloudApprovalRequired",
			"Online services need a cloud approval first.",
		),
	approval_exists: ({ t, service }) =>
		t(
			"devices:gate.approvalExists",
			"{{service}} already has a cloud approval.",
			{ service },
		),
	no_models_in_approval: ({ t, service }) =>
		t(
			"devices:gate.noModelsInApproval",
			"The cloud approval for {{service}} doesn't include any models.",
			{ service },
		),
	payer_only: ({ t }) =>
		t(
			"devices:gate.payerOnly",
			"Only the person who pays for it can revoke this spending limit.",
		),
	delegator_or_owner_only: ({ t }) =>
		t(
			"devices:gate.delegatorOrOwnerOnly",
			"Only the device owner or the person who approved it can do this.",
		),
	operation_too_old: ({ t }) =>
		t(
			"devices:gate.operationTooOld",
			"Results are kept for 24 hours, and this one is older.",
		),
	policy_not_applied: ({ t, device }) =>
		t(
			"devices:gate.policyNotApplied",
			"{{device}} hasn't applied your latest access rules yet.",
			{ device },
		),
	not_a_reader: ({ t }) =>
		t("devices:gate.notAReader", "You're not on the readers list."),
	access_slots_full: ({ t, p }) =>
		t(
			"devices:gate.accessSlotsFull",
			"This device already has the maximum of {{max, number}} access grants.",
			{ max: p.max },
		),
	readiness_failing: ({ t }) =>
		t(
			"devices:gate.readinessFailing",
			"This hub isn't ready to set up devices yet.",
		),
	device_limit_reached: ({ t, p }) =>
		typeof p.pending === "number" && p.pending > 0
			? t(
					"devices:gate.deviceLimitReachedPending",
					"You've reached this hub's device limit ({{used, number}} of {{max, number}}, counting setups that haven't started yet).",
					{ used: p.used, max: p.max },
				)
			: t(
					"devices:gate.deviceLimitReached",
					"You've reached this hub's device limit ({{used, number}} of {{max, number}}).",
					{ used: p.used, max: p.max },
				),
	pending_setup_limit_reached: ({ t, p }) =>
		p.period === "day"
			? t(
					"devices:gate.dailySetupLimitReached",
					"You've reached today's setup limit ({{used, number}} of {{max, number}}).",
					{ used: p.used, max: p.max },
				)
			: t(
					"devices:gate.pendingSetupLimitReached",
					"Too many setups are waiting to be started ({{used, number}} of {{max, number}}).",
					{ used: p.used, max: p.max },
				),
	no_placement_in_app: ({ t, device }) =>
		t(
			"devices:gate.noPlacementInApp",
			"This app has no service on {{device}}.",
			{ device },
		),
	authority_missing: ({ t }) =>
		t(
			"devices:gate.authorityMissing",
			"There's no organisation authority on this computer.",
		),
} satisfies Record<GateReason, (c: GateCopyContext) => Sentence>;

const SCOPED_NEEDS = {
	device: (c: CopyContext, need: string) =>
		c.t("devices:gate.needsOnDevice", "Needs {{need}} on the whole device.", {
			need,
		}),
	project: (c: CopyContext, need: string) =>
		c.t("devices:gate.needsOnProject", "Needs {{need}} on this app.", {
			need,
		}),
	service: (c: CopyContext, need: string) =>
		c.t("devices:gate.needsOnService", "Needs {{need}} on this service.", {
			need,
		}),
	any: (c: CopyContext, need: string) =>
		c.t("devices:gate.needs", "Needs {{need}}.", { need }),
};

function needsSentence(c: GateCopyContext): Sentence {
	const need = c.result.need ?? [];
	const list = c.list(need, c.p.any === 1 ? "disjunction" : "conjunction");
	const scope = c.p.scope as keyof typeof SCOPED_NEEDS;
	const title = (SCOPED_NEEDS[scope] ?? SCOPED_NEEDS.any)(c, list);
	if (c.result.copy.code !== "needs_device_scope") return title;
	return [
		title,
		c.t(
			"devices:gate.needsDeviceScopeText",
			"You have it only for some apps or services.",
		),
	];
}

const FIX_COPY = {
	sign_in: (t) => t("devices:gate.fix.signIn", "Sign in"),
	use_full_token: (t) => t("devices:gate.fix.useFullToken", "Use a full token"),
	open_hub_status: (t) =>
		t("devices:gate.fix.openHubStatus", "Open hub status"),
	unlock: (t) => t("devices:gate.fix.unlock", "Unlock…"),
	take_over: (t) => t("devices:gate.fix.takeOver", "Use here"),
	restore_keys: (t) => t("devices:gate.fix.restoreKeys", "Restore keys…"),
	import_key_file: (t) =>
		t("devices:gate.fix.importKeyFile", "Import backup files…"),
	request_access: (t) =>
		t("devices:gate.fix.requestAccess", "Request shared access"),
	ask_to_renew: (t) => t("devices:gate.fix.askToRenew", "Ask to renew"),
	connect: (t) => t("devices:gate.fix.connect", "Connect live"),
	diagnose: (t) => t("devices:gate.fix.diagnose", "Diagnose"),
	update_agent: (t) => t("devices:gate.fix.updateAgent", "Update agent…"),
	ask_owner: (t) => t("devices:gate.fix.askOwner", "Ask the owner"),
	use_desktop: (t) => t("devices:gate.fix.useDesktop", "Open the desktop app"),
	see_plans: (t) => t("devices:gate.fix.seePlans", "See plans"),
	keep_keys_safely: (t) =>
		t("devices:gate.fix.keepKeysSafely", "Keep keys safely"),
	review_identity: (t) =>
		t("devices:gate.fix.reviewIdentity", "Compare on the device"),
	forget_identity: (t) =>
		t("devices:gate.fix.forgetIdentity", "Forget trusted identity…"),
	fix_clock: (t) => t("devices:gate.fix.fixClock", "How to fix the clock"),
} satisfies Record<FixAction["kind"], (t: DevicesT) => string>;

/** Button label for a fix action (SPEC §6.6 verbs). */
export function fixLabel(t: DevicesT, fix: FixAction): string {
	return FIX_COPY[fix.kind](t);
}

export interface GateCopy {
	/** The bold first sentence of GateNotice. */
	title: string;
	/** The block form's second sentence, when there is one. */
	text?: string;
	/** "You have View status and Read metrics." on capability gates. */
	have?: string;
	/** Label of the primary fix button. */
	fix?: string;
	/** The one line next to a disabled control (GateInline). */
	inline: string;
}

function haveSentence(
	t: DevicesT,
	c: CopyContext,
	have: readonly Capability[],
) {
	return have.length
		? t("devices:gate.have", "You have {{have}}.", {
				have: c.list(have, "conjunction"),
			})
		: t("devices:gate.haveNone", "You have no permissions there.");
}

/** Translates a failed gate (R3: codes never reach the screen). */
export function gateCopy(
	t: DevicesT,
	result: GateFailure,
	fmt?: CopyFormat,
): GateCopy {
	const c = { ...copyContext(t, result.copy.params, fmt), result };
	const sentence = GATE_COPY[result.copy.code](c);
	const [title, text] =
		typeof sentence === "string" ? [sentence, undefined] : sentence;
	const have = result.need ? haveSentence(t, c, result.have ?? []) : undefined;
	return {
		title,
		...(text ? { text } : {}),
		...(have ? { have } : {}),
		...(result.fix ? { fix: fixLabel(t, result.fix) } : {}),
		inline: have ? `${title} ${have}` : title,
	};
}
