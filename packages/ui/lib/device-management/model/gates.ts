import type { Capability, InventoryScope } from "../types";
import { orderCapabilities } from "./permissions";
import { presence } from "./presence";
import {
	type ActionId,
	type AgentFeature,
	type CopyParams,
	type FixAction,
	GATE_IDS,
	type GateCheck,
	type GateContext,
	type GateFailure,
	type GateFeatures,
	type GateId,
	type GateNoticeKind,
	type GateReason,
	type GateRequirement,
	type GateResult,
	type HostIsolationMode,
	type Relationship,
} from "./types";

type Target = GateContext["target"];

type Failure = GateCheck & {
	gate: GateId;
	kind: GateNoticeKind;
	have?: Capability[];
	need?: Capability[];
	/** IA §3.3 "Other": evaluated after G13, so a lasting reason always wins over a transient one. */
	late?: true;
};
type Check = (ctx: GateContext) => Failure | null;
type LockedReason = Extract<GateReason, `locked_${string}`>;

/** `GateRequirement` plus the IA §3.3 details its fields can't express. */
export interface ActionGate extends GateRequirement {
	/** At least one of these ("status or metrics"). */
	capsAny?: readonly Capability[];
	/** Capabilities the current state adds, e.g. `start` to restart a stopped service. */
	capsWhen?: (ctx: GateContext) => readonly Capability[];
	/** Works without the hub: G1–G3 are skipped. */
	offline?: true;
	/** G7 sentence while locked. */
	locked?: LockedReason;
	/** G10: the device's host policy constrains the isolation profile. */
	hostPolicy?: true;
	checks?: readonly Check[];
}

export const BROWSER_UPLOAD_LIMITS = {
	fileBytes: 64 * 1024 ** 2,
	totalBytes: 256 * 1024 ** 2,
} as const;
export const ADVANCED_CONFIG_MAX_BYTES = 12_000;
export const MAX_GRANTS = 24;
export const OPERATION_LOOKUP_S = 24 * 3600;

const ANY: readonly Relationship[] = [
	"owner",
	"shared",
	"cloud_approval",
	"unknown",
];
const MANAGE: readonly Relationship[] = ["owner", "shared", "unknown"];
const OWNER: readonly Relationship[] = ["owner"];

function params(
	entries: Record<string, string | number | null | undefined>,
): CopyParams | undefined {
	const result: CopyParams = {};
	for (const [key, value] of Object.entries(entries))
		if (value !== undefined && value !== null && value !== "")
			result[key] = value;
	return Object.keys(result).length ? result : undefined;
}

function fail(
	gate: GateId,
	kind: GateNoticeKind,
	reason: GateReason,
	rest: Omit<Failure, "gate" | "kind" | "reason"> = {},
): Failure {
	return { gate, kind, reason, ...rest };
}

const late = (
	gate: GateId,
	kind: GateNoticeKind,
	reason: GateReason,
	rest: Omit<Failure, "gate" | "kind" | "reason" | "late"> = {},
): Failure => fail(gate, kind, reason, { ...rest, late: true });

const deviceName = (ctx: GateContext) =>
	ctx.device?.display_name || ctx.device?.name;

function deviceFix(
	ctx: GateContext,
	make: (deviceId: string) => FixAction,
): FixAction | undefined {
	return ctx.device ? make(ctx.device.device_id) : undefined;
}

/* G0–G13 (IA §3.1). Each returns the failure of its own rung or null. */

interface Walk {
	req: ActionGate;
	ctx: GateContext;
	/** A recipient's capabilities are not known before unlock (P3): G5 defers to G7. */
	capsUnknown: boolean;
}

function platformGate({ req, ctx }: Walk): Failure | null {
	return req.platform === "desktop" && ctx.platform !== "desktop"
		? fail("G0", "platform", "desktop_only", { fix: { kind: "use_desktop" } })
		: null;
}

function signInGate({ req, ctx }: Walk): Failure | null {
	if (req.offline || ctx.auth.signedIn) return null;
	return fail("G1", "hub", "sign_in", { fix: { kind: "sign_in" } });
}

function hubGate({ req, ctx }: Walk): Failure | null {
	if (req.offline) return null;
	switch (ctx.hub.state) {
		case "on":
			return null;
		case "checking":
			return fail("G2", "hub", "hub_checking");
		case "off":
			return fail("G2", "hub", "hub_devices_off", {
				fix: { kind: "open_hub_status" },
			});
		case "unreachable":
			return fail("G2", "hub", "hub_unreachable", {
				fix: { kind: "open_hub_status" },
			});
	}
}

function tokenGate({ req, ctx }: Walk): Failure | null {
	if (req.offline || ctx.auth.tokenScopeAll) return null;
	return fail("G3", "noaccess", "token_restricted", {
		fix: { kind: "use_full_token" },
	});
}

