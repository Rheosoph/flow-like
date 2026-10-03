import { describe, expect, test } from "bun:test";
import {
	ACCESS_FILE_MAX_BYTES,
	ACCESS_RULES_LIFETIME_S,
	type AccessChange,
	AccessRulesError,
	type AccessRulesErrorCode,
	type AccessStorage,
	accessHeadline,
	accessRequestFileName,
	accessRequestFileText,
	accessRulesOf,
	capabilityDiff,
	changeEntries,
	connectionFileText,
	createAccessLocalStore,
	grantChangeKind,
	grantExpiry,
	grantRows,
	isolationModeOf,
	mergeRecipientGrants,
	needsTrustConfirmation,
	nextAccessRules,
	parseAccessRequestFile,
	parseConnectionFile,
	parseHostIsolation,
	permissionBlock,
	policyDigest,
	renewalOptions,
	requestFileDevice,
	requestKeysState,
	scopedCapabilities,
	usedSlots,
} from "./sharing";
import type {
	DeviceReceipt,
	Ed25519PublicKey,
	ManagementGrant,
	ManagementPolicy,
	PolicyView,
} from "./types";

const DAY = 86_400;
const NOW = 1_790_769_600;
const DEVICE = "5b794764-6afc-4ac9-89c2-c6d9eb91fd42";

const key = (seed: string): Ed25519PublicKey => ({
	kty: "OKP",
	crv: "Ed25519",
	x: seed.padEnd(43, "A").slice(0, 43),
});

function grant(row: Partial<ManagementGrant> = {}): ManagementGrant {
	return {
		grant_id: "existing",
		user_id: "existing-user",
		controller_key: key("existing"),
		scope: { kind: "device" },
		capabilities: ["status"],
		expires_at: NOW + DAY,
		group_id: null,
		group_version: null,
		...row,
	};
}

function policy(version: number, grants: ManagementGrant[]): ManagementPolicy {
	return {
		version: 1,
		device_id: DEVICE,
		policy_version: version,
		previous_policy_digest: null,
		grants,
		issued_at: NOW - DAY,
		expires_at: NOW + 20 * DAY,
	};
}

function view(version: number, applied = version): PolicyView {
	return {
		policy_jws: version ? `jws-v${version}` : null,
		version,
		digest: version ? `digest-v${version}` : null,
		applied_version: applied,
		applied_digest: applied ? `digest-v${applied}` : null,
	};
}

function codeOf(run: () => unknown): AccessRulesErrorCode | undefined {
	try {
		run();
	} catch (error) {
		return error instanceof AccessRulesError ? error.code : undefined;
	}
	return undefined;
}

describe("host isolation", () => {
	test("reads the device's report and refuses anything incomplete", () => {
		expect(
			parseHostIsolation({
				platform: "linux",
				sandbox_available: true,
				require_isolation: true,
			}),
		).toEqual({
			platform: "linux",
			sandboxAvailable: true,
			requireIsolation: true,
		});
		expect(parseHostIsolation({ platform: "linux" })).toBeUndefined();
		expect(parseHostIsolation(null)).toBeUndefined();
		expect(parseHostIsolation("linux")).toBeUndefined();
		expect(
			parseHostIsolation({
				platform: "x".repeat(200),
				sandbox_available: false,
				require_isolation: false,
			})?.platform,
		).toHaveLength(64);
	});

	test("maps the report to the sandbox mode; no report is unknown", () => {
		expect(isolationModeOf(undefined)).toBeNull();
		expect(
			isolationModeOf({
				platform: "linux",
				sandboxAvailable: true,
				requireIsolation: true,
			}),
		).toBe("required");
		expect(
			isolationModeOf({
				platform: "linux",
				sandboxAvailable: true,
				requireIsolation: false,
			}),
		).toBe("optional");
		expect(
			isolationModeOf({
				platform: "macos",
				sandboxAvailable: false,
				requireIsolation: false,
			}),
		).toBe("none");
	});
});

