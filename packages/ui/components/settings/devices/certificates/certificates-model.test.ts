import { describe, expect, test } from "bun:test";
import type { LocalCertificateAuthority } from "../../../../lib/device-management/certificate-authority";
import {
	SAMPLE_IDS,
	sampleFleet,
} from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import {
	DAY_S,
	type FleetInput,
	addressProblem,
	authorityDates,
	authorityView,
	buildCertificateFleet,
	canSign,
	filterFleet,
	findCertificate,
	fleetCounts,
	needsYou,
	selfRenewing,
	signedBy,
	suffixProblem,
	upcomingReminders,
	validYears,
} from "./certificates-model";

function golden(patch: Partial<FleetInput> = {}): FleetInput {
	const seed = sampleFleet();
	return {
		devices: seed.devices,
		me: seed.me,
		now: seed.now,
		inventory: seed.certInventory,
		fleetKnown: true,
		live: seed.live,
		...patch,
	};
}

const names = (entries: readonly { deviceName: string }[]) =>
	entries.map((entry) => entry.deviceName);

describe("certificate fleet", () => {
	test("lists every reported certificate most urgent first and joins what a live read added", () => {
		const fleet = buildCertificateFleet(golden());
		expect(names(fleet.rows)).toEqual([
			"warehouse-pi",
			"edge-berlin-01",
			"edge-berlin-01",
		]);
		const [expired, soon, valid] = fleet.rows;
		expect(expired?.days).toBeLessThan(0);
		expect(expired?.detail).toBeUndefined();
		expect(expired?.mode).toBeNull();
		expect(soon?.detail?.label).toBe("internal-mqtt");
		expect(soon?.mode).toBe("delegated");
		expect(soon?.failing).toBe(true);
		expect(valid?.detail?.label).toBe("edge-api");
		expect(valid?.mode).toBe("acme");
		expect(valid?.failing).toBe(false);
		expect(fleet.devicesWithCertificates).toBe(2);
	});

	test("says why a device lists no certificate, and leaves revoked devices out", () => {
		const fleet = buildCertificateFleet(golden());
		const states = Object.fromEntries(
			fleet.silent.map((entry) => [entry.deviceName, entry.state]),
		);
		expect(states).toEqual({
			"cold-storage-nas": "never",
			"studio-mac-mini": "never",
			"lab-gpu-02": "noaccess",
		});
		expect(fleet.revoked.length).toBeGreaterThan(0);
		const active = new Set(fleet.devices.map((row) => row.device_id));
		for (const row of fleet.revoked)
			expect(active.has(row.device_id)).toBe(false);
	});

	test("an older hub reads device by device: refused, still reading and not read are told apart", () => {
		const { certInventory } = sampleFleet();
		const fleet = buildCertificateFleet(
			golden({
				fleetKnown: false,
				inventory: { [SAMPLE_IDS.edge]: certInventory[SAMPLE_IDS.edge] },
				perDevice: {
					[SAMPLE_IDS.lab]: "forbidden",
					[SAMPLE_IDS.warehouse]: "reading",
					[SAMPLE_IDS.studio]: "failed",
				},
			}),
		);
		const states = Object.fromEntries(
			fleet.silent.map((entry) => [entry.deviceName, entry.state]),
		);
		expect(states["lab-gpu-02"]).toBe("noaccess");
		expect(states["warehouse-pi"]).toBe("reading");
		expect(states["studio-mac-mini"]).toBe("unread");
		expect(states["cold-storage-nas"]).toBe("unread");
		expect(fleet.rows).toHaveLength(2);
	});

	test("a certificate needs the reader when it expired or expires within a week without a working renewal", () => {
		const fleet = buildCertificateFleet(golden());
		const [expired, soon, valid] = fleet.rows;
		expect(expired && needsYou(expired)).toBe(true);
		expect(soon && needsYou(soon)).toBe(true);
		expect(soon && selfRenewing(soon)).toBe(false);
		expect(valid && needsYou(valid)).toBe(false);
		expect(valid && selfRenewing(valid)).toBe(true);
	});

	test("counts the six windows; an expired certificate is critical only when a service uses it", () => {
		const counts = fleetCounts(buildCertificateFleet(golden()));
		expect(counts.expired).toHaveLength(1);
		expect(counts.week).toHaveLength(1);
		expect(counts.month).toHaveLength(1);
		expect(counts.errors).toHaveLength(1);
		expect(names(counts.never)).toEqual([
			"cold-storage-nas",
			"studio-mac-mini",
		]);
		expect(names(counts.noAccess)).toEqual(["lab-gpu-02"]);
		expect(counts.expiredInUse).toBe(false);
		expect(counts.renewalUnknown).toBe(1);
		expect(counts.needYou).toBe(2);
	});

	test("filters by window and device; the device-only windows show no certificate rows", () => {
		const fleet = buildCertificateFleet(golden());
		expect(names(filterFleet(fleet, "expired", null).rows)).toEqual([
			"warehouse-pi",
		]);
		expect(filterFleet(fleet, "expired", null).silent).toEqual([]);
		expect(filterFleet(fleet, null, SAMPLE_IDS.edge).rows).toHaveLength(2);
		const silent = filterFleet(fleet, "silent", null);
		expect(silent.devicesOnly).toBe(true);
		expect(silent.rows).toEqual([]);
		expect(names(silent.silent)).toEqual([
			"cold-storage-nas",
			"studio-mac-mini",
		]);
		expect(names(filterFleet(fleet, "noaccess", null).silent)).toEqual([
			"lab-gpu-02",
		]);
	});
});

