import { afterEach, beforeEach, expect, test } from "bun:test";
import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../../types";
import {
	FLEET_READER_RENEW_S,
	type ReadFleetOptions,
	checkFleetAnchor,
	readFleet,
	registerFleetReader,
	removeFleetReader,
} from "./fleet";
import { SnapshotIntegrityError } from "./inventory";
import { type LocalDeviceVault, updateFleetState } from "./storage";
import { digestText } from "./telemetry";
import type {
	BrowserController,
	DeviceCrypto,
	DeviceReceipt,
	FleetAudience,
	FleetLocalState,
	FleetReaderState,
	FleetView,
	ManagementGrant,
} from "./types";

const account = {
	issuer: "issuer",
	account: "reader",
	apiOrigin: "https://hub.test",
	profileId: "profile",
};
const profile = {} as IProfile;
const identity = (management = 1) => ({
	auth_key: { kty: "OKP" as const, crv: "Ed25519" as const, x: "auth" },
	telemetry_key: { kty: "OKP" as const, crv: "Ed25519" as const, x: "tel" },
	management_key: Array(32).fill(management),
});
const receipt = {
	device_id: "device",
	enrollment_id: "enrollment",
	identity: identity(),
} as DeviceReceipt;
const vault = {
	deviceId: "device",
	grantId: "owner",
	manifestJws: "onboarding",
	controllerPublic: {
		controller_key: { kty: "OKP", crv: "Ed25519", x: "key" },
	},
} as LocalDeviceVault;
const original = Object.getOwnPropertyDescriptor(globalThis, "indexedDB");
let rows: Map<string, unknown>;
let epoch = 0;
let failCommit = false;
let durability: string | undefined;

beforeEach(() => {
	rows = new Map();
	failCommit = false;
	durability = undefined;
	epoch = Math.floor(Date.now() / 1000);
	// Structured clone and commit are separate, so aborts cannot modify persisted floors.
	Object.defineProperty(globalThis, "indexedDB", {
		configurable: true,
		value: {
			open() {
				const request = {
					onsuccess: undefined as (() => void) | undefined,
					result: {
						close() {},
						transaction(
							_name: string,
							_mode: string,
							options?: IDBTransactionOptions,
						) {
							durability = options?.durability;
							const staged = new Map(rows);
							let aborted = false;
							const tx = {
								oncomplete: undefined as (() => void) | undefined,
								onabort: undefined as (() => void) | undefined,
								abort() {
									aborted = true;
									queueMicrotask(() => tx.onabort?.());
								},
								objectStore() {
									return {
										get(key: string) {
											const read = {
												result: structuredClone(staged.get(key)),
												onsuccess: undefined as (() => void) | undefined,
											};
											queueMicrotask(() => {
												read.onsuccess?.();
												queueMicrotask(() => {
													if (aborted) return;
													if (failCommit) {
														failCommit = false;
														tx.abort();
														return;
													}
													rows = staged;
													tx.oncomplete?.();
												});
											});
											return read;
										},
										put(value: unknown, key: string) {
											staged.set(key, structuredClone(value));
										},
									};
								},
							};
							return tx;
						},
					},
				};
				queueMicrotask(() => request.onsuccess?.());
				return request;
			},
		},
	});
});
afterEach(() => {
	if (original) Object.defineProperty(globalThis, "indexedDB", original);
	else Reflect.deleteProperty(globalThis, "indexedDB");
});
const state = () =>
	updateFleetState(account, "device", "key", (current) => current);
const set = (next: FleetLocalState) =>
	updateFleetState(account, "device", "key", () => next);
const signed = (value: unknown) =>
	`header.${btoa(JSON.stringify(value)).replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "")}.signature`;
const declared = (revision: number, suffix = "") =>
	signed({ revision, issued_at: 1, expires_at: epoch + 365 * 86400, suffix });
const payload = (value: string) =>
	JSON.parse(
		atob(value.split(".")[1].replaceAll("-", "+").replaceAll("_", "/")),
	);