function ownerOnly(ctx: GateContext, gate: GateId, reason: GateReason) {
	return fail(gate, "owner", reason, {
		params: params({ owner: ctx.labels?.owner, device: deviceName(ctx) }),
	});
}

function accessEndedAt(ctx: GateContext): number | undefined {
	const endsAt = ctx.device?.access_expires_at;
	if (endsAt != null) return endsAt <= ctx.now ? endsAt : undefined;
	const grants = ctx.capabilities;
	if (!grants?.length) return undefined;
	const ends = grants.map((grant) => grant.expiresAt);
	if (ends.some((at) => at === undefined || at > ctx.now)) return undefined;
	return Math.max(...(ends as number[]));
}

function relationshipGate({ req, ctx }: Walk): Failure | null {
	const device = ctx.device;
	if (req.deviceActive && device?.status === "revoked")
		return fail("G4", "noaccess", "device_revoked", {
			params: params({ device: deviceName(ctx), at: device.revoked_at }),
		});
	const allowed = req.relationship;
	if (!allowed) return null;
	if (!allowed.includes(ctx.relationship)) {
		if (allowed.includes("shared"))
			return fail("G4", "noaccess", "not_shared_with_you", {
				params: params({ device: deviceName(ctx) }),
				fix: deviceFix(ctx, (deviceId) => ({
					kind: "request_access",
					deviceId,
				})),
			});
		return ownerOnly(ctx, "G4", "owner_only_device");
	}
	if (ctx.relationship !== "shared") return null;
	const endedAt = accessEndedAt(ctx);
	if (endedAt === undefined) return null;
	return fail("G4", "noaccess", "access_ended", {
		params: params({ device: deviceName(ctx), at: endedAt }),
		fix: deviceFix(ctx, (deviceId) => ({ kind: "ask_to_renew", deviceId })),
	});
}

function requiredCaps(req: ActionGate, ctx: GateContext): Capability[] {
	const base = req.caps === "owner" ? [] : (req.caps ?? []);
	return orderCapabilities([...base, ...(req.capsWhen?.(ctx) ?? [])]);
}

const needsCaps = (req: ActionGate, ctx: GateContext) =>
	req.caps === "owner" ||
	Boolean(req.capsAny?.length) ||
	requiredCaps(req, ctx).length > 0;

const covers = (
	scope: InventoryScope,
	target: Target,
	deviceOnly: boolean,
): boolean => {
	if (scope.kind === "device") return true;
	if (deviceOnly) return false;
	if (!target?.projectId) return true;
	if (scope.project_id !== target.projectId) return false;
	if (scope.kind === "project") return true;
	return scope.placement_id === target.placementId;
};

const askScope = (
	target: Target,
	deviceOnly: boolean,
): InventoryScope | undefined => {
	if (deviceOnly) return { kind: "device" };
	if (target?.projectId && target.placementId)
		return {
			kind: "placement",
			project_id: target.projectId,
			placement_id: target.placementId,
		};
	if (target?.projectId)
		return { kind: "project", project_id: target.projectId };
	return undefined;
};

function scopeLabel(target: Target, deviceOnly: boolean) {
	if (deviceOnly) return "device";
	if (target?.placementId) return "service";
	return target?.projectId ? "project" : "any";
}

function capabilityGate({ req, ctx, capsUnknown }: Walk): Failure | null {
	if (req.caps === "owner")
		return ctx.relationship === "owner"
			? null
			: ownerOnly(ctx, "G5", "owner_only_device");
	if (ctx.relationship === "owner" || capsUnknown || !ctx.capabilities)
		return null;
	const required = requiredCaps(req, ctx);
	const any = req.capsAny ?? [];
	if (!required.length && !any.length) return null;
	const deviceOnly = req.deviceScopeOnly === true;
	const current = ctx.capabilities.filter(
		(grant) => grant.expiresAt === undefined || grant.expiresAt > ctx.now,
	);
	const have = new Set(
		current
			.filter((grant) => covers(grant.scope, ctx.target, deviceOnly))
			.flatMap((grant) => grant.caps),
	);
	const missing = required.filter((capability) => !have.has(capability));
	const anyMissing = any.length > 0 && !any.some((cap) => have.has(cap));
	if (!missing.length && !anyMissing) return null;
	const need = orderCapabilities([...missing, ...(anyMissing ? any : [])]);
	const elsewhere = new Set(current.flatMap((grant) => grant.caps));
	const narrower =
		deviceOnly &&
		missing.every((capability) => elsewhere.has(capability)) &&
		(!anyMissing || any.some((capability) => elsewhere.has(capability)));
	const scope = askScope(ctx.target, deviceOnly);
	return fail(
		"G5",
		"noaccess",
		narrower ? "needs_device_scope" : "needs_capability",
		{
			params: params({
				scope: scopeLabel(ctx.target, deviceOnly),
				any: anyMissing && !missing.length ? 1 : undefined,
			}),
			have: orderCapabilities(have),
			need,
			fix: deviceFix(ctx, (deviceId) => ({
				kind: "ask_owner",
				deviceId,
				need,
				...(scope ? { scope } : {}),
			})),
		},
	);
}

