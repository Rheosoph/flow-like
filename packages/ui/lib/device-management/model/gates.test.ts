import { describe, expect, test } from "bun:test";
import type { Capability } from "../types";
import type { KeySessionSnapshot, LiveState } from "../workspace/types";
import {
	ACTION_GATES,
	evaluateGate,
	evaluateGates,
	isolationDefault,
} from "./gates";
import type {
	ActionId,
	DeviceRow,
	GateContext,
	GateFailure,
	GateReason,
	GateResult,
	SourcePlane,
} from "./types";

const NOW = 1_790_000_000;

const device = (patch: Partial<DeviceRow> = {}): DeviceRow => ({
	device_id: "dev-1",
	owner_id: "owner",
	name: "edge-berlin-01",
	status: "active",
	registered_at: NOW - 86_400,
	last_seen_at: NOW - 30,
	auth_epoch: 1,
	identity: {} as DeviceRow["identity"],
	...patch,
});

const keys = (patch: Partial<KeySessionSnapshot> = {}): KeySessionSnapshot => ({
	deviceId: "dev-1",
	state: "unlocked",
	role: "owner",
	grantId: "grant-1",
	canSign: true,
	keepUnlocked: false,
	restoredNeedsFreshEndpoint: false,
	...patch,
});

const LIVE: LiveState = {
	kind: "live",
	transport: "webrtc",
	expiresAt: NOW + 300,
	bootId: "boot-1",
	connectedAt: NOW - 10,
};

const ctx = (patch: Partial<GateContext> = {}): GateContext => ({
	now: NOW,
	platform: "desktop",
	auth: { signedIn: true, tokenScopeAll: true },
	hub: { state: "on" },
	device: device(),
	relationship: "owner",
	keys: keys(),
	live: LIVE,
	features: {
		flags: {},
		hostOperations: { reboot: true, update_agent: true },
		certificateManagement: true,
		certificateIssuance: true,
		certificateAcme: true,
		rolloutSources: ["offline", "online"],
		agentVersion: "0.9.3",
		os: "linux",
		source: { src: "live", age: "live" },
	},
	ownerPowers: true,
	labels: { owner: "Felix", service: "support-bot" },
	...patch,
});

/** A recipient whose device-scope grant holds `caps`, everything else ready. */
const recipient = (
	caps: Capability[],
	patch: Partial<GateContext> = {},
): GateContext =>
	ctx({
		relationship: "shared",
		device: device({ owner_id: "someone-else" }),
		keys: keys({ role: "shared", canSign: false }),
		capabilities: [{ scope: { kind: "device" }, caps, expiresAt: NOW + 3600 }],
		ownerPowers: false,
		...patch,
	});

function failure(result: GateResult): GateFailure {
	if (result.ok) throw new Error("expected the gate to fail");
	return result;
}

const ACTIONS = Object.keys(ACTION_GATES) as ActionId[];

type Need = "●" | "–";
interface Row {
	plane: SourcePlane | SourcePlane[];
	/** Capabilities a recipient needs; "owner" = owner only; "–" = not gated on a grant. */
	caps: Capability[] | { any: Capability[] } | "owner" | "–";
	keys: Need | "creates";
	unlocked: Need | "password";
	session: Need;
}

const p3 = (caps: Row["caps"]): Row => ({
	plane: "live",
	caps,
	keys: "●",
	unlocked: "●",
	session: "●",
});
const p4 = (keys: Row["keys"], unlocked: Row["unlocked"]): Row => ({
	plane: "local",
	caps: "–",
	keys,
	unlocked,
	session: "–",
});
const p1 = (caps: Row["caps"] = "–"): Row => ({
	plane: "hub",
	caps,
	keys: "–",
	unlocked: "–",
	session: "–",
});

