import { describe, expect, test } from "bun:test";
import { type PreflightInput, redactSecrets, runPreflight } from "./preflight";
import type { DeviceRow, PreflightId } from "./types";

const NOW = 1_790_000_000;

const device = (patch: Partial<DeviceRow> = {}): DeviceRow => ({
	device_id: "dev-1",
	owner_id: "me",
	name: "edge-berlin-01",
	status: "active",
	registered_at: NOW - 86_400,
	last_seen_at: NOW - 30,
	auth_epoch: 1,
	identity: {} as DeviceRow["identity"],
	...patch,
});

const input = (patch: Partial<PreflightInput> = {}): PreflightInput => ({
	now: NOW,
	deviceId: "dev-1",
	platform: "desktop",
	hub: { state: "on" },
	auth: { signedIn: true, tokenScopeAll: true },
	device: device(),
	relationship: "owner",
	keys: { state: "locked", role: "owner" },
	local: {
		persistence: "persisted",
		webLocks: true,
		indexedDb: true,
		cryptoLoaded: true,
	},
	lock: "free",
	identity: {
		check: "match",
		pinnedAt: NOW - 86_400,
		fingerprint: "3F2A 91C0",
	},
	clock: { hubOffsetS: 2 },
	...patch,
});

const row = (preflight: ReturnType<typeof runPreflight>, id: PreflightId) => {
	const found = preflight.rows.find((candidate) => candidate.id === id);
	if (!found) throw new Error(`row ${id} missing`);
	return found;
};