function keysGate({ req, ctx }: Walk): Failure | null {
	if (req.keys !== true && req.keys !== "stored") return null;
	const state = ctx.keys?.state ?? "none";
	if (state !== "none" && (state !== "stale" || req.keys === "stored"))
		return null;
	return fail(
		"G6",
		"nokeys",
		state === "none" ? "no_keys_here" : "keys_unusable",
		{
			params: params({ device: deviceName(ctx) }),
			fix: deviceFix(ctx, (deviceId) => ({ kind: "restore_keys", deviceId })),
		},
	);
}

function unlockGate({ req, ctx, capsUnknown }: Walk): Failure | null {
	if (!req.unlocked || !ctx.keys) return null;
	const { state, lastError } = ctx.keys;
	if (state === "unlocked") return null;
	const device = params({ device: deviceName(ctx) });
	if (state === "blocked")
		return fail("G7", "policy", "identity_blocked", {
			params: device,
			fix: deviceFix(ctx, (deviceId) => ({
				kind: "review_identity",
				deviceId,
			})),
		});
	if (state === "held_elsewhere")
		return fail("G7", "locked", "held_elsewhere", {
			params: device,
			fix: deviceFix(ctx, (deviceId) => ({ kind: "take_over", deviceId })),
		});
	if (lastError?.code === "lock_unsupported")
		return fail("G7", "platform", "lock_unsupported", {
			fix: { kind: "use_desktop" },
		});
	if (lastError?.code === "crypto_unavailable")
		return fail("G7", "platform", "crypto_unavailable", {
			fix: { kind: "use_desktop" },
		});
	if (state === "unlocking")
		return fail("G7", "locked", "unlocking", { params: device });
	if (req.unlocked === "password" || state !== "locked") return null;
	return fail(
		"G7",
		"locked",
		capsUnknown
			? "unlock_to_check_permissions"
			: (req.locked ?? "locked_change"),
		{
			params: device,
			fix: deviceFix(ctx, (deviceId) => ({
				kind: "unlock",
				deviceId,
				...(req.session ? { connectLive: true } : {}),
			})),
		},
	);
}

function sessionGate({ req, ctx }: Walk): Failure | null {
	if (!req.session) return null;
	const live = ctx.live;
	if (live?.kind === "live" || live?.kind === "renewing") return null;
	const device = deviceName(ctx);
	if (live?.kind === "connecting" || live?.kind === "reconnecting")
		return fail("G8", "live", "connecting", { params: params({ device }) });
	const cause =
		live?.kind === "failed" || live?.kind === "unreachable"
			? live.cause
			: undefined;
	if (
		cause?.step === "getting_pass" &&
		(cause.code === "not_configured" || cause.code === "needs_wss")
	)
		return fail("G8", "hub", "connection_not_configured", {
			fix: { kind: "open_hub_status" },
		});
	const seen = ctx.device ? presence(ctx.device, ctx.now) : undefined;
	const diagnose = deviceFix(ctx, (deviceId) => ({
		kind: "diagnose",
		deviceId,
	}));
	if (seen?.kind === "offline")
		return fail("G8", "live", "offline_needs_live", {
			params: params({ device, since: seen.since }),
			fix: diagnose,
		});
	if (seen?.kind === "never")
		return fail("G8", "live", "never_connected_needs_live", {
			params: params({ device }),
			fix: diagnose,
		});
	if (cause)
		return fail("G8", "live", "connection_failed", {
			params: params({ device }),
			fix: diagnose,
		});
	return fail("G8", "live", "connect_first", {
		params: params({ device }),
		fix: deviceFix(ctx, (deviceId) => ({ kind: "connect", deviceId })),
	});
}

function featureGate({ req, ctx }: Walk): Failure | null {
	if (!req.features) return null;
	if (!ctx.features) return fail("G9", "unsupported", "agent_features_unknown");
	const check = req.features(ctx.features, ctx);
	return check ? { gate: "G9", kind: "unsupported", ...check } : null;
}

function hostPolicyGate({ req, ctx }: Walk): Failure | null {
	if (!req.hostPolicy) return null;
	const profile = ctx.extra?.isolationProfile;
	const device = params({ device: deviceName(ctx) });
	if (ctx.hostIsolation === "required" && profile === "trusted_process")
		return fail("G10", "policy", "sandbox_required", { params: device });
	if (ctx.hostIsolation === "none" && profile === "linux_sandbox")
		return fail("G10", "policy", "sandbox_unavailable", { params: device });
	return null;
}

