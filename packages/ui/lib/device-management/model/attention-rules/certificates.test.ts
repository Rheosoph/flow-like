import { describe, expect, test } from "bun:test";
import {
	SAMPLE_IDS,
	SAMPLE_NOW,
	sampleFleet,
} from "../__fixtures__/sample-fleet";
import type { AttentionInputExt } from "../attention";
import type { AttentionKey } from "../types";
import { CERTIFICATE_RULES } from "./certificates";

const DAY = 86_400;
const INTERNAL_MQTT = "93bcc1ef-5f49-4bb3-b90c-7b052822bf02";
const EDGE_API = "24f6fe22-c6c2-4e15-9d37-7a41a379afb9";

const evaluate = (key: AttentionKey, input: AttentionInputExt) =>
	CERTIFICATE_RULES.find((rule) => rule.key === key)?.evaluate(input) ?? [];

function edgeLive(input: AttentionInputExt) {
	const live = input.live[SAMPLE_IDS.edge];
	const certificates = live.certificates?.certificates;
	const issuer = live.certificateIssuers?.[0];
	const acme = live.acme?.[0];
	if (!certificates || !issuer || !acme)
		throw new Error("fixture: edge certificates");
	return { live, certificates, issuer, acme };
}

/** Moves internal-mqtt's expiry on the device and the hub inventory together. */
function mqttExpiresIn(input: AttentionInputExt, seconds: number) {
	const { certificates } = edgeLive(input);
	const notAfter = SAMPLE_NOW + seconds;
	certificates[1].not_after = notAfter;
	const hub = input.certInventory[SAMPLE_IDS.edge]?.certificates[1];
	if (hub) hub.not_after = notAfter;
}

function healthyIssuer(input: AttentionInputExt) {
	const { issuer } = edgeLive(input);
	issuer.last_error = null;
	issuer.not_after = SAMPLE_NOW + 60 * DAY;
	issuer.next_renewal_at = SAMPLE_NOW + DAY;
}

describe("expiry", () => {
	test("golden: expiring in 5 days, renewal blocked by an expired authority, one item", () => {
		const input = sampleFleet();
		const [item] = evaluate("certificate_expiring", input);
		expect(item.severity).toBe("warning");
		expect(item.copy.params).toMatchObject({
			label: "internal-mqtt",
			days: 5,
			renewal: "authority_expired",
			renewalAt: 1_790_766_000,
		});
		expect(item.action).toMatchObject({
			code: "fix_renewal",
			target: {
				screen: "device",
				tab: "certificates",
				certificateId: INTERNAL_MQTT,
			},
		});
		expect(item.source.src).toBe("hub");
		for (const key of [
			"renewal_authority_expiring",
			"renewal_delegation_error",
			"acme_error",
		] as const)
			expect(evaluate(key, input)).toEqual([]);
	});

	test("expiring with healthy automatic renewal scheduled earlier is Info", () => {
		const input = sampleFleet();
		healthyIssuer(input);
		const [item] = evaluate("certificate_expiring", input);
		expect(item.severity).toBe("info");
		expect(item.copy.params?.renewsAt).toBe(SAMPLE_NOW + DAY);
		// A renewal that was due and didn't happen is not "will renew automatically".
		edgeLive(input).issuer.next_renewal_at = SAMPLE_NOW - 60;
		const [overdue] = evaluate("certificate_expiring", { ...input });
		expect(overdue.severity).toBe("warning");
		expect(overdue.copy.params?.renewsAt).toBeUndefined();
	});

	test("expired: Critical when a service uses it, Warning otherwise (hub-only certificates have no name)", () => {
		const input = sampleFleet();
		const [warehouse] = evaluate("certificate_expired", input);
		expect(warehouse.severity).toBe("warning");
		expect(warehouse.lastKnown).toBe(true);
		expect(warehouse.copy.params?.label).toBeUndefined();
		const { certificates } = edgeLive(input);
		certificates[0].not_after = SAMPLE_NOW - 60;
		const edge = evaluate("certificate_expired", { ...input }).find(
			(item) =>
				item.subject.kind === "certificate" &&
				item.subject.certificateId === EDGE_API,
		);
		expect(edge?.severity).toBe("critical");
		expect(edge?.copy.params?.services).toBe(1);
	});

	test("a renewal the hub already knows wins over an older live read", () => {
		const input = sampleFleet();
		healthyIssuer(input);
		const hub = input.certInventory[SAMPLE_IDS.edge]?.certificates[1];
		if (!hub) throw new Error("fixture: edge inventory");
		hub.revision = edgeLive(input).certificates[1].revision + 1;
		hub.not_after = SAMPLE_NOW + 90 * DAY;
		expect(evaluate("certificate_expiring", input)).toEqual([]);
	});

	test("not yet valid is a Notice", () => {
		const input = sampleFleet();
		edgeLive(input).certificates[0].not_before = SAMPLE_NOW + DAY;
		expect(evaluate("certificate_not_yet_valid", input)[0]?.severity).toBe(
			"notice",
		);
	});
});

