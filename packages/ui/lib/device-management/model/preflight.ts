import type {
	KeySessionSnapshot,
	LiveError,
	LocalSummary,
	Preflight,
	PreflightRow,
} from "../workspace/types";
import { CLOCK_SKEW_FLAG_S } from "./freshness";
import { presence } from "./presence";
import type {
	AgentFeature,
	AgentFeatures,
	CopyParams,
	DeviceRow,
	FixAction,
	HubDeviceSupport,
	PreflightCode,
	PreflightId,
	Relationship,
} from "./types";

type Checking = "checking";

/** Everything D1–D13 read; D10–D13 are omitted until connecting starts. */
export interface PreflightInput {
	/** Hub-corrected unix seconds. */
	now: number;
	deviceId: string;
	platform: "desktop" | "web";
	hub: HubDeviceSupport;
	auth: { signedIn: boolean; tokenScopeAll: boolean };
	/** Undefined when the hub list doesn't contain the device. */
	device?: DeviceRow;
	relationship: Relationship;
	keys: Pick<KeySessionSnapshot, "state" | "role">;
	local: Pick<
		LocalSummary,
		"persistence" | "webLocks" | "indexedDb" | "cryptoLoaded"
	>;
	lock: Checking | "free" | "held_here" | "held_elsewhere" | "unsupported";
	identity:
		| Checking
		| {
				check: "match" | "mismatch" | "unpinned";
				pinnedAt?: number;
				fingerprint?: string;
		  };
	clock: { hubOffsetS?: number; deviceSkewS?: number };
	connection?: Checking | "ready" | LiveError;
	policy?: Checking | { applied?: boolean; version?: number };
	slots?: Checking | { busy: boolean; limit?: number };
	features?:
		| Checking
		| {
				flags?: AgentFeatures;
				agentVersion?: string;
				required?: readonly AgentFeature[];
		  };
}

type RowSpec = Omit<PreflightRow, "id" | "source" | "copy"> & {
	code: PreflightCode;
	params?: CopyParams;
};

const SOURCES: Record<PreflightId, PreflightRow["source"]> = {
	D1: "hub",
	D2: "hub",
	D3: "hub",
	D4: "local",
	D5: "local",
	D6: "local",
	D7: "hub",
	D8: "hub",
	D9: "hub",
	D10: "live",
	D11: "hub",
	D12: "live",
	D13: "live",
};

const KEY_CHECKS = new Set<PreflightId>([
	"D1",
	"D2",
	"D3",
	"D4",
	"D5",
	"D6",
	"D7",
]);

const pass = (code: PreflightCode, params?: CopyParams): RowSpec => ({
	status: "pass",
	code,
	params,
});
const checking: RowSpec = { status: "checking", code: "checking" };

function hubCheck(hub: HubDeviceSupport): RowSpec {
	switch (hub.state) {
		case "checking":
			return checking;
		case "on":
			return pass("hub_ready");
		case "off":
			return {
				status: "block",
				code: "hub_devices_off",
				fix: { kind: "open_hub_status" },
			};
		case "unreachable":
			return {
				status: "fail",
				code: "hub_unreachable",
				fix: { kind: "open_hub_status" },
			};
	}
}

function authCheck(auth: PreflightInput["auth"]): RowSpec {
	if (!auth.signedIn)
		return {
			status: "block",
			code: "sign_in_required",
			fix: { kind: "sign_in" },
		};
	if (!auth.tokenScopeAll)
		return {
			status: "block",
			code: "token_restricted",
			fix: { kind: "use_full_token" },
		};
	return pass("signed_in");
}

function revokedRow(revokedAt: number | null | undefined): RowSpec {
	return revokedAt == null
		? { status: "block", code: "device_revoked" }
		: { status: "block", code: "device_revoked", params: { at: revokedAt } };
}

function sharedAccess(
	endsAt: number | null | undefined,
	now: number,
	deviceId: string,
): RowSpec {
	if (endsAt == null) return pass("access_shared");
	if (endsAt > now) return pass("access_shared", { endsAt });
	return {
		status: "fail",
		code: "access_ended",
		params: { at: endsAt },
		fix: { kind: "ask_to_renew", deviceId },
	};
}

