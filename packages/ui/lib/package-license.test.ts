import { describe, expect, test } from "bun:test";
import {
	isExpiredPin,
	licenseExpiresAt,
	licenseTimeLeft,
	packageNeedsLicense,
	packagePinState,
} from "./package-license";
import type { AppPackageLicense } from "./schema/wasm";

const HOUR = 60 * 60 * 1000;
const DAY = 24 * HOUR;
const NOW = Date.parse("2026-09-24T12:00:00Z");

function license(overrides: Partial<AppPackageLicense>): AppPackageLicense {
	return { required: true, status: "active", graceDays: 30, ...overrides };
}

describe("packagePinState", () => {
	test("pins from servers without licensing keep the stale flag", () => {
		expect(packagePinState({ stale: true }, NOW)).toBe("stale");
		expect(packagePinState({ stale: false }, NOW)).toBe("active");
	});

	test("the server status wins over the stale flag", () => {
		expect(
			packagePinState(
				{ stale: true, license: license({ status: "expired" }) },
				NOW,
			),
		).toBe("expired");
		expect(
			packagePinState(
				{ stale: false, license: license({ status: "active" }) },
				NOW,
			),
		).toBe("active");
	});

	test("a lapsed pin turns expired once its grace ran out on the client", () => {
		const lapsed = (expiresAt: number) => ({
			stale: true,
			license: license({
				status: "lapsed",
				expiresAt: new Date(expiresAt).toISOString(),
			}),
		});
		expect(packagePinState(lapsed(NOW + HOUR), NOW)).toBe("lapsed");
		expect(packagePinState(lapsed(NOW), NOW)).toBe("expired");
	});
});

describe("licenseExpiresAt", () => {
	test("falls back to lapsedAt plus the grace period", () => {
		const lapsedAt = "2026-09-01T00:00:00Z";
		expect(
			licenseExpiresAt({
				stale: true,
				license: license({ status: "lapsed", lapsedAt }),
			}),
		).toBe(Date.parse(lapsedAt) + 30 * DAY);
	});

	test("is unknown without dates", () => {
		expect(
			licenseExpiresAt({ stale: true, license: license({ status: "lapsed" }) }),
		).toBeUndefined();
		expect(licenseExpiresAt({ stale: true })).toBeUndefined();
	});
});

describe("licenseTimeLeft", () => {
	test("counts whole days while at least one day is left", () => {
		expect(licenseTimeLeft(NOW + 23 * DAY + 5 * HOUR, NOW)).toMatchObject({
			days: 23,
		});
		expect(licenseTimeLeft(NOW + DAY, NOW)).toMatchObject({ days: 1 });
	});

	test("switches to hours rounded up in the last day", () => {
		expect(licenseTimeLeft(NOW + 5 * HOUR + 1, NOW)).toMatchObject({
			days: 0,
			hours: 6,
		});
		expect(licenseTimeLeft(NOW + 60_000, NOW)).toMatchObject({
			days: 0,
			hours: 1,
		});
	});

	test("never goes negative", () => {
		expect(licenseTimeLeft(NOW - DAY, NOW)).toEqual({
			remainingMs: 0,
			days: 0,
			hours: 0,
		});
		expect(licenseTimeLeft(undefined, NOW)).toBeUndefined();
	});
});

describe("packageNeedsLicense", () => {
	test("free public packages need no licence", () => {
		expect(packageNeedsLicense({ price: 0, visibility: "public" })).toBe(false);
	});

	test("paid, private and request-access packages do", () => {
		expect(packageNeedsLicense({ price: 499, visibility: "public" })).toBe(
			true,
		);
		expect(packageNeedsLicense({ price: 0, visibility: "private" })).toBe(true);
		expect(
			packageNeedsLicense({ price: 0, visibility: "public_request_access" }),
		).toBe(true);
	});
});

test("isExpiredPin only trusts the server status", () => {
	expect(isExpiredPin({ license: license({ status: "expired" }) })).toBe(true);
	expect(isExpiredPin({ license: license({ status: "lapsed" }) })).toBe(false);
	expect(isExpiredPin({})).toBe(false);
});