function fixture() {
	let remote: FleetReaderState | undefined;
	let creations = 0;
	const uploads: string[] = [];
	const controller = {
		publicBundle: () => vault.controllerPublic,
		createFleetReader: (_api: string, _user: string, revision: bigint) => {
			creations++;
			return declared(Number(revision));
		},
		verifyFleetReader: (_trusted: unknown, _receipt: unknown, value: string) =>
			payload(value),
	} as unknown as BrowserController;
	const api = {
		get: async () => {
			if (!remote) throw { status: 404, code: "NOT_FOUND" };
			return remote;
		},
		put: async (
			_profile: IProfile,
			_path: string,
			body: { reader_jws: string },
		) => {
			uploads.push(body.reader_jws);
			remote = {
				reader_jws: body.reader_jws,
				revision: payload(body.reader_jws).revision,
				deleted: false,
			};
			return remote;
		},
	};
	return {
		api,
		controller,
		uploads,
		creations: () => creations,
		remote: (value: FleetReaderState) => {
			remote = value;
		},
		register: () =>
			registerFleetReader(
				api as unknown as IApiState,
				profile,
				account,
				controller,
				vault,
				receipt,
			),
	};
}

test("fleet trust floors are private to account, hub, device and controller, with durable commits", async () => {
	await set({ revision: 7, anchors: {} });
	expect(durability).toBe("strict");
	for (const [scope, device, key] of [
		[{ ...account, account: "other" }, "device", "key"],
		[{ ...account, apiOrigin: "https://other.test" }, "device", "key"],
		[account, "other-device", "key"],
		[account, "device", "other-key"],
	] as const)
		expect(
			(await updateFleetState(scope, device, key, (current) => current))
				.revision,
		).toBe(0);
	failCommit = true;
	await expect(set({ revision: 8, anchors: {} })).rejects.toThrow("committed");
	expect((await state()).revision).toBe(7);
});

test("lost fleet registration response retries the exact persisted declaration", async () => {
	const f = fixture();
	const put = f.api.put;
	f.api.put = async (...args) => {
		await put(...args);
		throw new Error("lost response");
	};
	await expect(f.register()).rejects.toThrow("lost response");
	const pending = (await state()).pending;
	if (!pending) throw new Error("Expected a pending reader registration");
	expect(pending.revision).toBe(1);
	f.api.put = put;
	await f.register();
	expect(f.creations()).toBe(1);
	expect(f.uploads).toEqual([pending.reader_jws]);
	expect((await state()).pending).toBeUndefined();
	expect((await state()).readerJws).toBe(pending.reader_jws);
});

test("an unavailable registration API never creates a competing declaration", async () => {
	const f = fixture();
	f.api.get = async () => {
		throw new Error("offline");
	};
	await expect(f.register()).rejects.toThrow("offline");
	expect(f.creations()).toBe(0);
	expect(f.uploads).toEqual([]);
});

test("registration cannot lower a trust floor advanced while its GET was pending", async () => {
	const f = fixture();
	const older = declared(1);
	const newer = declared(2);
	await set({ revision: 1, readerJws: older, anchors: {} });
	f.api.get = async () => {
		await set({ revision: 2, readerJws: newer, anchors: {} });
		return { revision: 1, reader_jws: older, deleted: false };
	};
	await expect(f.register()).rejects.toThrow();
	expect((await state()).revision).toBe(2);
	expect((await state()).readerJws).toBe(newer);
});

test("a pending registration superseded by a verified copy of this controller is adopted", async () => {
	const f = fixture();
	const put = f.api.put;
	f.api.put = async () => {
		throw new Error("offline");
	};
	await expect(f.register()).rejects.toThrow("offline");
	expect((await state()).pending?.revision).toBe(1);
	const restored = declared(1, "restored-browser");
	f.remote({ revision: 1, reader_jws: restored, deleted: false });
	f.api.put = put;
	await f.register();
	expect(await state()).toMatchObject({ revision: 1, readerJws: restored });
	expect((await state()).pending).toBeUndefined();
	expect(f.uploads).toEqual([]);
});