function accessCheck(input: PreflightInput): RowSpec {
	const { device, deviceId } = input;
	if (!device) return { status: "fail", code: "access_unknown" };
	if (device.status === "revoked") return revokedRow(device.revoked_at);
	switch (input.relationship) {
		case "owner":
			return pass("access_owner");
		case "shared":
			return sharedAccess(device.access_expires_at, input.now, deviceId);
		case "cloud_approval":
			return { status: "fail", code: "access_cloud_approval" };
		case "none":
			return {
				status: "fail",
				code: "access_ended",
				fix: { kind: "request_access", deviceId },
			};
		case "unknown":
			return { status: "warn", code: "access_unknown" };
	}
}

function keysCheck(input: PreflightInput): RowSpec {
	const { deviceId, keys } = input;
	if (keys.state === "none") {
		const otherFixes: FixAction[] = [{ kind: "import_key_file", deviceId }];
		if (input.relationship !== "owner")
			otherFixes.push({ kind: "request_access", deviceId });
		return {
			status: "fail",
			code: "keys_missing",
			fix: { kind: "restore_keys", deviceId },
			otherFixes,
		};
	}
	if (keys.state === "stale")
		return {
			status: "fail",
			code: "keys_unusable",
			fix: { kind: "restore_keys", deviceId },
			otherFixes: [{ kind: "import_key_file", deviceId }],
		};
	return pass(keys.role === "owner" ? "keys_owner_here" : "keys_shared_here");
}

function browserCheck(input: PreflightInput): RowSpec {
	const { local } = input;
	if (local.cryptoLoaded === false)
		return { status: "fail", code: "crypto_unavailable" };
	if (!local.indexedDb)
		return { status: "fail", code: "browser_cannot_protect_keys" };
	if (input.platform === "web" && local.persistence !== "persisted")
		return {
			status: "warn",
			code: "browser_may_delete_keys",
			fix: { kind: "keep_keys_safely" },
		};
	return pass("browser_ready");
}

function lockCheck(input: PreflightInput): RowSpec {
	switch (input.lock) {
		case "checking":
			return checking;
		case "free":
			return pass("lock_free");
		case "held_here":
			return pass("lock_held_here");
		case "held_elsewhere":
			return {
				status: "fail",
				code: "lock_held_elsewhere",
				fix: { kind: "take_over", deviceId: input.deviceId },
			};
		case "unsupported":
			return {
				status: "fail",
				code: "lock_unsupported",
				fix: { kind: "use_desktop" },
			};
	}
}

function identityCheck(input: PreflightInput): RowSpec {
	const { identity, deviceId } = input;
	if (identity === "checking") return checking;
	const params: CopyParams = {};
	if (identity.pinnedAt !== undefined) params.since = identity.pinnedAt;
	if (identity.fingerprint) params.fingerprint = identity.fingerprint;
	if (identity.check === "mismatch")
		return {
			status: "block",
			code: "identity_mismatch",
			params,
			fix: { kind: "review_identity", deviceId },
			otherFixes: [{ kind: "forget_identity", deviceId }],
		};
	return pass(
		identity.check === "match" ? "identity_trusted" : "identity_first_use",
		params,
	);
}

function checkinCheck(input: PreflightInput): RowSpec {
	const { device } = input;
	if (!device) return checking;
	const seen = presence(
		{ status: "active", last_seen_at: device.last_seen_at },
		input.now,
	);
	if (seen.kind === "online") return pass("checkin_online");
	const diagnose: FixAction = { kind: "diagnose", deviceId: input.deviceId };
	if (seen.kind === "never")
		return { status: "warn", code: "checkin_never", fix: diagnose };
	const params = { since: device.last_seen_at ?? 0 };
	return seen.kind === "late"
		? { status: "warn", code: "checkin_late", params }
		: { status: "warn", code: "checkin_offline", params, fix: diagnose };
}