describe("reminders", () => {
	test("works out the stages still ahead of each certificate, soonest first", () => {
		const input = golden();
		const upcoming = upcomingReminders(
			buildCertificateFleet(input).rows,
			input.now,
		);
		expect(upcoming.map((entry) => entry.row.detail?.label)).toEqual([
			"internal-mqtt",
			"edge-api",
			undefined,
		]);
		expect(upcoming[0]?.next.map((stage) => stage.stage)).toEqual([
			"three_days",
			"day",
			"expired",
		]);
		expect(upcoming[1]?.next).toHaveLength(4);
		expect(upcoming[2]?.next).toEqual([]);
	});

	test("finds a certificate by the ID a reminder names, from its first eight characters on", () => {
		const { rows } = buildCertificateFleet(golden());
		const [first] = rows;
		expect(findCertificate(rows, "1234")).toEqual({ kind: "short" });
		expect(findCertificate(rows, "00000000-0000")).toEqual({ kind: "missing" });
		const found = findCertificate(
			rows,
			` ${first?.certificateId.slice(0, 13).toUpperCase()} `,
		);
		expect(found.kind === "found" && found.row.key).toBe(first?.key ?? "");
	});
});

function authority(
	patch: Partial<LocalCertificateAuthority["public_bundle"]> = {},
): LocalCertificateAuthority {
	return {
		public_bundle: {
			account_binding: "binding",
			authority_id: "7e2d9c41-58af-4b3e-9d61-2c8f0a7b5e14",
			label: "Rheosoph Internal",
			dns_suffixes: ["lab.internal"],
			ip_addresses: ["10.0.4.20"],
			root_certificate_pem: "root",
			issuer_certificate_pem: "issuer",
			sha256_fingerprint: "a".repeat(64),
			not_before: 1_000,
			not_after: 1_000 + 1_000 * DAY_S,
			issuer_not_after: 1_000 + 365 * DAY_S,
			...patch,
		},
		vault: new Uint8Array(64),
	};
}

describe("organisation authorities", () => {
	test("is active until its signing key or root expires, and warns 30 and 90 days before", () => {
		const now = 1_000 + 100 * DAY_S;
		expect(authorityView(authority(), now).status).toBe("active");
		expect(authorityView(authority(), now).signingSoon).toBe(false);
		const soon = authorityView(
			authority({ issuer_not_after: now + 11 * DAY_S }),
			now,
		);
		expect(soon.signingSoon).toBe(true);
		expect(canSign(soon)).toBe(true);
		const expired = authorityView(
			authority({ issuer_not_after: now - 1 }),
			now,
		);
		expect(expired.status).toBe("signing_expired");
		expect(expired.signingSoon).toBe(false);
		expect(canSign(expired)).toBe(false);
		expect(authorityView(authority({ not_after: now - 1 }), now).status).toBe(
			"root_expired",
		);
		expect(
			authorityView(authority({ not_after: now + 60 * DAY_S }), now).rootSoon,
		).toBe(true);
	});

	test("attributes certificates to an authority by the issuer name a live read reported", () => {
		const { rows } = buildCertificateFleet(golden());
		expect(
			signedBy(rows, "Rheosoph Internal").map((row) => row.detail?.label),
		).toEqual(["internal-mqtt"]);
		expect(signedBy(rows, "Someone Else")).toEqual([]);
	});

	test("refuses names an authority can never sign for", () => {
		const none = new Set<string>();
		expect(suffixProblem("lab.internal", none)).toBeNull();
		expect(suffixProblem("  ", none)).toBeNull();
		expect(suffixProblem("*.lab.internal", none)).toBe("wildcard");
		expect(suffixProblem("lab.internal:8443", none)).toBe("port_or_path");
		expect(suffixProblem("https://lab.internal", none)).toBe("port_or_path");
		expect(suffixProblem("lab.internal.", none)).toBe("trailing_dot");
		expect(suffixProblem("lab_internal", none)).toBe("characters");
		expect(suffixProblem("10.0.4", none)).toBe("numeric_end");
		expect(suffixProblem("Lab.Internal", new Set(["lab.internal"]))).toBe(
			"duplicate",
		);
		expect(addressProblem("10.0.4.20", none)).toBeNull();
		expect(addressProblem("fd00::20", none)).toBeNull();
		expect(addressProblem("10.0.4.0/24", none)).toBe("not_an_address");
		expect(addressProblem("10.0.4.20", new Set(["10.0.4.20"]))).toBe(
			"duplicate",
		);
	});

	test("a root lasts 1 to 10 whole years and its signing key one year at most", () => {
		expect(validYears("3")).toBe(3);
		expect(validYears("0")).toBeNull();
		expect(validYears("11")).toBeNull();
		expect(validYears("2.5")).toBeNull();
		expect(validYears("")).toBeNull();
		expect(authorityDates(3, 0)).toEqual({
			root: 3 * 365 * DAY_S,
			signing: 365 * DAY_S,
		});
		expect(authorityDates(1, 0).signing).toBe(365 * DAY_S);
	});
});