function planGate({ req, ctx }: Walk): Failure | null {
	if (req.plan !== "history" || ctx.planTier?.storesHistory !== false)
		return null;
	return fail("G11", "plan", "plan_no_history", {
		params: params({ tier: ctx.planTier.tierName }),
		fix: { kind: "see_plans" },
	});
}

type ProjectRole = NonNullable<GateContext["projectRole"]>;

const ROLE_RULES = {
	read_boards: [(role) => role.readBoards, "role_read_boards"],
	admin_execute: [
		(role) => (role.admin || role.owner) && role.executeBoards,
		"role_admin_execute",
	],
	app_owner: [(role) => role.owner, "role_app_owner"],
} satisfies Record<
	NonNullable<GateRequirement["role"]>,
	readonly [(role: ProjectRole) => boolean, GateReason]
>;

function roleGate({ req, ctx }: Walk): Failure | null {
	if (!req.role || !ctx.projectRole) return null;
	const [allowed, reason] = ROLE_RULES[req.role];
	return allowed(ctx.projectRole) ? null : fail("G12", "role", reason);
}

function ownerPowersGate({ req, ctx }: Walk): Failure | null {
	if (!req.ownerPowers) return null;
	if (ctx.relationship !== "owner") return ownerOnly(ctx, "G13", "owner_only");
	if (ctx.ownerPowers) return null;
	return fail("G13", "nokeys", "owner_keys_elsewhere", {
		params: params({ device: deviceName(ctx) }),
		fix: deviceFix(ctx, (deviceId) => ({ kind: "restore_keys", deviceId })),
	});
}

const LADDER: Record<GateId, (walk: Walk) => Failure | null> = {
	G0: platformGate,
	G1: signInGate,
	G2: hubGate,
	G3: tokenGate,
	G4: relationshipGate,
	G5: capabilityGate,
	G6: keysGate,
	G7: unlockGate,
	G8: sessionGate,
	G9: featureGate,
	G10: hostPolicyGate,
	G11: planGate,
	G12: roleGate,
	G13: ownerPowersGate,
};

/* Action preconditions (IA §3.3 "Other"). */

const uploadPlatform: Check = (ctx) => {
	if (ctx.platform !== "web") return null;
	if (ctx.extra?.offlineLocalApp)
		return fail("G0", "platform", "desktop_only_offline_app", {
			fix: { kind: "use_desktop" },
		});
	const bytes = ctx.extra?.browserUploadBytes;
	if (
		!bytes ||
		(bytes.largestFile <= BROWSER_UPLOAD_LIMITS.fileBytes &&
			bytes.total <= BROWSER_UPLOAD_LIMITS.totalBytes)
	)
		return null;
	return fail("G0", "platform", "browser_size_limit", {
		params: {
			largestFile: bytes.largestFile,
			total: bytes.total,
			fileLimit: BROWSER_UPLOAD_LIMITS.fileBytes,
			totalLimit: BROWSER_UPLOAD_LIMITS.totalBytes,
		},
		fix: { kind: "use_desktop" },
	});
};

const hubFix: FixAction = { kind: "open_hub_status" };

const releaseTrust: Check = (ctx) =>
	ctx.extra?.releaseTrust === false
		? fail("G2", "hub", "release_trust_missing", { fix: hubFix })
		: null;

const setupReady: Check = (ctx) => {
	if (ctx.extra?.readinessOk === false)
		return fail("G2", "hub", "readiness_failing", { fix: hubFix });
	const trust = releaseTrust(ctx);
	if (trust) return trust;
	const { limits, usage } = ctx.hub;
	if (!limits || !usage) return null;
	// The hub counts unused setup packages against the device limit too.
	const used = usage.active_devices + usage.pending_enrollments;
	if (used >= limits.max_devices)
		return fail("G2", "hub", "device_limit_reached", {
			params: {
				used,
				max: limits.max_devices,
				pending: usage.pending_enrollments,
			},
		});
	if (usage.pending_enrollments >= limits.max_pending_enrollments)
		return fail("G2", "hub", "pending_setup_limit_reached", {
			params: {
				used: usage.pending_enrollments,
				max: limits.max_pending_enrollments,
			},
		});
	if (usage.enrollments_last_24h >= limits.max_enrollments_per_day)
		return fail("G2", "hub", "pending_setup_limit_reached", {
			params: {
				used: usage.enrollments_last_24h,
				max: limits.max_enrollments_per_day,
				period: "day",
			},
		});
	return null;
};

const signalingConfigured: Check = (ctx) => {
	const live = ctx.live;
	const cause =
		live?.kind === "failed" || live?.kind === "unreachable"
			? live.cause
			: undefined;
	return cause?.step === "getting_pass" &&
		(cause.code === "not_configured" || cause.code === "needs_wss")
		? fail("G8", "hub", "connection_not_configured", { fix: hubFix })
		: null;
};