test("a superseding registration that fails verification keeps the pending declaration", async () => {
	const f = fixture();
	f.api.put = async () => {
		throw new Error("offline");
	};
	await expect(f.register()).rejects.toThrow("offline");
	const pending = (await state()).pending;
	f.controller.verifyFleetReader = () => {
		throw new Error("Fleet reader identity changed");
	};
	f.remote({ revision: 2, reader_jws: declared(2, "forged"), deleted: false });
	await expect(f.register()).rejects.toThrow("identity changed");
	expect((await state()).pending).toEqual(pending);
});

test("same revision reader substitution is rejected even when both signatures are valid", async () => {
	const f = fixture();
	await set({ revision: 3, readerJws: declared(3, "first"), anchors: {} });
	f.remote({ revision: 3, reader_jws: declared(3, "second"), deleted: false });
	await expect(f.register()).rejects.toThrow();
	expect((await state()).readerJws).toBe(declared(3, "first"));
});

test("snapshot floors reject rollback, equivocation and invalid sequence numbers", () => {
	const previous = {
		sequence: 4,
		digest: "original",
		grant_id: "owner",
		reader_digest: "reader",
		policy_digest: null,
		scope: { kind: "device" as const },
		kind: "status" as const,
	};
	for (const [sequence, digest] of [
		[3, "original"],
		[4, "changed"],
		[0, "original"],
		[Number.MAX_SAFE_INTEGER + 1, "original"],
	] as const)
		expect(() => checkFleetAnchor(previous, sequence, digest)).toThrow(
			"backwards",
		);
	expect(() => checkFleetAnchor(previous, 4, "original")).not.toThrow();
});

function readerFixture(grant = "owner") {
	const audience: FleetAudience = {
		grant_id: grant,
		scope: { kind: "project", project_id: "project" },
		kind: "status",
		reader_digest: "reader",
		policy_digest: null,
	};
	const now = epoch;
	const bundle = {
		manifest_jws: signed({
			audience,
			sequence: 1,
			observed_at: now,
			boot_id: "boot",
		}),
		ciphertext: "encrypted",
	};
	const view: FleetView = {
		reader_jws: declared(1),
		policy_jws: null,
		snapshots: [bundle],
	};
	const inspection: Record<string, unknown> = {
		device_id: "device",
		boot_id: "boot",
		observed_at: now * 1000,
		placements: [],
	};
	let opened: Uint8Array | undefined;
	const controller = {
		publicBundle: () => vault.controllerPublic,
		verifyFleetView: () => ({
			reader_revision: payload(view.reader_jws).revision,
			reader_expires_at: now + 3600,
			audiences: [audience],
		}),
		openFleet: () => {
			opened = new TextEncoder().encode(JSON.stringify({ inspection }));
			return opened;
		},
	} as unknown as BrowserController;
	const gets: string[] = [];
	const api = {
		get: async (_profile: unknown, url: string) => {
			gets.push(url);
			return url.endsWith("/identity") ? receipt : view;
		},
	} as unknown as IApiState;
	const crypto = {
		verifyManagementPolicy: (value: string) => payload(value),
		verifyDeviceReceipt: () => ({
			owner_invitation_key: vault.controllerPublic.controller_key,
		}),
	} as unknown as DeviceCrypto;
	return {
		view,
		audience,
		inspection,
		controller,
		gets,
		opened: () => opened,
		publish: (sequence: number) => {
			view.snapshots = [
				{
					manifest_jws: signed({
						audience,
						sequence,
						observed_at: now,
						boot_id: "boot",
					}),
					ciphertext: "encrypted",
				},
			];
		},
		read: (options?: ReadFleetOptions, reader: LocalDeviceVault = vault) =>
			readFleet(
				api,
				profile,
				account,
				controller,
				reader,
				crypto,
				undefined,
				options,
			),
	};
}

test("fleet reading zeroes opened plaintext and pins each stream before returning", async () => {
	const f = readerFixture();
	const result = await f.read();
	expect(result.observations).toHaveLength(1);
	expect(f.opened()?.every((byte) => byte === 0)).toBe(true);
	expect(Object.keys((await state()).anchors)).toHaveLength(1);
	f.view.snapshots = [];
	await expect(f.read()).rejects.toThrow("missing");
});

