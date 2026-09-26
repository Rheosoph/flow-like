import { afterEach, beforeEach, expect, test } from "bun:test";
import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../../types";
import { checkFleetAnchor, readFleet, registerFleetReader } from "./fleet";
import { updateFleetState, type LocalDeviceVault } from "./storage";
import { digestText } from "./telemetry";
import type {
	BrowserController,
	DeviceCrypto,
	DeviceReceipt,
	FleetAudience,
	FleetLocalState,
	FleetReaderState,
	FleetView,
} from "./types";

const account = {
	issuer: "issuer",
	account: "reader",
	apiOrigin: "https://hub.test",
	profileId: "profile",
};
const profile = {} as IProfile;
const receipt = {} as DeviceReceipt;
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
	expect(pending?.revision).toBe(1);
	f.api.put = put;
	await f.register();
	expect(f.creations()).toBe(1);
	expect(f.uploads).toEqual([pending!.reader_jws]);
	expect((await state()).pending).toBeUndefined();
	expect((await state()).readerJws).toBe(pending?.reader_jws);
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
	const older = declared(1),
		newer = declared(2);
	await set({ revision: 1, readerJws: older, anchors: {} });
	f.api.get = async () => {
		await set({ revision: 2, readerJws: newer, anchors: {} });
		return { revision: 1, reader_jws: older, deleted: false };
	};
	await expect(f.register()).rejects.toThrow();
	expect((await state()).revision).toBe(2);
	expect((await state()).readerJws).toBe(newer);
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
	let opened: Uint8Array | undefined;
	const controller = {
		publicBundle: () => vault.controllerPublic,
		verifyFleetView: () => ({
			reader_revision: payload(view.reader_jws).revision,
			reader_expires_at: now + 3600,
			audiences: [audience],
		}),
		openFleet: () => {
			opened = new TextEncoder().encode(
				JSON.stringify({
					inspection: {
						device_id: "device",
						boot_id: "boot",
						observed_at: now * 1000,
						placements: [],
					},
				}),
			);
			return opened;
		},
	} as unknown as BrowserController;
	const api = {
		get: async (_profile: unknown, url: string) =>
			url.endsWith("/identity") ? receipt : view,
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
		controller,
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
		read: () => readFleet(api, profile, account, controller, vault, crypto),
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
	f.view.reader_jws = (await state()).readerJws!;
	f.audience.reader_digest = await digestText(f.view.reader_jws);
	f.view.snapshots = [];
	for (let poll = 0; poll < 2; poll++) {
		await expect(f.read()).resolves.toEqual({ observations: [], metrics: [] });
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
		await expect(f.read()).resolves.toEqual({ observations: [], metrics: [] });
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