const serviceParams = (ctx: GateContext) =>
	params({ service: ctx.labels?.service, device: deviceName(ctx) });

const busyRollout: Check = (ctx) =>
	ctx.extra?.activeRollout
		? late("G8", "busy", "rollout_in_progress", { params: serviceParams(ctx) })
		: null;

const waitRollout: Check = (ctx) =>
	ctx.extra?.activeRollout
		? late("G8", "busy", "wait_for_rollout", {
				params: params({
					service: ctx.labels?.service,
					by: ctx.extra.rolloutDeadlineAt,
				}),
			})
		: null;

const busySecret: Check = (ctx) =>
	ctx.extra?.pendingSecret
		? late("G8", "busy", "secret_pending", { params: serviceParams(ctx) })
		: null;

const busyHostOperation: Check = (ctx) =>
	ctx.extra?.hostOperationActive
		? late("G8", "busy", "host_operation_running", {
				params: params({ device: deviceName(ctx) }),
			})
		: null;

const mustBeStopped: Check = (ctx) =>
	ctx.extra?.desiredState === "running"
		? late("G8", "busy", "service_must_be_stopped", {
				params: serviceParams(ctx),
			})
		: null;

const mustBeRunning: Check = (ctx) =>
	ctx.extra?.desiredState === "stopped"
		? late("G8", "busy", "service_must_be_running", {
				params: serviceParams(ctx),
			})
		: null;

/** A person-started run needs a process that runs now: the device refuses one for a service that is starting or failing. */
const runsNow: Check = (ctx) => {
	const observed = ctx.extra?.observedState;
	return observed !== undefined && observed !== "running"
		? late("G8", "busy", "service_must_be_running", {
				params: serviceParams(ctx),
			})
		: null;
};

const singleInstance: Check = (ctx) => {
	const max = ctx.extra?.maxReplicas;
	return max !== undefined && max <= 1
		? late("G8", "unsupported", "single_instance_only", {
				params: serviceParams(ctx),
			})
		: null;
};

const staged =
	(states: readonly string[]): Check =>
	(ctx) => {
		const state = ctx.extra?.rolloutState;
		if (state === undefined && ctx.extra?.activeRollout === undefined)
			return null;
		return state !== undefined && states.includes(state)
			? null
			: late("G8", "busy", "no_staged_update", { params: serviceParams(ctx) });
	};

const sameSource: Check = (ctx) =>
	ctx.extra?.sameSource === false
		? fail("G9", "unsupported", "different_source", {
				params: params({ source: ctx.extra.source }),
			})
		: null;

const certificateValid: Check = (ctx) =>
	ctx.extra?.certificateValidNow === false
		? late("G8", "unsupported", "certificate_not_valid_now")
		: null;

const certificateUnused: Check = (ctx) => {
	const count = ctx.extra?.bindingCount ?? 0;
	return count > 0
		? late("G8", "busy", "certificate_in_use", { params: { count } })
		: null;
};

const configSize: Check = (ctx) => {
	const bytes = ctx.extra?.configBytes;
	return bytes !== undefined && bytes > ADVANCED_CONFIG_MAX_BYTES
		? late("G8", "unsupported", "config_too_large", {
				params: { bytes, max: ADVANCED_CONFIG_MAX_BYTES },
			})
		: null;
};

const hadPlacement: Check = (ctx) =>
	ctx.extra?.hadPlacementInApp === false
		? late("G8", "unsupported", "no_placement_in_app", {
				params: params({ device: deviceName(ctx) }),
			})
		: null;

const operationAge: Check = (ctx) => {
	const issuedAt = ctx.extra?.operationIssuedAt;
	return issuedAt !== undefined && ctx.now - issuedAt > OPERATION_LOOKUP_S
		? late("G8", "unsupported", "operation_too_old", {
				params: { at: issuedAt },
			})
		: null;
};

const rosterMember: Check = (ctx) =>
	ctx.relationship !== "owner" && ctx.extra?.rosterMember === false
		? fail("G5", "noaccess", "not_a_reader", {
				params: params({ owner: ctx.labels?.owner }),
			})
		: null;

const policyApplied: Check = (ctx) =>
	ctx.extra?.policyApplied === false
		? late("G8", "busy", "policy_not_applied", {
				params: params({ device: deviceName(ctx) }),
			})
		: null;

const queueOpen: Check = (ctx) =>
	ctx.extra?.queueQuarantined
		? late("G8", "policy", "queue_quarantined", { params: serviceParams(ctx) })
		: null;

const grantLimit: Check = (ctx) => {
	const count = ctx.extra?.grantCount;
	return count !== undefined && count >= MAX_GRANTS
		? late("G4", "policy", "access_slots_full", {
				params: { count, max: MAX_GRANTS },
			})
		: null;
};