describe("recipient grants", () => {
	test("re-approving an access request replaces its grant, and a foreign grant id is refused", () => {
		const previous = { grants: [grant()] };
		const template = {
			scope: { kind: "device" as const },
			capabilities: ["status" as const, "deploy" as const],
			expires_at: 99,
			group_id: null,
			group_version: null,
		};
		const renewed = mergeRecipientGrants(
			previous.grants,
			[
				{
					user_id: "existing-user",
					controller_key: key("existing"),
					grant_id: "existing",
				},
			],
			template,
		);
		expect(renewed).toHaveLength(1);
		expect(renewed[0]).toMatchObject({
			grant_id: "existing",
			capabilities: ["status", "deploy"],
			expires_at: 99,
		});
		expect(() =>
			mergeRecipientGrants(
				previous.grants,
				[
					{
						user_id: "intruder",
						controller_key: key("existing"),
						grant_id: "existing",
					},
				],
				template,
			),
		).toThrow("already belongs to another account");
		expect(
			codeOf(() =>
				mergeRecipientGrants(
					previous.grants,
					[
						{
							user_id: "existing-user",
							controller_key: key("other"),
							grant_id: "existing",
						},
					],
					template,
				),
			),
		).toBe("grant_conflict");
		expect(() =>
			mergeRecipientGrants(
				[],
				[
					{ user_id: "a", controller_key: key("a"), grant_id: "same" },
					{ user_id: "b", controller_key: key("b"), grant_id: "same" },
				],
				template,
			),
		).toThrow("listed more than once");
		expect(
			mergeRecipientGrants(
				previous.grants,
				[{ user_id: "new-user", controller_key: key("new") }],
				template,
				() => "generated",
			).map((row) => row.grant_id),
		).toEqual(["existing", "generated"]);
	});
});

describe("access request files", () => {
	const entry = {
		user_id: "usr_7JkD2wQe",
		controller_key: key("jonas"),
		grant_id: "4f1d0b6e-2c7a-4f3e-9a51-8e2b7c0d9a11",
	};

	test("round-trips what the request flow writes", () => {
		const text = accessRequestFileText(
			entry.user_id,
			entry.controller_key,
			entry.grant_id,
		);
		expect(parseAccessRequestFile(text, text.length)).toEqual({
			ok: true,
			recipients: [entry],
		});
		expect(accessRequestFileName(DEVICE)).toBe(`device-access-${DEVICE}.json`);
	});

	test("tolerates fields a newer app adds and a missing request id", () => {
		const text = JSON.stringify([
			{
				user_id: "a",
				controller_key: { ...key("a"), use: "sig" },
				requested_at: 5,
			},
		]);
		expect(parseAccessRequestFile(text, text.length)).toEqual({
			ok: true,
			recipients: [{ user_id: "a", controller_key: key("a") }],
		});
	});

	test("names why a file cannot be used", () => {
		const good = JSON.stringify([entry]);
		expect(parseAccessRequestFile(good, ACCESS_FILE_MAX_BYTES + 1)).toEqual({
			ok: false,
			error: "too_large",
		});
		for (const text of ["not json", "{}", "[]", '"text"'])
			expect(parseAccessRequestFile(text, text.length)).toEqual({
				ok: false,
				error: "not_request_file",
			});
		const many = JSON.stringify(Array.from({ length: 25 }, () => entry));
		expect(parseAccessRequestFile(many, many.length)).toEqual({
			ok: false,
			error: "too_many_people",
			count: 25,
		});
		for (const bad of [
			{ ...entry, user_id: "" },
			{ ...entry, user_id: "x".repeat(129) },
			{ ...entry, controller_key: { ...entry.controller_key, x: "short" } },
			{ ...entry, controller_key: { ...entry.controller_key, kty: "RSA" } },
			{ ...entry, grant_id: "has spaces" },
			null,
		]) {
			const text = JSON.stringify([entry, bad]);
			expect(parseAccessRequestFile(text, text.length)).toEqual({
				ok: false,
				error: "incomplete_entry",
			});
		}
	});

	test("finds the device in the file name by its id or its first eight characters", () => {
		const other = "54484ac9-891c-4570-a0e5-51d016e09143";
		const ids = [DEVICE, other];
		expect(requestFileDevice(`device-access-${DEVICE}.json`, ids)).toBe(DEVICE);
		expect(requestFileDevice("device-access-54484ac9.json", ids)).toBe(other);
		expect(requestFileDevice("device-access-54484AC9 (2).json", ids)).toBe(
			other,
		);
		expect(requestFileDevice("request.json", ids)).toBeUndefined();
		expect(
			requestFileDevice("device-access-00000000.json", ids),
		).toBeUndefined();
		expect(
			requestFileDevice("device-access-5b794764.json", [DEVICE, `${DEVICE}x`]),
		).toBeUndefined();
	});
});