describe("renewal problems on certificates that aren't expiring yet", () => {
	test("authority expiring within 30 days", () => {
		const input = sampleFleet();
		mqttExpiresIn(input, 40 * DAY);
		healthyIssuer(input);
		edgeLive(input).issuer.not_after = SAMPLE_NOW + 20 * DAY;
		const [item] = evaluate("renewal_authority_expiring", input);
		expect(item.copy.params?.authorityExpiresAt).toBe(SAMPLE_NOW + 20 * DAY);
		expect(evaluate("certificate_expiring", input)).toEqual([]);
	});

	test("delegation error", () => {
		const input = sampleFleet();
		mqttExpiresIn(input, 40 * DAY);
		healthyIssuer(input);
		edgeLive(input).issuer.last_error = "CA unreachable";
		expect(
			evaluate("renewal_delegation_error", input)[0]?.copy.params?.message,
		).toBe("CA unreachable");
		expect(evaluate("renewal_authority_expiring", input)).toEqual([]);
	});

	test("ACME error and a staging certificate in use", () => {
		const input = sampleFleet();
		const { acme } = edgeLive(input);
		acme.last_error = "rate limited";
		acme.environment = "lets_encrypt_staging";
		expect(evaluate("acme_error", input)[0]?.copy.params?.nextAttemptAt).toBe(
			acme.next_attempt_at,
		);
		expect(evaluate("acme_staging_in_use", input)[0]?.severity).toBe("notice");
	});
});

describe("requests, inventory and slots", () => {
	test("a signing request expiring within 7 days, or out of date", () => {
		const input = sampleFleet();
		expect(evaluate("signing_request_attention", input)).toEqual([]);
		const request = input.live[SAMPLE_IDS.edge].certificateRequests?.[0];
		if (!request) throw new Error("fixture");
		request.expires_at = SAMPLE_NOW + 3 * DAY;
		expect(
			evaluate("signing_request_attention", { ...input })[0]?.copy.params
				?.reason,
		).toBe("expiring");
		request.expires_at = SAMPLE_NOW + 30 * DAY;
		request.certificate_id = INTERNAL_MQTT;
		request.expected_revision = 1;
		expect(
			evaluate("signing_request_attention", { ...input })[0]?.copy.params
				?.reason,
		).toBe("stale");
	});

	test("inventory stale: online and older than 2 h; never reported only when certificates are known", () => {
		const input = sampleFleet();
		expect(evaluate("certificate_inventory_stale", input)).toEqual([]);
		const inventory = input.certInventory[SAMPLE_IDS.edge];
		if (!inventory) throw new Error("fixture");
		inventory.updated_at = SAMPLE_NOW - 7_201;
		expect(
			evaluate("certificate_inventory_stale", { ...input })[0]?.action?.code,
		).toBe("connect_live");
		inventory.updated_at = null;
		expect(
			evaluate("certificate_inventory_stale", { ...input })[0]?.copy.params,
		).toEqual({ device: "edge-berlin-01" });
		const warehouse = input.certInventory[SAMPLE_IDS.warehouse];
		if (!warehouse) throw new Error("fixture");
		warehouse.updated_at = SAMPLE_NOW - 30 * DAY;
		expect(
			evaluate("certificate_inventory_stale", { ...input }).some(
				(item) =>
					item.subject.kind === "certificate" &&
					item.subject.deviceId === SAMPLE_IDS.warehouse,
			),
		).toBe(false);
	});

	test("28 of 32 certificate slots", () => {
		const input = sampleFleet();
		const { certificates } = edgeLive(input);
		for (let index = certificates.length; index < 28; index++)
			certificates.push({
				...certificates[0],
				certificate_id: `00000000-0000-4000-a000-${String(index).padStart(12, "0")}`,
			});
		expect(
			evaluate("certificate_slots_nearly_full", input)[0]?.copy.params,
		).toMatchObject({ used: 28, max: 32 });
	});
});

describe("organisation authorities on this computer", () => {
	test("signing key within 30 days is a Warning, expired Critical; root within 90 days", () => {
		const input = sampleFleet();
		expect(evaluate("org_ca_signing_key_expiring", input)).toEqual([]);
		input.local.authorities[0].issuingNotAfter = SAMPLE_NOW + 10 * DAY;
		input.local.authorities[0].rootNotAfter = SAMPLE_NOW + 80 * DAY;
		expect(evaluate("org_ca_signing_key_expiring", input)[0]?.severity).toBe(
			"warning",
		);
		expect(evaluate("org_ca_root_expiring", input)[0]?.copy.params?.label).toBe(
			"Rheosoph Internal",
		);
		input.local.authorities[0].issuingNotAfter = SAMPLE_NOW - 1;
		expect(evaluate("org_ca_signing_key_expiring", input)[0]?.severity).toBe(
			"critical",
		);
	});
});