test("owner audiences without a policy digest still detect an omitted snapshot", async () => {
	const f = readerFixture();
	await f.read();
	const controller = f.controller as unknown as {
		verifyFleetView: () => { audiences: Record<string, unknown>[] };
	};
	const verify = controller.verifyFleetView;
	controller.verifyFleetView = () => {
		const verified = verify();
		return {
			...verified,
			audiences: verified.audiences.map(
				({ policy_digest: _omitted, ...audience }) => audience,
			),
		};
	};
	f.view.snapshots = [];
	await expect(f.read()).rejects.toThrow("missing");
});

test("fleet reading stops when the hub presents different device keys", async () => {
	const f = readerFixture();
	await f.read();
	receipt.identity = identity(2);
	try {
		await expect(f.read()).rejects.toThrow("first verified");
	} finally {
		receipt.identity = identity();
	}
});

test("a replacement grant may start its own stream while old grant rollback floors remain", async () => {
	await readerFixture("old-grant").read();
	const replacement = readerFixture("new-grant");
	await expect(replacement.read()).resolves.toMatchObject({
		observations: expect.any(Array),
	});
	expect(Object.keys((await state()).anchors)).toHaveLength(2);
});

test("a view cannot substitute a reader declaration at the locally pinned revision", async () => {
	const f = readerFixture();
	await set({ revision: 1, readerJws: declared(1, "original"), anchors: {} });
	await expect(f.read()).rejects.toThrow();
	expect(f.opened()?.every((byte) => byte === 0)).toBe(true);
});

test("reader renewal allows repeated pending views after registration without resetting stream floors", async () => {
	const f = readerFixture();
	f.view.reader_jws = signed({
		revision: 1,
		issued_at: 1,
		expires_at: epoch + 60,
	});
	f.audience.reader_digest = await digestText(f.view.reader_jws);
	f.publish(4);
	await f.read();
	const previous = structuredClone((await state()).anchors);
	const registration = fixture();
	registration.remote({
		reader_jws: f.view.reader_jws,
		revision: 1,
		deleted: false,
	});
	await registration.register();
	expect((await state()).revision).toBe(2);
	f.view.reader_jws = (await state()).readerJws ?? "";
	f.audience.reader_digest = await digestText(f.view.reader_jws);
	f.view.snapshots = [];
	for (let poll = 0; poll < 2; poll++) {
		await expect(f.read()).resolves.toMatchObject({
			observations: [],
			metrics: [],
			streams: [],
			readerRevision: 2,
		});
		expect((await state()).anchors).toEqual(previous);
	}
	for (const sequence of [3, 4]) {
		f.publish(sequence);
		await expect(f.read()).rejects.toThrow("backwards");
		expect((await state()).anchors).toEqual(previous);
	}
	f.publish(5);
	await f.read();
	expect(Object.values((await state()).anchors)[0]).toMatchObject({
		sequence: 5,
		reader_digest: f.audience.reader_digest,
		policy_digest: null,
	});
	f.view.snapshots = [];
	await expect(f.read()).rejects.toThrow("missing");
});

test("new policy prunes removed grants atomically while retaining its rollback floor", async () => {
	const f = readerFixture("old-grant");
	f.view.policy_jws = signed({ policy_version: 3 });
	f.audience.policy_digest = await digestText(f.view.policy_jws);
	f.publish(4);
	await f.read();
	const prior = await state();
	f.view.policy_jws = signed({ policy_version: 4 });
	const currentPolicy = f.view.policy_jws;
	f.audience.policy_digest = await digestText(currentPolicy);
	f.audience.grant_id = "new-grant";
	f.view.snapshots = [];
	failCommit = true;
	await expect(f.read()).rejects.toThrow("committed");
	expect(await state()).toEqual(prior);
	await f.read();
	expect(await state()).toMatchObject({
		anchors: {},
		policyVersion: 4,
		policyDigest: await digestText(currentPolicy),
	});
	for (const policy of [
		signed({ policy_version: 3 }),
		signed({ policy_version: 4, different: true }),
	]) {
		f.view.policy_jws = policy;
		await expect(f.read()).rejects.toThrow("policy moved backwards");
		expect((await state()).policyDigest).toBe(await digestText(currentPolicy));
	}
	f.view.policy_jws = null;
	await f.read();
	expect((await state()).policyVersion).toBe(4);
	expect((await state()).policyDigest).toBe(await digestText(currentPolicy));
});