describe("connection files", () => {
	const receipt = {
		device_id: DEVICE,
		manifest_jws: "manifest",
	} as DeviceReceipt;
	const ownerKey = key("owner");

	test("round-trips what the owner downloads", () => {
		const text = connectionFileText(receipt, ownerKey);
		expect(parseConnectionFile(text, text.length)).toEqual({
			ok: true,
			file: { version: 1, receipt, owner_controller_key: ownerKey },
		});
	});

	test("refuses oversized files and anything that is not a connection file", () => {
		const text = connectionFileText(receipt, ownerKey);
		expect(parseConnectionFile(text, ACCESS_FILE_MAX_BYTES + 1)).toEqual({
			ok: false,
			error: "too_large",
		});
		for (const bad of [
			"nope",
			"[]",
			JSON.stringify({ version: 2, receipt, owner_controller_key: ownerKey }),
			JSON.stringify({ version: 1, receipt }),
			JSON.stringify({
				version: 1,
				receipt: { device_id: DEVICE },
				owner_controller_key: ownerKey,
			}),
			JSON.stringify({
				version: 1,
				receipt,
				owner_controller_key: { ...ownerKey, x: "bad" },
			}),
		])
			expect(parseConnectionFile(bad, bad.length)).toEqual({
				ok: false,
				error: "not_connection_file",
			});
	});

	test("keys made for the same file are reusable; owner keys and keys of another owner block a new request", () => {
		const file = {
			version: 1 as const,
			receipt,
			owner_controller_key: ownerKey,
		};
		expect(requestKeysState(undefined, file)).toBe("none");
		expect(
			requestKeysState(
				{
					grantId: "lost-request-grant",
					ownerControllerKey: ownerKey,
					manifestJws: "manifest",
				},
				file,
			),
		).toBe("reusable");
		expect(
			requestKeysState({ grantId: "owner", manifestJws: "manifest" }, file),
		).toBe("conflict");
		expect(
			requestKeysState(
				{
					grantId: "g",
					ownerControllerKey: key("someone-else"),
					manifestJws: "manifest",
				},
				file,
			),
		).toBe("conflict");
		expect(
			requestKeysState(
				{
					grantId: "g",
					ownerControllerKey: ownerKey,
					manifestJws: "another-registration",
				},
				file,
			),
		).toBe("conflict");
	});
});