const authorityPresent: Check = (ctx) =>
	ctx.extra?.authorityPresent === false
		? fail("G6", "nokeys", "authority_missing")
		: null;

const cloudApproval: Check = (ctx) =>
	ctx.extra?.source === "online" && ctx.extra.approvalExists === false
		? late("G4", "policy", "cloud_approval_required", {
				params: serviceParams(ctx),
			})
		: null;

const approvalAbsent: Check = (ctx) =>
	ctx.extra?.approvalExists
		? late("G4", "policy", "approval_exists", { params: serviceParams(ctx) })
		: null;

const approvalHasModels: Check = (ctx) =>
	ctx.extra?.approvalHasModels === false
		? late("G4", "policy", "no_models_in_approval", {
				params: serviceParams(ctx),
			})
		: null;

const ownerOrDelegator: Check = (ctx) =>
	ctx.relationship !== "owner" && ctx.extra?.delegatorIsMe === false
		? fail("G4", "owner", "delegator_or_owner_only", {
				params: params({ owner: ctx.labels?.owner }),
			})
		: null;

const payer: Check = (ctx) =>
	ctx.extra?.payerIsMe === false ? fail("G4", "owner", "payer_only") : null;

const whenStopped =
	(caps: readonly Capability[]) =>
	(ctx: GateContext): readonly Capability[] =>
		ctx.extra?.desiredState === "stopped" ? caps : [];

const whenRunning =
	(caps: readonly Capability[]) =>
	(ctx: GateContext): readonly Capability[] =>
		ctx.extra?.desiredState === "running" ? caps : [];

/* G9 feature checks: they read the agent's flags, never `agent_version`. */

function agentUpdate(features: GateFeatures, ctx: GateContext): GateCheck {
	return {
		reason: "agent_update_needed",
		params: params({ version: features.agentVersion }),
		fix: deviceFix(ctx, (deviceId) => ({ kind: "update_agent", deviceId })),
	};
}

const hostOperation =
	(operation: "reboot" | "update_agent") =>
	(features: GateFeatures, ctx: GateContext): GateCheck | null => {
		if (!features.hostOperations) return agentUpdate(features, ctx);
		if (features.hostOperations[operation]) return null;
		return {
			reason: "needs_linux_systemd",
			hide: true,
			params: params({ os: features.os }),
		};
	};

const flag =
	(key: "certificateManagement" | "certificateIssuance" | "certificateAcme") =>
	(features: GateFeatures, ctx: GateContext): GateCheck | null =>
		features[key] ? null : agentUpdate(features, ctx);

const agentFlag =
	(key: AgentFeature) =>
	(features: GateFeatures, ctx: GateContext): GateCheck | null =>
		features.flags[key] ? null : agentUpdate(features, ctx);

function rolloutSource(
	features: GateFeatures,
	ctx: GateContext,
): GateCheck | null {
	const source = ctx.extra?.source ?? "offline";
	if ((features.rolloutSources ?? ["offline"]).includes(source)) return null;
	return {
		reason: "rollout_source_unsupported",
		params: params({ source, version: features.agentVersion }),
		fix: deviceFix(ctx, (deviceId) => ({ kind: "update_agent", deviceId })),
	};
}

/* IA §3.3, row by row. */

const live = (
	caps: readonly Capability[],
	rest: Partial<ActionGate> = {},
): ActionGate => ({
	plane: "live",
	relationship: MANAGE,
	deviceActive: true,
	caps,
	keys: true,
	unlocked: true,
	session: true,
	locked: "locked_change",
	...rest,
});
const hub = (rest: Partial<ActionGate> = {}): ActionGate => ({
	plane: "hub",
	...rest,
});
const local = (rest: Partial<ActionGate> = {}): ActionGate => ({
	plane: "local",
	offline: true,
	...rest,
});
const certificates = (
	caps: readonly Capability[],
	feature: Parameters<typeof flag>[0],
	rest: Partial<ActionGate> = {},
): ActionGate =>
	live(caps, {
		deviceScopeOnly: caps.includes("manage_certificates"),
		locked: "locked_certificates",
		features: flag(feature),
		...rest,
	});