describe("runPreflight", () => {
	test("all clear: D1–D9 pass, password enabled, connect live suggested", () => {
		const preflight = runPreflight(input());
		expect(preflight.rows.map((r) => r.id)).toEqual([
			"D1",
			"D2",
			"D3",
			"D4",
			"D5",
			"D6",
			"D7",
			"D8",
			"D9",
		]);
		expect(preflight.rows.every((r) => r.status === "pass")).toBe(true);
		expect(preflight.passwordEnabled).toBe(true);
		expect(preflight.suggestConnectLive).toBe(true);
	});

	test("D1 hub: checking, off and unreachable are three states", () => {
		expect(
			row(runPreflight(input({ hub: { state: "checking" } })), "D1").status,
		).toBe("checking");
		const off = row(runPreflight(input({ hub: { state: "off" } })), "D1");
		expect(off).toMatchObject({
			status: "block",
			copy: { code: "hub_devices_off" },
			fix: { kind: "open_hub_status" },
		});
		const down = row(
			runPreflight(input({ hub: { state: "unreachable" } })),
			"D1",
		);
		expect(down).toMatchObject({
			status: "fail",
			copy: { code: "hub_unreachable" },
		});
		expect(runPreflight(input({ hub: { state: "off" } })).passwordEnabled).toBe(
			false,
		);
	});

	test("D2 sign-in and restricted token", () => {
		expect(
			row(
				runPreflight(
					input({ auth: { signedIn: false, tokenScopeAll: false } }),
				),
				"D2",
			),
		).toMatchObject({
			status: "block",
			copy: { code: "sign_in_required" },
			fix: { kind: "sign_in" },
		});
		expect(
			row(
				runPreflight(input({ auth: { signedIn: true, tokenScopeAll: false } })),
				"D2",
			),
		).toMatchObject({
			status: "block",
			copy: { code: "token_restricted" },
			fix: { kind: "use_full_token" },
		});
	});

	test("D3 access: shared with end, ended, revoked, unknown on older hubs", () => {
		const shared = runPreflight(
			input({
				relationship: "shared",
				device: device({
					owner_id: "other",
					relationship: "shared",
					access_expires_at: NOW + 3_600,
				}),
				keys: { state: "locked", role: "shared" },
			}),
		);
		expect(row(shared, "D3")).toMatchObject({
			status: "pass",
			copy: { code: "access_shared", params: { endsAt: NOW + 3_600 } },
		});
		expect(row(shared, "D4").copy.code).toBe("keys_shared_here");

		const ended = runPreflight(
			input({
				relationship: "shared",
				device: device({ access_expires_at: NOW - 1 }),
			}),
		);
		expect(row(ended, "D3")).toMatchObject({
			status: "fail",
			copy: { code: "access_ended" },
			fix: { kind: "ask_to_renew", deviceId: "dev-1" },
		});
		expect(ended.passwordEnabled).toBe(false);

		const revoked = runPreflight(
			input({ device: device({ status: "revoked", revoked_at: NOW - 60 }) }),
		);
		expect(row(revoked, "D3")).toMatchObject({
			status: "block",
			copy: { code: "device_revoked", params: { at: NOW - 60 } },
		});
		expect(revoked.suggestConnectLive).toBe(false);

		const unknown = runPreflight(input({ relationship: "unknown" }));
		expect(row(unknown, "D3")).toMatchObject({
			status: "warn",
			copy: { code: "access_unknown" },
		});
		expect(unknown.passwordEnabled).toBe(true);
	});

	test("D4 no keys offers Restore, Import and Request before any password prompt", () => {
		const missing = runPreflight(
			input({
				relationship: "shared",
				keys: { state: "none", role: "shared" },
			}),
		);
		const d4 = row(missing, "D4");
		expect(d4).toMatchObject({
			status: "fail",
			copy: { code: "keys_missing" },
			fix: { kind: "restore_keys", deviceId: "dev-1" },
		});
		expect(d4.otherFixes?.map((fix) => fix.kind)).toEqual([
			"import_key_file",
			"request_access",
		]);
		expect(missing.passwordEnabled).toBe(false);

		const owner = row(
			runPreflight(input({ keys: { state: "none", role: "owner" } })),
			"D4",
		);
		expect(owner.otherFixes?.map((fix) => fix.kind)).toEqual([
			"import_key_file",
		]);

		expect(
			row(
				runPreflight(input({ keys: { state: "stale", role: "owner" } })),
				"D4",
			).copy.code,
		).toBe("keys_unusable");
	});

	test("D5 browser support", () => {
		const web = (local: Partial<PreflightInput["local"]>) =>
			row(
				runPreflight(
					input({ platform: "web", local: { ...input().local, ...local } }),
				),
				"D5",
			);
		expect(web({ persistence: "unknown" })).toMatchObject({
			status: "warn",
			copy: { code: "browser_may_delete_keys" },
			fix: { kind: "keep_keys_safely" },
		});
		expect(web({ indexedDb: false })).toMatchObject({
			status: "fail",
			copy: { code: "browser_cannot_protect_keys" },
		});
		expect(web({ cryptoLoaded: false })).toMatchObject({
			status: "fail",
			copy: { code: "crypto_unavailable" },
		});
		expect(web({}).status).toBe("pass");
		expect(
			row(
				runPreflight(
					input({ local: { ...input().local, persistence: "unknown" } }),
				),
				"D5",
			).status,
		).toBe("pass");
	});

	test("D6 lock holder", () => {
		expect(
			row(runPreflight(input({ lock: "held_here" })), "D6").copy.code,
		).toBe("lock_held_here");
		const elsewhere = runPreflight(input({ lock: "held_elsewhere" }));
		expect(row(elsewhere, "D6")).toMatchObject({
			status: "fail",
			fix: { kind: "take_over", deviceId: "dev-1" },
		});
		expect(elsewhere.passwordEnabled).toBe(false);
		expect(
			row(runPreflight(input({ lock: "unsupported" })), "D6"),
		).toMatchObject({ status: "fail", copy: { code: "lock_unsupported" } });
	});

	test("D7 identity mismatch is a hard block", () => {
		const mismatch = runPreflight(
			input({
				identity: {
					check: "mismatch",
					pinnedAt: NOW - 86_400,
					fingerprint: "3F2A 91C0",
				},
			}),
		);
		const d7 = row(mismatch, "D7");
		expect(d7).toMatchObject({
			status: "block",
			copy: {
				code: "identity_mismatch",
				params: { since: NOW - 86_400, fingerprint: "3F2A 91C0" },
			},
			fix: { kind: "review_identity", deviceId: "dev-1" },
			otherFixes: [{ kind: "forget_identity", deviceId: "dev-1" }],
		});
		expect(mismatch.passwordEnabled).toBe(false);
		expect(
			row(runPreflight(input({ identity: { check: "unpinned" } })), "D7"),
		).toMatchObject({ status: "pass", copy: { code: "identity_first_use" } });
	});

	test("D8 offline warns that snapshots still work and stops suggesting live", () => {
		const offline = runPreflight(
			input({ device: device({ last_seen_at: NOW - 3_600 }) }),
		);
		expect(row(offline, "D8")).toMatchObject({
			status: "warn",
			copy: { code: "checkin_offline", params: { since: NOW - 3_600 } },
		});
		expect(offline.passwordEnabled).toBe(true);
		expect(offline.suggestConnectLive).toBe(false);
		const never = runPreflight(
			input({ device: device({ last_seen_at: null }) }),
		);
		expect(row(never, "D8").copy.code).toBe("checkin_never");
		expect(never.suggestConnectLive).toBe(false);
		const late = runPreflight(
			input({ device: device({ last_seen_at: NOW - 300 }) }),
		);
		expect(row(late, "D8").copy.code).toBe("checkin_late");
		expect(late.suggestConnectLive).toBe(true);
	});

	test("D9 clocks: computer offset from the Date header, device skew from BG6", () => {
		expect(
			row(runPreflight(input({ clock: { hubOffsetS: 240 } })), "D9"),
		).toMatchObject({
			status: "warn",
			copy: { code: "clock_computer_off", params: { minutes: 4 } },
		});
		expect(
			row(
				runPreflight(
					input({
						device: device({
							auth_rejection: {
								code: "clock_skew",
								skew_seconds: -420,
								count: 3,
								first_at: NOW - 600,
								last_at: NOW - 60,
							},
						}),
					}),
				),
				"D9",
			),
		).toMatchObject({
			status: "warn",
			copy: {
				code: "clock_device_off",
				params: { device: "edge-berlin-01", minutes: 7 },
			},
		});
		expect(
			row(runPreflight(input({ clock: { hubOffsetS: 120 } })), "D9").copy.code,
		).toBe("clock_ok");
		// An older hub has no server time before a live connection: resolved, never a spinner.
		expect(row(runPreflight(input({ clock: {} })), "D9")).toMatchObject({
			status: "pass",
			copy: { code: "clock_unknown" },
		});
	});

	test("D10 maps 503, 401 and relay failures", () => {
		const d10 = (connection: PreflightInput["connection"]) =>
			row(runPreflight(input({ connection })), "D10");
		expect(d10("ready").copy.code).toBe("connection_ready");
		expect(d10("checking").status).toBe("checking");
		expect(
			d10({ step: "getting_pass", code: "not_configured", status: 503 }),
		).toMatchObject({
			copy: { code: "connection_not_configured" },
			fix: { kind: "open_hub_status" },
		});
		expect(
			d10({ step: "getting_pass", code: "needs_wss", status: 503 }).copy.code,
		).toBe("connection_needs_wss");
		expect(
			d10({ step: "getting_pass", code: "access_expired", status: 401 }),
		).toMatchObject({
			copy: { code: "connection_access_expired" },
			fix: { kind: "ask_to_renew" },
		});
		expect(
			d10({ step: "getting_pass", code: "invalid_admission" }).copy.code,
		).toBe("connection_invalid_admission");
		expect(
			d10({
				step: "reaching_device",
				code: "relay_unreachable",
				url: "wss://relay.example.com/path?token=abc",
			}),
		).toMatchObject({
			copy: {
				code: "relay_unreachable",
				params: { url: "wss://relay.example.com" },
			},
		});
		expect(d10({ step: "securing", code: "handshake_failed" }).copy.code).toBe(
			"connection_failed",
		);
	});

	test("D11–D13 only appear once evaluated", () => {
		const connected = runPreflight(
			input({
				connection: "ready",
				policy: { applied: false, version: 2 },
				slots: { busy: true, limit: 2 },
				features: {
					flags: { task_health: 1 },
					agentVersion: "0.9.3",
					required: ["task_health", "placement_events"],
				},
			}),
		);
		expect(connected.rows.map((r) => r.id).slice(9)).toEqual([
			"D10",
			"D11",
			"D12",
			"D13",
		]);
		expect(row(connected, "D11")).toMatchObject({
			status: "warn",
			copy: { code: "policy_waiting", params: { version: 2 } },
		});
		expect(row(connected, "D12")).toMatchObject({
			status: "warn",
			copy: { code: "slots_full", params: { limit: 2 } },
		});
		expect(row(connected, "D13")).toMatchObject({
			status: "warn",
			copy: {
				code: "agent_features_missing",
				params: { count: 1, version: "0.9.3" },
			},
			fix: { kind: "update_agent" },
		});
		expect(connected.passwordEnabled).toBe(true);
	});

	test("D11–D13 pass once the device confirms", () => {
		const ok = runPreflight(
			input({
				policy: { applied: true },
				slots: { busy: false },
				features: { flags: {}, agentVersion: "0.9.3" },
			}),
		);
		expect(row(ok, "D11").copy.code).toBe("policy_applied");
		expect(row(ok, "D12").copy.code).toBe("slots_available");
		expect(row(ok, "D13")).toMatchObject({
			copy: { code: "agent_features_ok", params: { version: "0.9.3" } },
		});
		expect(row(runPreflight(input({ features: {} })), "D13").copy.code).toBe(
			"agent_features_unknown",
		);
	});

	test("rows carry no machine wording beyond codes and copy params", () => {
		for (const r of runPreflight(input({ connection: "ready" })).rows) {
			expect(r.copy.code).toMatch(/^[a-z_]+$/);
			expect(["hub", "local", "live"]).toContain(r.source);
		}
	});
});