describe("permissions", () => {
	test("device-wide permissions need whole-device access; certificates need agent support unless already held", () => {
		expect(permissionBlock("reboot", "project", true)).toBe("device_only");
		expect(permissionBlock("update_agent", "placement", true)).toBe(
			"device_only",
		);
		expect(permissionBlock("reboot", "device", undefined)).toBeNull();
		expect(permissionBlock("manage_certificates", "device", true)).toBeNull();
		expect(permissionBlock("manage_certificates", "device", false)).toBe(
			"certificates_unsupported",
		);
		expect(permissionBlock("manage_certificates", "device", undefined)).toBe(
			"certificates_unknown",
		);
		expect(
			permissionBlock("manage_certificates", "device", false, true),
		).toBeNull();
		expect(permissionBlock("manage_certificates", "project", true, true)).toBe(
			"device_only",
		);
		expect(permissionBlock("deploy", "project", false)).toBeNull();
	});

	test("a narrower scope drops device-wide permissions and keeps the display order", () => {
		expect(
			scopedCapabilities(["reboot", "deploy", "status", "status"], "project"),
		).toEqual(["status", "deploy"]);
		expect(scopedCapabilities(["reboot", "status"], "device")).toEqual([
			"status",
			"reboot",
		]);
	});

	test("adding a code-running permission asks for trust unless the device requires a sandbox", () => {
		expect(
			needsTrustConfirmation(undefined, ["status", "deploy"], "none"),
		).toBe(true);
		expect(needsTrustConfirmation(undefined, ["start"], "optional")).toBe(true);
		expect(needsTrustConfirmation(undefined, ["restart"], null)).toBe(true);
		expect(needsTrustConfirmation(undefined, ["scale"], undefined)).toBe(true);
		expect(needsTrustConfirmation(undefined, ["deploy"], "required")).toBe(
			false,
		);
		expect(needsTrustConfirmation(undefined, ["status", "stop"], "none")).toBe(
			false,
		);
		expect(
			needsTrustConfirmation(["deploy"], ["deploy", "status"], "none"),
		).toBe(false);
		expect(
			needsTrustConfirmation(["deploy"], ["deploy", "start"], "none"),
		).toBe(true);
	});

	test("the permission diff lists additions, removals and what stays, in display order", () => {
		expect(
			capabilityDiff(["status", "logs", "stop"], ["deploy", "status", "stop"]),
		).toEqual([
			{ capability: "status", kind: "kept" },
			{ capability: "logs", kind: "removed" },
			{ capability: "deploy", kind: "added" },
			{ capability: "stop", kind: "kept" },
		]);
	});

	test("classifies a change as new, changed, renewed or unchanged", () => {
		const before = grant({ capabilities: ["status", "logs"] });
		expect(grantChangeKind(undefined, before)).toBe("new");
		expect(grantChangeKind(before, { ...before })).toBe("same");
		expect(
			grantChangeKind(before, { ...before, capabilities: ["logs", "status"] }),
		).toBe("same");
		expect(
			grantChangeKind(before, { ...before, expires_at: before.expires_at + 1 }),
		).toBe("renewed");
		expect(
			grantChangeKind(before, { ...before, capabilities: ["status"] }),
		).toBe("changed");
		expect(
			grantChangeKind(before, {
				...before,
				scope: { kind: "project", project_id: "p" },
			}),
		).toBe("changed");
	});
});

describe("durations", () => {
	test("access never outlasts the re-signed rules", () => {
		expect(grantExpiry(NOW, DAY)).toBe(NOW + DAY);
		expect(grantExpiry(NOW, 90 * DAY)).toBe(NOW + ACCESS_RULES_LIFETIME_S);
		expect(grantExpiry(NOW, -5)).toBe(NOW);
	});

	test("renewal steps stop at the rules' expiry and end with it", () => {
		const options = renewalOptions(NOW + 6 * 3600, NOW);
		expect(options.map((option) => option.id)).toEqual([
			String(DAY),
			String(3 * DAY),
			String(7 * DAY),
			String(14 * DAY),
			"rules",
		]);
		expect(options[0]?.until).toBe(NOW + 6 * 3600 + DAY);
		expect(options.at(-1)?.until).toBe(NOW + ACCESS_RULES_LIFETIME_S);
		expect(
			renewalOptions(NOW + 20 * DAY, NOW).map((option) => option.id),
		).toEqual([String(DAY), String(3 * DAY), String(7 * DAY), "rules"]);
		expect(renewalOptions(NOW - 5 * DAY, NOW)[0]?.until).toBe(NOW + DAY);
	});
});