/** IA §3.3, one entry per action. */
const IA_MATRIX: Record<ActionId, Row> = {
	view_list: p1(),
	view_cert_summary: p1({ any: ["status", "manage_certificates"] }),
	revoke_device: p1("owner"),
	rename_device: p1("owner"),
	setup_device: { ...p1(), plane: ["hub", "local"], keys: "creates" },
	cancel_setup: p1(),
	request_access: { ...p1(), plane: ["local", "hub"], keys: "creates" },
	fleet_monitor: {
		plane: "snap",
		caps: { any: ["status", "metrics"] },
		keys: "●",
		unlocked: "●",
		session: "–",
	},
	saved_inventory: {
		plane: "saved",
		caps: ["status"],
		keys: "●",
		unlocked: "●",
		session: "–",
	},
	connect_live: { ...p3([]), session: "–" },
	refresh_status: p3(["status"]),
	start: p3(["start"]),
	stop: p3(["stop"]),
	restart: p3(["restart"]),
	scale: p3(["scale"]),
	remove_service: p3(["remove"]),
	upload_revision: p3(["deploy"]),
	create_service: p3(["deploy"]),
	update_service: p3(["deploy"]),
	update_with_checks: p3(["deploy", "start"]),
	activate_staged: p3(["deploy", "start"]),
	discard_staged: p3(["deploy"]),
	set_secret: p3(["deploy"]),
	change_tls: p3(["manage_certificates"]),
	advanced_config: p3(["deploy"]),
	live_metrics: p3(["metrics"]),
	project_metrics: p3(["metrics"]),
	logs: p3(["logs"]),
	messages: p3(["logs"]),
	check_unconfirmed: p3([]),
	group_metrics_read: p3(["metrics"]),
	approve_metric_readers: p3("owner"),
	reset_metric_reader: {
		plane: ["local", "live"],
		caps: [],
		keys: "●",
		unlocked: "password",
		session: "–",
	},
	approve_history_readers: p3("owner"),
	history_read_device: p3({ any: ["logs", "metrics"] }),
	history_read_cloud: {
		plane: ["hub", "snap"],
		caps: [],
		keys: "●",
		unlocked: "●",
		session: "–",
	},
	share_access: { ...p1("owner"), keys: "●", unlocked: "password" },
	agent_update: p3(["update_agent"]),
	reboot: p3(["reboot"]),
	certificates_list: p3(["status"]),
	certificate_import: p3(["manage_certificates"]),
	certificate_delete: p3(["manage_certificates"]),
	csr_create: p3(["manage_certificates"]),
	csr_install: p3(["manage_certificates"]),
	sign_with_org_ca: p4("–", "–"),
	renewal_delegation: p3("owner"),
	acme_configure: p3("owner"),
	org_ca_manage: p4("–", "–"),
	certificate_reminders: p1(),
	offline_queue_read: p3(["status"]),
	offline_queue_retry: p3(["deploy"]),
	offline_queue_skip: p3(["deploy"]),
	cloud_access_create: p1(["deploy"]),
	cloud_access_revoke: p1([]),
	spending_limit_create: p1([]),
	spending_limit_revoke: p1([]),
	account_backup_save: {
		...p1([]),
		plane: ["hub", "local"],
		keys: "●",
		unlocked: "password",
	},
	account_backup_restore: {
		...p1([]),
		plane: ["hub", "local"],
		keys: "creates",
		unlocked: "password",
	},
	import_key_file: p4("creates", "password"),
	download_key_file: p4("●", "●"),
	change_device_password: p4("●", "password"),
	delete_local_keys: p4("●", "–"),
	forget_identity: p4("–", "–"),
};

describe("IA §3.3 action matrix", () => {
	test("every action is ready for an owner with everything in place", () => {
		const results = evaluateGates(ACTIONS, ctx());
		const failing = ACTIONS.filter((action) => !results[action].ok);
		expect(failing).toEqual([]);
	});

	for (const action of ACTIONS) {
		const row = IA_MATRIX[action];
		test(`${action}: plane, keys, unlock, session and capability gates`, () => {
			expect(ACTION_GATES[action].plane).toEqual(row.plane);

			const noKeys = evaluateGate(
				action,
				ctx({ keys: keys({ state: "none" }) }),
			);
			if (row.keys === "●") expect(failure(noKeys).gate).toBe("G6");
			else expect(noKeys.ok).toBe(true);

			const locked = evaluateGate(
				action,
				ctx({ keys: keys({ state: "locked" }) }),
			);
			if (row.unlocked === "●") {
				expect(failure(locked).gate).toBe("G7");
				expect(failure(locked).kind).toBe("locked");
				expect(failure(locked).fix?.kind).toBe("unlock");
			} else expect(locked.ok).toBe(true);

			const idle = evaluateGate(action, ctx({ live: { kind: "idle" } }));
			if (row.session === "●") {
				expect(failure(idle).gate).toBe("G8");
				expect(failure(idle).copy.code).toBe("connect_first");
			} else expect(idle.ok).toBe(true);

			const bare = evaluateGate(action, recipient([]));
			if (row.caps === "owner") {
				expect(failure(bare).kind).toBe("owner");
				expect(failure(bare).copy.code).toBe("owner_only_device");
			} else if (
				row.caps === "–" ||
				(Array.isArray(row.caps) && !row.caps.length)
			)
				expect(bare.ok).toBe(true);
			else {
				const need = Array.isArray(row.caps) ? row.caps : row.caps.any;
				expect(failure(bare).gate).toBe("G5");
				expect(failure(bare).need).toEqual(need);
				const granted = evaluateGate(action, recipient(need));
				expect(granted.ok).toBe(true);
			}
		});
	}
});

