import { describe, expect, test } from "bun:test";
import { presence, relationshipOf } from "./presence";
import type { DeviceRow } from "./types";

const NOW = 1_790_000_000;

const row = (patch: Partial<DeviceRow> = {}): DeviceRow => ({
	device_id: "dev-1",
	owner_id: "me",
	name: "edge-berlin-01",
	status: "active",
	registered_at: NOW - 86_400,
	last_seen_at: NOW,
	auth_epoch: 1,
	identity: {} as DeviceRow["identity"],
	...patch,
});

describe("presence", () => {
	test.each([
		[0, "online"],
		[120, "online"],
		[121, "late"],
		[600, "late"],
		[601, "offline"],
		[86_400, "offline"],
	] as const)("age %i s is %s", (age, kind) => {
		const result = presence(row({ last_seen_at: NOW - age }), NOW);
		expect(result).toEqual({ kind, since: NOW - age });
	});

	test("never checked in", () => {
		expect(presence(row({ last_seen_at: null }), NOW)).toEqual({
			kind: "never",
		});
	});

	test("revoked wins over a recent heartbeat and carries revoked_at when known", () => {
		expect(presence(row({ status: "revoked" }), NOW)).toEqual({
			kind: "revoked",
		});
		expect(
			presence(row({ status: "revoked", revoked_at: NOW - 10 }), NOW),
		).toEqual({ kind: "revoked", since: NOW - 10 });
	});

	test("a heartbeat from the future (clock skew) still counts as online", () => {
		expect(presence(row({ last_seen_at: NOW + 30 }), NOW).kind).toBe("online");
	});
});

describe("relationshipOf", () => {
	test("BG1 relationship wins when present", () => {
		expect(relationshipOf(row({ relationship: "shared" }), "me")).toBe(
			"shared",
		);
		expect(
			relationshipOf(
				row({ owner_id: "other", relationship: "cloud_approval" }),
				"me",
			),
		).toBe("cloud_approval");
	});

	test("older hub: owner by owner_id, otherwise unknown", () => {
		expect(relationshipOf(row(), "me")).toBe("owner");
		expect(relationshipOf(row({ owner_id: "other" }), "me")).toBe("unknown");
	});
});