describe("access rules", () => {
	test("no version is 'not shared', an older applied version is waiting", () => {
		expect(accessRulesOf(undefined)).toBeUndefined();
		expect(accessRulesOf(view(0))).toBeNull();
		expect(accessRulesOf(view(5), policy(5, []))).toEqual({
			saved: 5,
			applied: 5,
			waiting: false,
			expiresAt: NOW + 20 * DAY,
			issuedAt: NOW - DAY,
		});
		expect(accessRulesOf(view(2, 1), undefined, 77)).toEqual({
			saved: 2,
			applied: 1,
			waiting: true,
			expiresAt: 77,
		});
		expect(accessRulesOf(view(2, 1), undefined, null)).toEqual({
			saved: 2,
			applied: 1,
			waiting: true,
		});
	});

	test("rows: active, expired, preset and count", () => {
		const rows = grantRows(
			policy(5, [
				grant({ grant_id: "a", capabilities: ["status", "metrics", "logs"] }),
				grant({
					grant_id: "b",
					capabilities: ["status", "deploy"],
					expires_at: NOW - 1,
				}),
			]),
			view(5),
			NOW,
		);
		expect(rows.map((row) => [row.grant.grant_id, row.status])).toEqual([
			["a", "active"],
			["b", "expired"],
		]);
		expect(rows[0]).toMatchObject({
			preset: "viewer",
			count: 3,
			runsCode: false,
		});
		expect(rows[1]).toMatchObject({
			preset: "custom",
			count: 2,
			runsCode: true,
		});
		expect(usedSlots(rows)).toBe(1);
	});

	test("while the device uses an older version, rows this computer did not change stay active", () => {
		const kept = grant({ grant_id: "kept" });
		const added = grant({ grant_id: "added", user_id: "new" });
		const changed = grant({ grant_id: "changed", user_id: "c" });
		const removed = grant({ grant_id: "removed", user_id: "r" });
		const changes: AccessChange[] = [
			{
				deviceId: DEVICE,
				version: 2,
				savedAt: NOW - 60,
				entries: [
					{ grantId: "added", userId: "new", kind: "added", after: added },
					{ grantId: "changed", userId: "c", kind: "changed", after: changed },
					{ grantId: "removed", userId: "r", kind: "removed", before: removed },
				],
			},
		];
		const rows = grantRows(
			policy(2, [kept, added, changed]),
			view(2, 1),
			NOW,
			changes,
		);
		expect(
			rows.map((row) => [
				row.grant.grant_id,
				row.status,
				row.pendingVersion,
				row.isNew,
			]),
		).toEqual([
			["kept", "active", undefined, undefined],
			["added", "waiting", 2, true],
			["changed", "waiting", 2, undefined],
			["removed", "removing", 2, undefined],
		]);
		expect(usedSlots(rows)).toBe(3);
		expect(
			grantRows(policy(2, [kept, added, changed]), view(2), NOW, changes).map(
				(row) => row.status,
			),
		).toEqual(["active", "active", "active"]);
	});

	test("a version saved elsewhere leaves every row waiting, because any of them may have changed", () => {
		const rows = grantRows(
			policy(3, [grant({ grant_id: "a" }), grant({ grant_id: "b" })]),
			view(3, 1),
			NOW,
			[
				{
					deviceId: DEVICE,
					version: 3,
					savedAt: NOW,
					entries: [{ grantId: "b", userId: "existing-user", kind: "renewed" }],
				},
			],
		);
		expect(rows.map((row) => [row.status, row.pendingVersion])).toEqual([
			["waiting", 3],
			["waiting", 3],
		]);
	});
});