test("policy renewal leaves still-authorized streams pending until their next signed snapshot", async () => {
	const f = readerFixture("project-grant");
	f.view.policy_jws = signed({ policy_version: 1 });
	f.audience.policy_digest = await digestText(f.view.policy_jws);
	f.publish(8);
	await f.read();
	const prior = structuredClone((await state()).anchors);
	f.view.policy_jws = signed({ policy_version: 2 });
	f.audience.policy_digest = await digestText(f.view.policy_jws);
	f.view.snapshots = [];
	for (let poll = 0; poll < 2; poll++) {
		await expect(f.read()).resolves.toMatchObject({
			observations: [],
			metrics: [],
			streams: [],
			policyVersion: 2,
		});
		expect((await state()).anchors).toEqual(prior);
		expect((await state()).policyVersion).toBe(2);
	}
	f.publish(7);
	await expect(f.read()).rejects.toThrow("backwards");
	f.publish(9);
	await f.read();
	expect(Object.values((await state()).anchors)[0]).toMatchObject({
		sequence: 9,
		policy_digest: f.audience.policy_digest,
	});
	f.view.snapshots = [];
	await expect(f.read()).rejects.toThrow("missing");
});

test("a cached receipt skips the identity read and the second pin: one GET per poll", async () => {
	const f = readerFixture();
	await f.read();
	expect(f.gets).toEqual([
		"devices/device/identity",
		"devices/device/fleet/snapshots/key",
	]);
	f.gets.length = 0;
	f.publish(2);
	const cached = { ...receipt, identity: identity(2) };
	await expect(f.read({ receipt: cached })).resolves.toMatchObject({
		streams: [{ sequence: 2 }],
	});
	expect(f.gets).toEqual(["devices/device/fleet/snapshots/key"]);
});

test("a read reports the reader expiry, each stream's sequence and boot, and the authorized streams", async () => {
	const f = readerFixture();
	f.publish(3);
	const id = JSON.stringify(["project/project", "status", "owner"]);
	const result = await f.read();
	expect(result).toMatchObject({
		readerRevision: 1,
		readerExpiresAt: epoch + 3600,
		streams: [
			{
				id,
				kind: "status",
				scope: { kind: "project", project_id: "project" },
				grantId: "owner",
				sequence: 3,
				bootId: "boot",
				observedAt: epoch,
			},
		],
		authorized: [id],
	});
	expect(result.policyVersion).toBeUndefined();
	expect(result.myGrant).toBeUndefined();
});

/** The agent's snapshot row (`Rows::placement`, plan §3.4.2) at its smallest detail level. */
const snapshotRow = {
	id: "placement",
	project_id: "project",
	deployment_id: "deployment",
	revision: "r1",
	desired_state: "running",
	observed_state: "running",
	config_revision: 2,
	intent_revision: 3,
	applied_revision: 2,
	desired_replicas: 1,
	running_replicas: 1,
	ready_replicas: 1,
	max_replicas: 2,
	replicas: [{ slot: 0, observed_state: "running", applied_revision: 2 }],
	has_error: true,
	source: "online",
	events_truncated: true,
};

