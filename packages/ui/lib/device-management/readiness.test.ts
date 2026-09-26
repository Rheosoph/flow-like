import { expect, test } from "bun:test";
import { parseDeviceSetupReadiness } from "./readiness";

const response = () => ({
	version: 1,
	ready: true,
	checks: ["policy", "signing", "api", "signaling", "release", "database"].map(
		(id) => ({ id, ready: true, message: "Ready." }),
	),
});
test("setup requires complete consistent checks before enabling enrollment", () => {
	expect(parseDeviceSetupReadiness(response()).ready).toBe(true);
	const blocked = response();
	blocked.checks[0].ready = false;
	expect(() => parseDeviceSetupReadiness(blocked)).toThrow();
	blocked.ready = false;
	expect(parseDeviceSetupReadiness(blocked).ready).toBe(false);
	const duplicate = response();
	duplicate.checks[1] = duplicate.checks[0];
	expect(() => parseDeviceSetupReadiness(duplicate)).toThrow();
	expect(() =>
		parseDeviceSetupReadiness({
			...response(),
			checks: response().checks.slice(1),
		}),
	).toThrow();
});