describe("saving a new version", () => {
	test("the first rules start at version 1 without a predecessor", () => {
		const next = nextAccessRules({
			deviceId: DEVICE,
			view: view(0),
			now: NOW,
			upserts: [grant({ expires_at: NOW + 90 * DAY })],
		});
		expect(next).toMatchObject({
			version: 1,
			device_id: DEVICE,
			policy_version: 1,
			previous_policy_digest: null,
			issued_at: NOW,
			expires_at: NOW + ACCESS_RULES_LIFETIME_S,
		});
		expect(next.grants[0]?.expires_at).toBe(NOW + ACCESS_RULES_LIFETIME_S);
	});

	test("keeps everyone else, replaces a grant by id, drops grants that ran out and chains the digest of the signed rules", () => {
		const mira = grant({ grant_id: "mira", user_id: "mira" });
		const jonas = grant({
			grant_id: "jonas",
			user_id: "jonas",
			controller_key: key("jonas"),
		});
		const gone = grant({
			grant_id: "gone",
			user_id: "gone",
			expires_at: NOW - 10,
		});
		const next = nextAccessRules({
			deviceId: DEVICE,
			view: view(5),
			policy: policy(5, [mira, jonas, gone]),
			now: NOW,
			upserts: [
				{ ...jonas, capabilities: ["deploy", "status"], expires_at: NOW + 99 },
			],
		});
		expect(next.policy_version).toBe(6);
		expect(next.previous_policy_digest).toBe(policyDigest("jws-v5"));
		expect(next.previous_policy_digest).not.toBe("digest-v5");
		expect(next.grants.map((row) => row.grant_id)).toEqual(["mira", "jonas"]);
		expect(next.grants[0]).toEqual(mira);
		expect(next.grants[1]).toMatchObject({
			capabilities: ["status", "deploy"],
			expires_at: NOW + 99,
		});
	});

	test("renewing the rules alone changes nothing but the dates", () => {
		const mira = grant({ grant_id: "mira" });
		const next = nextAccessRules({
			deviceId: DEVICE,
			view: view(5),
			policy: policy(5, [mira]),
			now: NOW,
		});
		expect(next.grants).toEqual([mira]);
		expect(next.expires_at).toBe(NOW + ACCESS_RULES_LIFETIME_S);
	});

	test("removes a grant, and refuses one that is no longer there", () => {
		const mira = grant({ grant_id: "mira" });
		const jonas = grant({ grant_id: "jonas", user_id: "jonas" });
		const draft = {
			deviceId: DEVICE,
			view: view(5),
			policy: policy(5, [mira, jonas]),
			now: NOW,
		};
		expect(
			nextAccessRules({ ...draft, removeIds: ["mira"] }).grants.map(
				(row) => row.grant_id,
			),
		).toEqual(["jonas"]);
		expect(
			codeOf(() => nextAccessRules({ ...draft, removeIds: ["nobody"] })),
		).toBe("grant_gone");
	});

	test("refuses rules it could not check, a grant id of someone else, empty permissions and more than 24 people", () => {
		const mira = grant({ grant_id: "mira", user_id: "mira" });
		const base = { deviceId: DEVICE, now: NOW };
		expect(codeOf(() => nextAccessRules({ ...base, view: view(5) }))).toBe(
			"unverified",
		);
		expect(
			codeOf(() =>
				nextAccessRules({
					...base,
					view: { ...view(0), version: 3 },
				}),
			),
		).toBe("incomplete");
		expect(
			codeOf(() =>
				nextAccessRules({ ...base, view: view(5), policy: policy(4, []) }),
			),
		).toBe("wrong_version");
		expect(
			codeOf(() =>
				nextAccessRules({
					...base,
					view: view(5),
					policy: { ...policy(5, []), device_id: "other" },
				}),
			),
		).toBe("wrong_device");
		expect(
			codeOf(() =>
				nextAccessRules({
					...base,
					view: view(5),
					policy: policy(5, [mira]),
					upserts: [{ ...mira, user_id: "intruder" }],
				}),
			),
		).toBe("grant_conflict");
		expect(
			codeOf(() =>
				nextAccessRules({
					...base,
					view: view(0),
					upserts: [grant({ grant_id: "a" }), grant({ grant_id: "a" })],
				}),
			),
		).toBe("duplicate_grant");
		expect(
			codeOf(() =>
				nextAccessRules({
					...base,
					view: view(0),
					upserts: [grant({ capabilities: [] })],
				}),
			),
		).toBe("no_permissions");
		const crowd = Array.from({ length: 24 }, (_, index) =>
			grant({ grant_id: `g${index}`, user_id: `u${index}` }),
		);
		expect(
			nextAccessRules({ ...base, view: view(5), policy: policy(5, crowd) })
				.grants,
		).toHaveLength(24);
		expect(
			codeOf(() =>
				nextAccessRules({
					...base,
					view: view(5),
					policy: policy(5, crowd),
					upserts: [grant({ grant_id: "one-more", user_id: "x" })],
				}),
			),
		).toBe("too_many");
	});

	test("the change log names what a save did per person and skips what stayed the same", () => {
		const mira = grant({ grant_id: "mira", user_id: "mira" });
		const jonas = grant({ grant_id: "jonas", user_id: "jonas" });
		const anna = grant({ grant_id: "anna", user_id: "anna" });
		const entries = changeEntries(
			[mira, jonas],
			[{ ...mira }, { ...jonas, expires_at: jonas.expires_at + DAY }, anna],
			[],
		);
		expect(entries.map((entry) => [entry.grantId, entry.kind])).toEqual([
			["jonas", "renewed"],
			["anna", "added"],
		]);
		expect(changeEntries([mira, jonas], [], ["mira"])).toEqual([
			{ grantId: "mira", userId: "mira", kind: "removed", before: mira },
		]);
		expect(
			changeEntries([mira], [{ ...mira, capabilities: ["logs"] }], [])[0],
		).toMatchObject({ kind: "changed", before: mira });
	});
});