export const ACTION_GATES: Readonly<Record<ActionId, ActionGate>> = {
	view_list: hub(),
	view_cert_summary: hub({
		relationship: MANAGE,
		capsAny: ["status", "manage_certificates"],
		deviceScopeOnly: true,
	}),
	revoke_device: hub({ relationship: OWNER, deviceActive: true }),
	rename_device: hub({ relationship: OWNER, deviceActive: true }),
	setup_device: {
		plane: ["hub", "local"],
		keys: "creates",
		checks: [setupReady],
	},
	cancel_setup: hub(),
	request_access: { plane: ["local", "hub"], keys: "creates" },
	fleet_monitor: {
		plane: "snap",
		relationship: MANAGE,
		deviceActive: true,
		capsAny: ["status", "metrics"],
		keys: true,
		unlocked: true,
		locked: "locked_status",
	},
	saved_inventory: {
		plane: "saved",
		relationship: MANAGE,
		caps: ["status"],
		keys: true,
		unlocked: true,
		locked: "locked_status",
	},
	connect_live: live([], {
		session: undefined,
		locked: "locked_status",
		checks: [signalingConfigured],
	}),
	refresh_status: live(["status"], { locked: "locked_status" }),
	start: live(["start"], {
		checks: [busyRollout, busySecret, busyHostOperation],
	}),
	stop: live(["stop"]),
	restart: live(["restart"], {
		capsWhen: whenStopped(["start"]),
		checks: [busyRollout, busySecret, busyHostOperation],
	}),
	scale: live(["scale"], { checks: [singleInstance, busyRollout] }),
	remove_service: live(["remove"], { checks: [mustBeStopped, busyRollout] }),
	run_event: live(["start"], {
		locked: "locked_run",
		features: agentFlag("on_demand_events"),
		checks: [mustBeRunning, runsNow, busyRollout, busyHostOperation],
	}),
	upload_revision: live(["deploy"], { checks: [uploadPlatform] }),
	create_service: live(["deploy"], {
		hostPolicy: true,
		checks: [uploadPlatform, cloudApproval],
	}),
	update_service: live(["deploy"], {
		capsWhen: whenRunning(["stop"]),
		hostPolicy: true,
		checks: [uploadPlatform, busyRollout],
	}),
	update_with_checks: live(["deploy", "start"], {
		features: rolloutSource,
		hostPolicy: true,
		checks: [
			uploadPlatform,
			sameSource,
			mustBeRunning,
			busyRollout,
			busyHostOperation,
			busySecret,
		],
	}),
	activate_staged: live(["deploy", "start"], { checks: [staged(["staged"])] }),
	discard_staged: live(["deploy"], {
		checks: [staged(["staged", "validating"])],
	}),
	set_secret: live(["deploy"], { checks: [busyRollout] }),
	change_tls: certificates(["manage_certificates"], "certificateManagement", {
		checks: [certificateValid],
	}),
	advanced_config: live(["deploy"], {
		capsWhen: whenRunning(["stop"]),
		hostPolicy: true,
		checks: [configSize, busyRollout],
	}),
	live_metrics: live(["metrics"], { locked: "locked_metrics" }),
	project_metrics: live(["metrics"], {
		locked: "locked_metrics",
		role: "read_boards",
		checks: [hadPlacement],
	}),
	logs: live(["logs"], { locked: "locked_logs" }),
	messages: live(["logs"], { locked: "locked_logs" }),
	check_unconfirmed: live([], {
		locked: "locked_lookup",
		checks: [operationAge],
	}),
	group_metrics_read: live(["metrics"], {
		locked: "locked_metrics",
		checks: [rosterMember],
	}),
	approve_metric_readers: live([], {
		relationship: OWNER,
		ownerPowers: true,
		locked: "locked_metrics",
		checks: [policyApplied],
	}),
	reset_metric_reader: {
		plane: ["local", "live"],
		relationship: MANAGE,
		deviceActive: true,
		keys: true,
		unlocked: "password",
	},
	approve_history_readers: live([], {
		relationship: OWNER,
		ownerPowers: true,
		locked: "locked_access",
		checks: [policyApplied],
	}),
	history_read_device: live([], {
		capsAny: ["logs", "metrics"],
		locked: "locked_logs",
	}),
	history_read_cloud: {
		plane: ["hub", "snap"],
		relationship: MANAGE,
		keys: true,
		unlocked: true,
		plan: "history",
		locked: "locked_logs",
		checks: [rosterMember],
	},
	share_access: {
		plane: "hub",
		relationship: OWNER,
		deviceActive: true,
		keys: true,
		unlocked: "password",
		ownerPowers: true,
		locked: "locked_access",
		checks: [grantLimit],
	},
	agent_update: live(["update_agent"], {
		deviceScopeOnly: true,
		features: hostOperation("update_agent"),
		checks: [releaseTrust, waitRollout, busyHostOperation],
	}),
	reboot: live(["reboot"], {
		deviceScopeOnly: true,
		features: hostOperation("reboot"),
		checks: [waitRollout, busyHostOperation],
	}),
	certificates_list: certificates(["status"], "certificateManagement"),
	certificate_import: certificates(
		["manage_certificates"],
		"certificateManagement",
	),
	certificate_delete: certificates(
		["manage_certificates"],
		"certificateManagement",
		{ checks: [certificateUnused] },
	),
	csr_create: certificates(["manage_certificates"], "certificateIssuance"),
	csr_install: certificates(["manage_certificates"], "certificateIssuance"),
	sign_with_org_ca: local({ checks: [authorityPresent] }),
	renewal_delegation: certificates([], "certificateIssuance", {
		relationship: OWNER,
		ownerPowers: true,
		checks: [authorityPresent],
	}),
	acme_configure: certificates([], "certificateAcme", {
		relationship: OWNER,
		ownerPowers: true,
	}),
	org_ca_manage: local(),
	certificate_reminders: hub(),
	offline_queue_read: live(["status"], { locked: "locked_status" }),
	offline_queue_retry: live(["deploy"], { checks: [queueOpen] }),
	offline_queue_skip: live(["deploy"], { checks: [queueOpen] }),
	cloud_access_create: hub({
		relationship: MANAGE,
		deviceActive: true,
		caps: ["deploy"],
		role: "admin_execute",
		checks: [approvalAbsent],
	}),
	cloud_access_revoke: hub({ relationship: ANY, checks: [ownerOrDelegator] }),
	spending_limit_create: hub({
		relationship: MANAGE,
		deviceActive: true,
		checks: [ownerOrDelegator, approvalHasModels],
	}),
	spending_limit_revoke: hub({ relationship: ANY, checks: [payer] }),
	account_backup_save: {
		plane: ["hub", "local"],
		relationship: MANAGE,
		deviceActive: true,
		keys: true,
		unlocked: "password",
	},
	account_backup_restore: {
		plane: ["hub", "local"],
		relationship: MANAGE,
		keys: "creates",
		unlocked: "password",
	},
	import_key_file: local({ keys: "creates", unlocked: "password" }),
	download_key_file: local({ keys: true, unlocked: "password" }),
	change_device_password: local({ keys: true, unlocked: "password" }),
	delete_local_keys: local({ keys: "stored" }),
	forget_identity: local(),
	models_view: live([], {
		capsAny: ["model_use", "model_manage"],
		deviceScopeOnly: true,
		locked: "locked_metrics",
		features: agentFlag("model_host"),
	}),
	models_manage: live(["model_manage"], {
		deviceScopeOnly: true,
		features: agentFlag("model_host"),
	}),
	models_use: live(["model_use"], {
		deviceScopeOnly: true,
		features: agentFlag("model_host"),
	}),
	models_ensure: live(["deploy"], { features: agentFlag("model_store") }),
};