describe("gate ladder", () => {
	test("the first failing gate explains the state", () => {
		const result = failure(
			evaluateGate(
				"start",
				ctx({
					auth: { signedIn: false, tokenScopeAll: false },
					hub: { state: "off" },
					keys: keys({ state: "locked" }),
				}),
			),
		);
		expect(result.gate).toBe("G1");
		expect(result.fix).toEqual({ kind: "sign_in" });
	});

	test("hub checking, off and unreachable are three states", () => {
		const codes = (["checking", "off", "unreachable"] as const).map(
			(state) =>
				failure(evaluateGate("view_list", ctx({ hub: { state } }))).copy.code,
		);
		expect(codes).toEqual([
			"hub_checking",
			"hub_devices_off",
			"hub_unreachable",
		]);
		expect(
			failure(evaluateGate("view_list", ctx({ hub: { state: "off" } }))).fix,
		).toEqual({ kind: "open_hub_status" });
	});

	test("a restricted token fails G3 with the full-token fix", () => {
		const result = failure(
			evaluateGate(
				"view_list",
				ctx({ auth: { signedIn: true, tokenScopeAll: false } }),
			),
		);
		expect(result).toMatchObject({
			gate: "G3",
			kind: "noaccess",
			copy: { code: "token_restricted" },
			fix: { kind: "use_full_token" },
		});
	});

	test("local-only actions work without the hub", () => {
		const offline = ctx({
			auth: { signedIn: false, tokenScopeAll: false },
			hub: { state: "unreachable" },
		});
		expect(evaluateGate("delete_local_keys", offline).ok).toBe(true);
		expect(evaluateGate("import_key_file", offline).ok).toBe(true);
		expect(evaluateGate("start", offline).ok).toBe(false);
	});

	test("needs Deploy on this project and names what the recipient has", () => {
		const result = failure(
			evaluateGate(
				"upload_revision",
				recipient([], {
					target: { projectId: "invoice-ai" },
					capabilities: [
						{
							scope: { kind: "project", project_id: "invoice-ai" },
							caps: ["metrics", "status"],
						},
						{
							scope: { kind: "project", project_id: "other" },
							caps: ["deploy"],
						},
					],
				}),
			),
		);
		expect(result).toMatchObject({
			gate: "G5",
			kind: "noaccess",
			hide: false,
			copy: { code: "needs_capability", params: { scope: "project" } },
			have: ["status", "metrics"],
			need: ["deploy"],
			fix: {
				kind: "ask_owner",
				deviceId: "dev-1",
				need: ["deploy"],
				scope: { kind: "project", project_id: "invoice-ai" },
			},
		});
	});

	test("placement grants cover only their own service", () => {
		const grant = {
			scope: {
				kind: "placement" as const,
				project_id: "invoice-ai",
				placement_id: "support-bot",
			},
			caps: ["start"] as Capability[],
		};
		const own = recipient([], {
			capabilities: [grant],
			target: { projectId: "invoice-ai", placementId: "support-bot" },
		});
		expect(evaluateGate("start", own).ok).toBe(true);
		const other = {
			...own,
			target: { projectId: "invoice-ai", placementId: "x" },
		};
		expect(failure(evaluateGate("start", other)).copy.params).toEqual({
			scope: "service",
		});
	});

	test("device-only capabilities held for a project ask for the whole device", () => {
		const result = failure(
			evaluateGate(
				"certificate_import",
				recipient([], {
					target: { projectId: "invoice-ai" },
					capabilities: [
						{
							scope: { kind: "project", project_id: "invoice-ai" },
							caps: ["manage_certificates"],
						},
					],
				}),
			),
		);
		expect(result.copy.code).toBe("needs_device_scope");
		expect(result.fix).toMatchObject({ scope: { kind: "device" } });
	});

	test("restart of a stopped service also needs Start", () => {
		const result = failure(
			evaluateGate(
				"restart",
				recipient(["restart"], { extra: { desiredState: "stopped" } }),
			),
		);
		expect(result.need).toEqual(["start"]);
		expect(
			evaluateGate(
				"restart",
				recipient(["restart"], { extra: { desiredState: "running" } }),
			).ok,
		).toBe(true);
	});

	test("activating a staged update needs Deploy and Start, as the device checks", () => {
		const staged = { extra: { rolloutState: "staged" as const } };
		const startOnly = failure(
			evaluateGate("activate_staged", recipient(["start"], staged)),
		);
		expect(startOnly).toMatchObject({
			gate: "G5",
			copy: { code: "needs_capability" },
			need: ["deploy"],
		});
		expect(
			evaluateGate("activate_staged", recipient(["deploy", "start"], staged))
				.ok,
		).toBe(true);
	});

	test("before unlock a recipient is asked to unlock, never told 'no access'", () => {
		const result = failure(
			evaluateGate(
				"start",
				recipient([], {
					capabilities: undefined,
					keys: keys({ state: "locked", role: "shared" }),
				}),
			),
		);
		expect(result).toMatchObject({
			gate: "G7",
			kind: "locked",
			copy: { code: "unlock_to_check_permissions" },
			fix: { kind: "unlock", deviceId: "dev-1", connectLive: true },
		});
		expect(
			evaluateGate(
				"start",
				recipient([], { capabilities: undefined, relationship: "unknown" }),
			).ok,
		).toBe(true);
	});

	test("the no-keys pre-check happens before any password prompt", () => {
		const result = failure(
			evaluateGate(
				"connect_live",
				recipient(["status"], {
					keys: keys({ state: "none", role: "shared" }),
				}),
			),
		);
		expect(result).toMatchObject({
			gate: "G6",
			kind: "nokeys",
			copy: { code: "no_keys_here" },
			fix: { kind: "restore_keys", deviceId: "dev-1" },
		});
		expect(
			failure(evaluateGate("logs", ctx({ keys: keys({ state: "stale" }) })))
				.copy.code,
		).toBe("keys_unusable");
	});

	test("lock holder, identity block and browser support explain G7", () => {
		const code = (patch: Partial<KeySessionSnapshot>) =>
			failure(evaluateGate("logs", ctx({ keys: keys(patch) }))).copy.code;
		expect(code({ state: "held_elsewhere" })).toBe("held_elsewhere");
		expect(code({ state: "blocked" })).toBe("identity_blocked");
		expect(code({ state: "unlocking" })).toBe("unlocking");
		expect(
			code({ state: "locked", lastError: { code: "lock_unsupported" } }),
		).toBe("lock_unsupported");
		expect(code({ state: "locked" })).toBe("locked_logs");
		expect(
			failure(
				evaluateGate(
					"share_access",
					ctx({ keys: keys({ state: "held_elsewhere" }) }),
				),
			).fix,
		).toEqual({ kind: "take_over", deviceId: "dev-1" });
	});

	test("relationship: none, cloud approval only, ended access, revoked device", () => {
		const none = failure(evaluateGate("logs", ctx({ relationship: "none" })));
		expect(none.copy.code).toBe("not_shared_with_you");
		expect(none.fix).toEqual({ kind: "request_access", deviceId: "dev-1" });

		expect(
			failure(evaluateGate("logs", ctx({ relationship: "cloud_approval" })))
				.copy.code,
		).toBe("not_shared_with_you");
		expect(
			evaluateGate(
				"cloud_access_revoke",
				ctx({ relationship: "cloud_approval" }),
			).ok,
		).toBe(true);

		const ended = failure(
			evaluateGate(
				"logs",
				recipient(["logs"], {
					device: device({ owner_id: "x", access_expires_at: NOW - 60 }),
				}),
			),
		);
		expect(ended).toMatchObject({
			gate: "G4",
			copy: { code: "access_ended", params: { at: NOW - 60 } },
			fix: { kind: "ask_to_renew", deviceId: "dev-1" },
		});

		const expiredGrants = failure(
			evaluateGate(
				"logs",
				recipient([], {
					capabilities: [
						{ scope: { kind: "device" }, caps: ["logs"], expiresAt: NOW - 5 },
					],
				}),
			),
		);
		expect(expiredGrants.copy.code).toBe("access_ended");

		const revoked = ctx({
			device: device({ status: "revoked", revoked_at: NOW - 3600 }),
		});
		expect(failure(evaluateGate("start", revoked)).copy).toEqual({
			code: "device_revoked",
			params: { device: "edge-berlin-01", at: NOW - 3600 },
		});
		expect(evaluateGate("cloud_access_revoke", revoked).ok).toBe(true);
	});

	test("live session: offline, never connected, connecting, not configured", () => {
		const idle: LiveState = { kind: "idle" };
		const code = (patch: Partial<GateContext>) =>
			failure(evaluateGate("logs", ctx({ live: idle, ...patch }))).copy.code;
		expect(code({ device: device({ last_seen_at: NOW - 3600 }) })).toBe(
			"offline_needs_live",
		);
		expect(code({ device: device({ last_seen_at: null }) })).toBe(
			"never_connected_needs_live",
		);
		expect(
			code({
				live: { kind: "connecting", step: "unlocking_keys", startedAt: NOW },
			}),
		).toBe("connecting");
		const notConfigured: LiveState = {
			kind: "failed",
			cause: { step: "getting_pass", code: "not_configured", status: 503 },
		};
		expect(code({ live: notConfigured })).toBe("connection_not_configured");
		expect(
			failure(evaluateGate("connect_live", ctx({ live: notConfigured }))).copy
				.code,
		).toBe("connection_not_configured");
		expect(
			code({
				live: {
					kind: "failed",
					cause: { step: "securing", code: "handshake_failed" },
				},
			}),
		).toBe("connection_failed");
		expect(
			evaluateGate(
				"connect_live",
				ctx({ live: idle, device: device({ last_seen_at: NOW - 3600 }) }),
			).ok,
		).toBe(true);
	});

	test("G9 reads feature flags, never agent_version", () => {
		const features = ctx().features as NonNullable<GateContext["features"]>;
		const old = failure(
			evaluateGate(
				"certificates_list",
				ctx({
					features: {
						...features,
						certificateManagement: undefined,
						agentVersion: "99.0.0",
					},
				}),
			),
		);
		expect(old).toMatchObject({
			gate: "G9",
			kind: "unsupported",
			copy: { code: "agent_update_needed", params: { version: "99.0.0" } },
			fix: { kind: "update_agent", deviceId: "dev-1" },
		});
		expect(
			evaluateGate(
				"certificates_list",
				ctx({ features: { ...features, agentVersion: "0.0.1" } }),
			).ok,
		).toBe(true);
		expect(
			failure(evaluateGate("certificates_list", ctx({ features: undefined })))
				.copy.code,
		).toBe("agent_features_unknown");
	});

	test("Linux-only host operations are hidden elsewhere, whatever else fails", () => {
		const features = ctx().features as NonNullable<GateContext["features"]>;
		const mac = ctx({
			keys: keys({ state: "locked" }),
			features: {
				...features,
				os: "macos",
				hostOperations: { reboot: false, update_agent: false },
			},
		});
		for (const action of ["reboot", "agent_update"] as const)
			expect(failure(evaluateGate(action, mac))).toMatchObject({
				gate: "G9",
				hide: true,
				copy: { code: "needs_linux_systemd", params: { os: "macos" } },
			});
		const older = ctx({ features: { ...features, hostOperations: undefined } });
		expect(failure(evaluateGate("reboot", older)).hide).toBe(false);
	});

	test("rollout sources gate safe updates of online services", () => {
		const features = ctx().features as NonNullable<GateContext["features"]>;
		const offlineOnly = ctx({
			features: { ...features, rolloutSources: undefined },
			extra: { source: "online" },
		});
		expect(
			failure(evaluateGate("update_with_checks", offlineOnly)).copy.code,
		).toBe("rollout_source_unsupported");
		expect(
			evaluateGate("update_with_checks", {
				...offlineOnly,
				extra: { source: "offline" },
			}).ok,
		).toBe(true);
	});

	test("host_isolation required preselects the sandbox and refuses trusted processes", () => {
		expect(isolationDefault("required")).toEqual({
			profile: "linux_sandbox",
			locked: true,
			reason: "sandbox_required",
		});
		expect(isolationDefault("optional")).toEqual({
			profile: "trusted_process",
			locked: false,
		});
		expect(isolationDefault("none").locked).toBe(true);
		const result = failure(
			evaluateGate(
				"create_service",
				ctx({
					hostIsolation: "required",
					extra: { isolationProfile: "trusted_process" },
				}),
			),
		);
		expect(result).toMatchObject({
			gate: "G10",
			kind: "policy",
			copy: { code: "sandbox_required" },
		});
		expect(
			evaluateGate(
				"create_service",
				ctx({
					hostIsolation: "required",
					extra: { isolationProfile: "linux_sandbox" },
				}),
			).ok,
		).toBe(true);
	});

	test("web blocks offline local-project upload before anything else", () => {
		const result = failure(
			evaluateGate(
				"upload_revision",
				ctx({
					platform: "web",
					keys: keys({ state: "locked" }),
					extra: { offlineLocalApp: true },
				}),
			),
		);
		expect(result).toMatchObject({
			gate: "G0",
			kind: "platform",
			copy: { code: "desktop_only_offline_app" },
			fix: { kind: "use_desktop" },
		});
		expect(
			evaluateGate(
				"upload_revision",
				ctx({ platform: "desktop", extra: { offlineLocalApp: true } }),
			).ok,
		).toBe(true);
		const tooBig = failure(
			evaluateGate(
				"create_service",
				ctx({
					platform: "web",
					extra: {
						browserUploadBytes: {
							largestFile: 65 * 1024 ** 2,
							total: 70 * 1024 ** 2,
						},
					},
				}),
			),
		);
		expect(tooBig.copy.code).toBe("browser_size_limit");
	});

	test("owner-only powers give a read-only explanation", () => {
		const recipientResult = failure(
			evaluateGate("acme_configure", recipient(["manage_certificates"])),
		);
		expect(recipientResult).toMatchObject({
			gate: "G4",
			kind: "owner",
			hide: false,
			copy: { code: "owner_only_device", params: { owner: "Felix" } },
		});
		const elsewhere = failure(
			evaluateGate("acme_configure", ctx({ ownerPowers: false })),
		);
		expect(elsewhere).toMatchObject({
			gate: "G13",
			copy: { code: "owner_keys_elsewhere" },
			fix: { kind: "restore_keys", deviceId: "dev-1" },
		});
	});

	test("plan tier and project role", () => {
		expect(
			failure(
				evaluateGate(
					"history_read_cloud",
					ctx({ planTier: { storesHistory: false, tierName: "Free" } }),
				),
			),
		).toMatchObject({
			gate: "G11",
			kind: "plan",
			copy: { code: "plan_no_history", params: { tier: "Free" } },
			fix: { kind: "see_plans" },
		});
		const role = (patch: Partial<NonNullable<GateContext["projectRole"]>>) =>
			evaluateGate(
				"cloud_access_create",
				ctx({
					projectRole: {
						readBoards: true,
						admin: false,
						executeBoards: true,
						owner: false,
						...patch,
					},
				}),
			);
		expect(failure(role({})).copy.code).toBe("role_admin_execute");
		expect(role({ admin: true }).ok).toBe(true);
		expect(role({ owner: true, executeBoards: false }).ok).toBe(false);
	});

	test("busy preconditions come after every lasting gate", () => {
		const rollout = { activeRollout: true, rolloutDeadlineAt: NOW + 90 };
		expect(
			failure(evaluateGate("reboot", ctx({ extra: rollout }))).copy,
		).toEqual({
			code: "wait_for_rollout",
			params: { service: "support-bot", by: NOW + 90 },
		});
		expect(
			failure(
				evaluateGate(
					"reboot",
					ctx({ extra: rollout, keys: keys({ state: "locked" }) }),
				),
			).gate,
		).toBe("G7");
		expect(
			failure(evaluateGate("start", ctx({ extra: rollout }))).copy.code,
		).toBe("rollout_in_progress");
		expect(failure(evaluateGate("start", ctx({ extra: rollout }))).kind).toBe(
			"busy",
		);
		expect(evaluateGate("stop", ctx({ extra: rollout })).ok).toBe(true);
	});

	const PRECONDITIONS: [ActionId, GateContext["extra"], GateReason][] = [
		["set_secret", { desiredState: "running" }, "service_must_be_stopped"],
		["remove_service", { desiredState: "running" }, "service_must_be_stopped"],
		[
			"update_with_checks",
			{ desiredState: "stopped" },
			"service_must_be_running",
		],
		["update_with_checks", { sameSource: false }, "different_source"],
		["scale", { maxReplicas: 1 }, "single_instance_only"],
		["discard_staged", { rolloutState: "healthy" }, "no_staged_update"],
		["activate_staged", { rolloutState: "validating" }, "no_staged_update"],
		["certificate_delete", { bindingCount: 2 }, "certificate_in_use"],
		["change_tls", { certificateValidNow: false }, "certificate_not_valid_now"],
		["advanced_config", { configBytes: 12_001 }, "config_too_large"],
		["offline_queue_skip", { queueQuarantined: true }, "queue_quarantined"],
		[
			"check_unconfirmed",
			{ operationIssuedAt: NOW - 25 * 3600 },
			"operation_too_old",
		],
		["approve_history_readers", { policyApplied: false }, "policy_not_applied"],
		["share_access", { grantCount: 24 }, "access_slots_full"],
		["sign_with_org_ca", { authorityPresent: false }, "authority_missing"],
		["cloud_access_create", { approvalExists: true }, "approval_exists"],
		[
			"create_service",
			{ source: "online", approvalExists: false },
			"cloud_approval_required",
		],
		["agent_update", { releaseTrust: false }, "release_trust_missing"],
	];

	test("action preconditions name the state that blocks them", () => {
		for (const [action, extra, reason] of PRECONDITIONS)
			expect([
				action,
				failure(evaluateGate(action, ctx({ extra }))).copy.code,
			]).toEqual([action, reason]);
		expect(
			evaluateGate(
				"discard_staged",
				ctx({ extra: { rolloutState: "validating" } }),
			).ok,
		).toBe(true);
	});

	test("cloud approvals: delegator, payer and roster membership", () => {
		const shared = recipient([]);
		expect(
			failure(
				evaluateGate("cloud_access_revoke", {
					...shared,
					extra: { delegatorIsMe: false },
				}),
			).copy.code,
		).toBe("delegator_or_owner_only");
		expect(
			evaluateGate("cloud_access_revoke", {
				...shared,
				extra: { delegatorIsMe: true },
			}).ok,
		).toBe(true);
		expect(
			failure(
				evaluateGate(
					"spending_limit_revoke",
					ctx({ extra: { payerIsMe: false } }),
				),
			).copy.code,
		).toBe("payer_only");
		expect(
			failure(
				evaluateGate(
					"group_metrics_read",
					recipient(["metrics"], { extra: { rosterMember: false } }),
				),
			).copy.code,
		).toBe("not_a_reader");
		expect(
			evaluateGate(
				"group_metrics_read",
				ctx({ extra: { rosterMember: false } }),
			).ok,
		).toBe(true);
	});

	const SETUP_LIMITS = {
		max_devices: 10,
		max_pending_enrollments: 10,
		enrollment_ttl_seconds: 86_400,
		max_enrollments_per_day: 20,
		max_account_backups: 10,
	};
	const SETUP_USAGE = {
		active_devices: 10,
		revoked_devices: 0,
		pending_enrollments: 0,
		enrollments_last_24h: 0,
		account_backups: 0,
	};
	const setupWith = (usage: Partial<typeof SETUP_USAGE>) =>
		evaluateGate(
			"setup_device",
			ctx({
				hub: {
					state: "on",
					limits: SETUP_LIMITS,
					usage: { ...SETUP_USAGE, ...usage },
				},
			}),
		);

	test("setup checks the hub's device quota, counting setups that haven't started", () => {
		expect(failure(setupWith({})).copy).toEqual({
			code: "device_limit_reached",
			params: { used: 10, max: 10, pending: 0 },
		});
		expect(
			failure(setupWith({ active_devices: 8, pending_enrollments: 2 })).copy,
		).toEqual({
			code: "device_limit_reached",
			params: { used: 10, max: 10, pending: 2 },
		});
		expect(setupWith({ active_devices: 3 }).ok).toBe(true);
	});

	test("setup checks readiness before quotas", () => {
		expect(
			failure(
				evaluateGate("setup_device", ctx({ extra: { readinessOk: false } })),
			).copy.code,
		).toBe("readiness_failing");
	});

	test("evaluateGates returns one result per requested action", () => {
		const results = evaluateGates(
			["start", "stop", "reboot"],
			ctx({ keys: keys({ state: "locked" }) }),
		);
		expect(Object.keys(results)).toEqual(["start", "stop", "reboot"]);
		expect(failure(results.start).copy.code).toBe("locked_change");
	});
});