describe("headline", () => {
	test("counts distinct people, names the access that ends first and the devices that wait", () => {
		const mira = grant({
			grant_id: "m",
			user_id: "mira",
			expires_at: NOW + 6 * 3600,
		});
		const jonas = grant({
			grant_id: "j",
			user_id: "jonas",
			expires_at: NOW + 14 * 3600,
		});
		const headline = accessHeadline(
			[
				{
					deviceId: "edge",
					name: "edge-berlin-01",
					rules: { saved: 5, applied: 5, waiting: false },
					rows: grantRows(policy(5, [mira, jonas]), view(5), NOW),
				},
				{
					deviceId: "studio",
					name: "studio-mac-mini",
					rules: { saved: 2, applied: 1, waiting: true },
					rows: grantRows(
						policy(2, [{ ...mira, grant_id: "m2", expires_at: NOW + DAY }]),
						view(2, 1),
						NOW,
					),
				},
				{ deviceId: "cold", name: "cold-storage-nas", rules: null },
				{
					deviceId: "locked",
					name: "warehouse-pi",
					rules: { saved: 1, applied: 1, waiting: false },
				},
				{ deviceId: "loading", name: "loading", rules: undefined },
			],
			NOW,
		);
		expect(headline).toEqual({
			people: 2,
			soon: {
				userId: "mira",
				deviceId: "edge",
				deviceName: "edge-berlin-01",
				expiresAt: NOW + 6 * 3600,
			},
			waiting: [{ deviceId: "studio", name: "studio-mac-mini" }],
			unread: [{ deviceId: "locked", name: "warehouse-pi" }],
		});
	});

	test("expired access is nobody, and nothing ending soon leaves the sentence out", () => {
		const headline = accessHeadline(
			[
				{
					deviceId: "edge",
					name: "edge",
					rules: { saved: 1, applied: 1, waiting: false },
					rows: grantRows(
						policy(1, [
							grant({ user_id: "old", expires_at: NOW - 5 }),
							grant({
								grant_id: "b",
								user_id: "later",
								expires_at: NOW + 3 * DAY,
							}),
						]),
						view(1),
						NOW,
					),
				},
			],
			NOW,
		);
		expect(headline).toEqual({ people: 1, waiting: [], unread: [] });
	});
});