function clockCheck(input: PreflightInput): RowSpec {
	const { hubOffsetS } = input.clock;
	const rejection = input.device?.auth_rejection;
	const deviceSkewS =
		rejection?.code === "clock_skew" && rejection.skew_seconds != null
			? rejection.skew_seconds
			: input.clock.deviceSkewS;
	const minutes = (skew: number) =>
		Math.max(1, Math.round(Math.abs(skew) / 60));
	if (hubOffsetS !== undefined && Math.abs(hubOffsetS) > CLOCK_SKEW_FLAG_S)
		return {
			status: "warn",
			code: "clock_computer_off",
			params: { minutes: minutes(hubOffsetS), seconds: hubOffsetS },
			fix: { kind: "fix_clock" },
		};
	if (deviceSkewS !== undefined && Math.abs(deviceSkewS) > CLOCK_SKEW_FLAG_S)
		return {
			status: "warn",
			code: "clock_device_off",
			params: {
				device:
					input.device?.display_name || input.device?.name || input.deviceId,
				minutes: minutes(deviceSkewS),
				seconds: deviceSkewS,
			},
			fix: { kind: "fix_clock", deviceId: input.deviceId },
		};
	// Hubs without a server time never answer this before a live connection: not a spinner.
	if (hubOffsetS === undefined) return pass("clock_unknown");
	return pass("clock_ok");
}

function connectionCheck(
	connection: NonNullable<PreflightInput["connection"]>,
	deviceId: string,
): RowSpec {
	if (connection === "checking") return checking;
	if (connection === "ready") return pass("connection_ready");
	const hubFix: FixAction = { kind: "open_hub_status" };
	switch (connection.code) {
		case "not_configured":
			return { status: "fail", code: "connection_not_configured", fix: hubFix };
		case "needs_wss":
			return { status: "fail", code: "connection_needs_wss", fix: hubFix };
		case "access_expired":
			return {
				status: "fail",
				code: "connection_access_expired",
				fix: { kind: "ask_to_renew", deviceId },
			};
		case "epoch_mismatch":
		case "invalid_admission":
		case "http":
			return {
				status: "fail",
				code: "connection_invalid_admission",
				params:
					connection.step === "getting_pass" && connection.status
						? { status: connection.status }
						: undefined,
			};
		case "relay_unreachable":
			return {
				status: "fail",
				code: "relay_unreachable",
				params: connection.url ? { url: originOf(connection.url) } : undefined,
				fix: { kind: "diagnose", deviceId },
			};
		default:
			return {
				status: "fail",
				code: "connection_failed",
				fix: { kind: "diagnose", deviceId },
			};
	}
}

function policyCheck(policy: NonNullable<PreflightInput["policy"]>): RowSpec {
	if (policy === "checking") return checking;
	const params =
		policy.version === undefined ? undefined : { version: policy.version };
	if (policy.applied === true) return pass("policy_applied", params);
	if (policy.applied === false)
		return { status: "warn", code: "policy_waiting", params };
	return pass("policy_unknown", params);
}

function slotsCheck(slots: NonNullable<PreflightInput["slots"]>): RowSpec {
	if (slots === "checking") return checking;
	if (!slots.busy) return pass("slots_available");
	return {
		status: "warn",
		code: "slots_full",
		params: slots.limit === undefined ? undefined : { limit: slots.limit },
	};
}

function featuresCheck(
	features: NonNullable<PreflightInput["features"]>,
	deviceId: string,
): RowSpec {
	if (features === "checking") return checking;
	const version =
		features.agentVersion === undefined
			? undefined
			: { version: features.agentVersion };
	const flags = features.flags;
	if (!flags) return pass("agent_features_unknown", version);
	const missing = (features.required ?? []).filter((flag) => flags[flag] !== 1);
	if (missing.length === 0) return pass("agent_features_ok", version);
	return {
		status: "warn",
		code: "agent_features_missing",
		params: { ...version, count: missing.length, features: missing.join(", ") },
		fix: { kind: "update_agent", deviceId },
	};
}