describe("diagnostics", () => {
	const secretPatterns = [
		/password/i,
		/-----BEGIN/,
		/eyJ[\w-]{8,}/,
		/token=/i,
		/bearer\s/i,
	];

	test("the support report lists every row and contains no secrets", () => {
		const report = runPreflight(
			input({
				device: device({
					name: "-----BEGIN PRIVATE KEY-----MIIEvQIBADANBgkqhkiG9w0BAQEFAASC-----END PRIVATE KEY-----",
					last_seen_at: NOW - 3_600,
					auth_rejection: {
						code: "clock_skew",
						skew_seconds: 600,
						count: 1,
						first_at: NOW,
						last_at: NOW,
					},
				}),
				connection: {
					step: "reaching_device",
					code: "relay_unreachable",
					url: "wss://relay.example.com/x?token=eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.sig",
				},
				identity: {
					check: "mismatch",
					pinnedAt: NOW,
					fingerprint: "3F2A 91C0 77AB",
				},
			}),
		).diagnostics();
		for (const id of ["D1", "D4", "D7", "D9", "D10"])
			expect(report).toContain(`${id} `);
		expect(report).toContain("identity_mismatch");
		expect(report).toContain("fingerprint=3F2A 91C0 77AB");
		expect(report).toContain("wss://relay.example.com");
		for (const pattern of secretPatterns) expect(report).not.toMatch(pattern);
	});

	test("redactSecrets strips PEM blocks, JWTs, bearer tokens and long keys", () => {
		expect(
			redactSecrets(
				"a -----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE----- b",
			),
		).toBe("a [redacted] b");
		expect(redactSecrets("jwt eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.sig")).toBe(
			"jwt [redacted]",
		);
		expect(redactSecrets("Authorization: Bearer abc.def")).toBe(
			"Authorization: [redacted]",
		);
		expect(redactSecrets(`key ${"A".repeat(44)}`)).toBe("key [redacted]");
		expect(redactSecrets("edge-berlin-01")).toBe("edge-berlin-01");
	});
});