describe("what this computer remembers", () => {
	function memory(initial: Record<string, string> = {}) {
		const items = new Map(Object.entries(initial));
		const storage: AccessStorage = {
			getItem: (name) => items.get(name) ?? null,
			setItem: (name, value) => {
				items.set(name, value);
			},
		};
		return { items, storage };
	}

	const request = (id: string, userId = "usr") => ({
		id,
		file: `device-access-${id}.json`,
		userId,
		controllerKey: key(userId),
		grantId: `grant-${id}`,
		deviceId: DEVICE,
		importedAt: NOW,
	});

	test("imported requests and saved changes survive a reload, per account scope", () => {
		const { storage } = memory();
		const store = createAccessLocalStore("scope-a", storage);
		let notified = 0;
		const stop = store.subscribe(() => {
			notified += 1;
		});
		store.addRequests([request("1", "a"), request("2", "b")]);
		store.recordChange({
			deviceId: DEVICE,
			version: 6,
			savedAt: NOW,
			entries: [{ grantId: "g", userId: "a", kind: "added", after: grant() }],
		});
		stop();
		expect(notified).toBe(2);
		const again = createAccessLocalStore("scope-a", storage);
		expect(again.get().requests.map((row) => row.id)).toEqual(["1", "2"]);
		expect(again.get().changes).toHaveLength(1);
		expect(again.get().changes[0]?.entries[0]?.after).toEqual(grant());
		expect(createAccessLocalStore("scope-b", storage).get()).toEqual({
			requests: [],
			changes: [],
		});
	});

	test("importing the same person's request for the same device again replaces it; removing works by id", () => {
		const { storage } = memory();
		const store = createAccessLocalStore("scope", storage);
		store.addRequests([request("1", "a")]);
		store.addRequests([request("3", "a"), request("4", "b")]);
		expect(store.get().requests.map((row) => row.id)).toEqual(["3", "4"]);
		store.removeRequests(["3", "unknown"]);
		expect(store.get().requests.map((row) => row.id)).toEqual(["4"]);
	});

	test("a version saved twice keeps the later record", () => {
		const store = createAccessLocalStore("scope", memory().storage);
		const change = (savedAt: number): AccessChange => ({
			deviceId: DEVICE,
			version: 2,
			savedAt,
			entries: [],
		});
		store.recordChange(change(1));
		store.recordChange(change(2));
		expect(store.get().changes).toEqual([change(2)]);
	});

	test("damaged or refused storage never throws: the store works in memory", () => {
		const damaged = memory({
			"flow-like/devices/access/scope": "{not json",
			"flow-like/devices/access/half": JSON.stringify({
				requests: [{ id: 1 }],
				changes: "x",
			}),
		});
		expect(createAccessLocalStore("scope", damaged.storage).get()).toEqual({
			requests: [],
			changes: [],
		});
		expect(createAccessLocalStore("half", damaged.storage).get()).toEqual({
			requests: [],
			changes: [],
		});
		const refusing: AccessStorage = {
			getItem: () => {
				throw new Error("blocked");
			},
			setItem: () => {
				throw new Error("quota");
			},
		};
		const store = createAccessLocalStore("scope", refusing);
		store.addRequests([request("1")]);
		expect(store.get().requests).toHaveLength(1);
		const none = createAccessLocalStore("scope", undefined);
		none.addRequests([request("2")]);
		expect(none.get().requests).toHaveLength(1);
	});
});