function originOf(url: string): string {
	try {
		return new URL(url).origin;
	} catch {
		return "invalid URL";
	}
}

const SECRET_PATTERNS = [
	/-----BEGIN [A-Z ]+-----[\s\S]*?(-----END [A-Z ]+-----|$)/g,
	/\beyJ[\w-]{8,}(\.[\w-]+){0,2}/g,
	/\b(?:bearer|basic)\s+\S+/gi,
	/\b(?:pat|flp|sk|pk)_[\w-]{8,}/gi,
	/[A-Za-z0-9+/_-]{40,}={0,2}/g,
];

/** Strips anything that looks like a credential before it reaches a support report. */
export function redactSecrets(text: string): string {
	return SECRET_PATTERNS.reduce(
		(result, pattern) => result.replace(pattern, "[redacted]"),
		text,
	);
}

const FINGERPRINT = /^[0-9A-F ]+$/i;

function describeParams(params: CopyParams | undefined): string {
	if (!params) return "";
	return Object.entries(params)
		.map(([key, value]) => {
			const text = String(value);
			const safe =
				key === "fingerprint" && FINGERPRINT.test(text)
					? text
					: redactSecrets(text);
			return ` ${key}=${safe}`;
		})
		.join("");
}

function describeFix(fix: FixAction | undefined): string {
	return fix ? ` → ${fix.kind}` : "";
}

/** IA §6.4.3. Pure: D1–D9 always, D10–D13 once their inputs exist. */
export function runPreflight(input: PreflightInput): Preflight {
	const specs: [PreflightId, RowSpec][] = [
		["D1", hubCheck(input.hub)],
		["D2", authCheck(input.auth)],
		["D3", accessCheck(input)],
		["D4", keysCheck(input)],
		["D5", browserCheck(input)],
		["D6", lockCheck(input)],
		["D7", identityCheck(input)],
		["D8", checkinCheck(input)],
		["D9", clockCheck(input)],
	];
	if (input.connection !== undefined)
		specs.push(["D10", connectionCheck(input.connection, input.deviceId)]);
	if (input.policy !== undefined)
		specs.push(["D11", policyCheck(input.policy)]);
	if (input.slots !== undefined) specs.push(["D12", slotsCheck(input.slots)]);
	if (input.features !== undefined)
		specs.push(["D13", featuresCheck(input.features, input.deviceId)]);

	const rows: PreflightRow[] = specs.map(([id, spec]) => {
		const { code, params, ...rest } = spec;
		const row: PreflightRow = {
			id,
			source: SOURCES[id],
			copy: params && Object.keys(params).length ? { code, params } : { code },
			status: rest.status,
		};
		if (rest.fix) row.fix = rest.fix;
		if (rest.otherFixes?.length) row.otherFixes = rest.otherFixes;
		return row;
	});

	const passwordEnabled = !rows.some(
		(row) =>
			KEY_CHECKS.has(row.id) &&
			(row.status === "block" || row.status === "fail"),
	);
	const checkin = rows.find((row) => row.id === "D8")?.copy.code;
	const suggestConnectLive =
		input.device?.status !== "revoked" &&
		checkin !== "checkin_offline" &&
		checkin !== "checkin_never";

	return {
		rows,
		passwordEnabled,
		suggestConnectLive,
		diagnostics: () =>
			[
				`Flow-Like device pre-flight · device ${redactSecrets(input.deviceId)} · ${new Date(input.now * 1_000).toISOString()} · ${input.platform}`,
				`Hub: ${input.hub.state}${input.hub.error ? ` (${input.hub.error.code})` : ""} · unlock ${passwordEnabled ? "allowed" : "blocked"} · connect live ${suggestConnectLive ? "suggested" : "not suggested"}`,
				...rows.map(
					(row) =>
						`${row.id} ${row.status} ${row.copy.code}${describeParams(row.copy.params)}${describeFix(row.fix)}`,
				),
			].join("\n"),
	};
}