test("agent status snapshots are accepted at every detail level and never keep error text or process ids", async () => {
	const f = readerFixture();
	f.audience.scope = { kind: "device" };
	Object.assign(f.inspection, {
		agent: { version: "0.1.0", release_version: null, release_sequence: null },
		host: { booted_at: null, agent_started_at: 100 },
		tasks: [
			{
				name: "fleet_publisher",
				state: "failing",
				since: 100,
				category: "hub_unreachable",
			},
		],
	});
	const full = {
		...snapshotRow,
		process_id: 4211,
		last_error: "failed under <state>",
		online_metadata_sha256: null,
		events: [
			{ event_id: "event", event_version: [1, 2, 0], board_version: [3, 0, 1] },
		],
		events_truncated: false,
		replicas: [
			{
				...snapshotRow.replicas[0],
				has_error: true,
				process_id: 4212,
				restarts: {
					failures: 3,
					max_restarts: 5,
					crash_looping: true,
					last_started_at: 100,
				},
			},
		],
	};
	for (const [index, row] of [full, snapshotRow].entries()) {
		f.inspection.placements = [row];
		f.publish(index + 1);
		const { observations } = await f.read();
		expect(observations).toMatchObject([
			{
				scope: { kind: "device" },
				placements: [
					{
						id: "placement",
						ready_replicas: 1,
						replicas: [
							{ slot: 0, observed_state: "running", applied_revision: 2 },
						],
					},
				],
			},
		]);
		expect(JSON.stringify(observations)).not.toMatch(/last_error|process_id/u);
	}
});

test("a status snapshot keeps the agent's validated facts and nothing else", async () => {
	const f = readerFixture();
	f.audience.scope = { kind: "device" };
	const task = {
		name: "fleet_publisher",
		state: "failing",
		since: 100,
		category: "hub_unreachable",
	};
	const facts = {
		agent: { version: "0.1.0", release_version: "1.4.0", release_sequence: 44 },
		host: { booted_at: 90, agent_started_at: 100 },
		tasks: [task],
		host_operation: null,
	};
	Object.assign(f.inspection, facts, { hostname: "edge-berlin-01" });
	const event = {
		event_id: "event",
		event_version: [1, 2, 0],
		board_version: [3, 0, 1],
	};
	const restarts = {
		failures: 3,
		max_restarts: 5,
		crash_looping: true,
		last_started_at: 100,
	};
	f.inspection.placements = [
		{
			...snapshotRow,
			online_metadata_sha256: "a".repeat(64),
			events: [event],
			events_truncated: false,
			variables: { "api-key": "value" },
			replicas: [
				{ ...snapshotRow.replicas[0], has_error: true, restarts, note: "x" },
			],
		},
	];
	const [full] = (await f.read()).observations;
	expect(full).toMatchObject({
		agent: facts.agent,
		host: facts.host,
		tasks: [task],
		hostOperation: null,
	});
	expect(full.placements[0]).toMatchObject({
		has_error: true,
		source: "online",
		events: [event],
		events_truncated: false,
		online_metadata_sha256: "a".repeat(64),
		replicas: [
			{
				slot: 0,
				has_error: true,
				restarts: { ...restarts, retry_in_seconds: null },
			},
		],
	});
	expect(JSON.stringify(full)).not.toMatch(/hostname|variables|note/u);

	f.inspection.placements = [{ ...snapshotRow, source: "elsewhere" }];
	Object.assign(f.inspection, { agent: "0.1.0", tasks: [{ name: "bad" }] });
	f.publish(2);
	const [minimal] = (await f.read()).observations;
	expect(minimal.placements[0]).toMatchObject({
		has_error: true,
		events_truncated: true,
	});
	expect(minimal.placements[0]).not.toHaveProperty("source");
	expect(minimal.placements[0]).not.toHaveProperty("events");
	expect(minimal).not.toHaveProperty("agent");
	expect(minimal).not.toHaveProperty("tasks");
	expect(minimal.host).toEqual(facts.host);

	const old = readerFixture();
	old.inspection.placements = [
		{
			...snapshotRow,
			has_error: undefined,
			source: undefined,
			events_truncated: undefined,
		},
	];
	const [legacy] = (await old.read()).observations;
	expect(Object.keys(legacy).sort()).toEqual([
		"boot_id",
		"device_id",
		"observed_at",
		"placements",
		"scope",
	]);
	expect(Object.keys(legacy.placements[0])).not.toContain("has_error");
	expect(Object.keys(legacy.placements[0].replicas?.[0] ?? {}).sort()).toEqual([
		"applied_revision",
		"observed_state",
		"slot",
	]);
});