function toResult(failure: Failure): GateFailure {
	const result: GateFailure = {
		ok: false,
		gate: failure.gate,
		kind: failure.kind,
		hide: failure.hide === true,
		copy: failure.params
			? { code: failure.reason, params: failure.params }
			: { code: failure.reason },
	};
	if (failure.have) result.have = failure.have;
	if (failure.need) result.need = failure.need;
	if (failure.fix) result.fix = failure.fix;
	return result;
}

const OK: GateResult = { ok: true };

/**
 * Walks G0 → G13 and returns the first failure (IA §3.1); transient preconditions come last.
 * A control that cannot apply to this object (R7, e.g. Reboot on a Mac) is hidden whatever else fails.
 */
export function evaluateGate(action: ActionId, ctx: GateContext): GateResult {
	const req = ACTION_GATES[action];
	const extras = [...(req.checks ?? []), ...(req.extra ? [req.extra] : [])]
		.map((check) => check(ctx))
		.filter((failure): failure is Failure => failure !== null);
	const walk: Walk = {
		req,
		ctx,
		capsUnknown:
			ctx.relationship !== "owner" &&
			ctx.capabilities === undefined &&
			needsCaps(req, ctx),
	};
	const cannotApply = featureGate(walk);
	if (cannotApply?.hide) return toResult(cannotApply);
	for (const gate of GATE_IDS) {
		const failure =
			LADDER[gate](walk) ??
			extras.find((extra) => extra.gate === gate && !extra.late);
		if (failure) return toResult(failure);
	}
	const transient = extras.find((extra) => extra.late);
	return transient ? toResult(transient) : OK;
}

export function evaluateGates<A extends ActionId>(
	actions: readonly A[],
	ctx: GateContext,
): Record<A, GateResult> {
	const results = {} as Record<A, GateResult>;
	for (const action of actions) results[action] = evaluateGate(action, ctx);
	return results;
}

/** G10 for forms: the isolation profile to preselect and whether the user may change it. */
export function isolationDefault(
	hostIsolation: HostIsolationMode | undefined,
): {
	profile: "trusted_process" | "linux_sandbox";
	locked: boolean;
	reason?: GateReason;
} {
	if (hostIsolation === "required")
		return {
			profile: "linux_sandbox",
			locked: true,
			reason: "sandbox_required",
		};
	if (hostIsolation === "none")
		return {
			profile: "trusted_process",
			locked: true,
			reason: "sandbox_unavailable",
		};
	return { profile: "trusted_process", locked: false };
}