test("a shared reader gets its own grant from the verified policy", async () => {
	const f = readerFixture("grant");
	const grant: ManagementGrant = {
		grant_id: "grant",
		user_id: "reader",
		controller_key: vault.controllerPublic.controller_key,
		scope: { kind: "project", project_id: "project" },
		capabilities: ["status"],
		expires_at: epoch + 86400,
		group_id: null,
		group_version: null,
	};
	f.view.policy_jws = signed({
		policy_version: 5,
		grants: [{ ...grant, grant_id: "other" }, grant],
	});
	f.audience.policy_digest = await digestText(f.view.policy_jws);
	f.publish(1);
	const result = await f.read(undefined, { ...vault, grantId: "grant" });
	expect(result.policyVersion).toBe(5);
	expect(result.myGrant).toEqual(grant);
});

test("verification failures are integrity errors; hub and storage failures are not", async () => {
	const f = readerFixture();
	await f.read();
	f.view.snapshots = [];
	await expect(f.read()).rejects.toBeInstanceOf(SnapshotIntegrityError);
	const controller = f.controller as unknown as { verifyFleetView: () => void };
	controller.verifyFleetView = () => {
		throw new Error("Fleet view signature is invalid");
	};
	await expect(f.read()).rejects.toBeInstanceOf(SnapshotIntegrityError);
	const offline = {
		get: async () => {
			throw new Error("Failed to fetch");
		},
	} as unknown as IApiState;
	const network = await readFleet(
		offline,
		profile,
		account,
		f.controller,
		vault,
		{} as DeviceCrypto,
	).catch((error: unknown) => error);
	expect(network).toBeInstanceOf(Error);
	expect(network).not.toBeInstanceOf(SnapshotIntegrityError);
	Reflect.deleteProperty(globalThis, "indexedDB");
	const storage = await readerFixture()
		.read()
		.catch((error: unknown) => error);
	expect((storage as Error).message).toContain("unavailable");
	expect(storage).not.toBeInstanceOf(SnapshotIntegrityError);
});

test("registration renews a reader with 30 days or less left, or when asked to", async () => {
	const expiring = (seconds: number) =>
		signed({ revision: 1, issued_at: 1, expires_at: epoch + seconds });
	const f = fixture();
	f.remote({
		revision: 1,
		reader_jws: expiring(FLEET_READER_RENEW_S + 3600),
		deleted: false,
	});
	await f.register();
	expect(f.creations()).toBe(0);
	await registerFleetReader(
		f.api as unknown as IApiState,
		profile,
		account,
		f.controller,
		vault,
		receipt,
		undefined,
		{ renew: true },
	);
	expect(f.creations()).toBe(1);
	expect((await state()).revision).toBe(2);
	rows = new Map();
	const g = fixture();
	g.remote({
		revision: 1,
		reader_jws: expiring(FLEET_READER_RENEW_S - 60),
		deleted: false,
	});
	await g.register();
	expect(g.creations()).toBe(1);
	expect(payload(g.uploads[0] ?? "").expires_at).toBeGreaterThan(
		epoch + 364 * 86400,
	);
});

test("stop receiving deletes the reader at its next revision once", async () => {
	const deletes: unknown[] = [];
	let reader: FleetReaderState | undefined = {
		revision: 3,
		reader_jws: "reader",
		deleted: false,
	};
	const api = {
		get: async () => {
			if (!reader) throw { status: 404, code: "NOT_FOUND" };
			return reader;
		},
		del: async (_profile: IProfile, url: string, body: unknown) => {
			deletes.push([url, body]);
			return reader;
		},
	} as unknown as IApiState;
	await removeFleetReader(api, profile, "device", "key");
	expect(deletes).toEqual([
		["devices/device/fleet/readers/key", { revision: 4 }],
	]);
	reader = { revision: 4, reader_jws: "reader", deleted: true };
	await removeFleetReader(api, profile, "device", "key");
	reader = undefined;
	await removeFleetReader(api, profile, "device", "key");
	expect(deletes).toHaveLength(1);
});
