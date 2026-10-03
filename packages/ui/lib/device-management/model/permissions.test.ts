import { describe, expect, test } from "bun:test";
import type { Capability } from "../types";
import {
	CAPABILITIES,
	DEVICE_ONLY_CAPABILITIES,
	PERMISSION_PRESETS,
	PERMISSION_PRESET_IDS,
	capabilitiesAllowedFor,
	orderCapabilities,
	presetOf,
	presetsFor,
	refusedFor,
	runsCode,
} from "./permissions";

describe("capabilities", () => {
	test("all 12 in IA §6.6.2 order, three of them whole-device only", () => {
		expect(CAPABILITIES).toEqual([
			"status",
			"logs",
			"metrics",
			"deploy",
			"start",
			"stop",
			"restart",
			"remove",
			"scale",
			"update_agent",
			"reboot",
			"manage_certificates",
		]);
		expect(DEVICE_ONLY_CAPABILITIES).toEqual([
			"update_agent",
			"reboot",
			"manage_certificates",
		]);
	});

	test("ordering drops duplicates", () => {
		expect(orderCapabilities(["reboot", "status", "reboot", "logs"])).toEqual([
			"status",
			"logs",
			"reboot",
		]);
	});

	test("device-only capabilities are refused for narrower scopes", () => {
		expect(capabilitiesAllowedFor({ kind: "device" })).toEqual(CAPABILITIES);
		for (const scope of ["project", "placement"] as const) {
			const allowed = capabilitiesAllowedFor(scope);
			expect(allowed).toHaveLength(9);
			for (const capability of DEVICE_ONLY_CAPABILITIES)
				expect(allowed).not.toContain(capability);
		}
		expect(
			refusedFor({ kind: "project", project_id: "invoice-ai" }, [
				"manage_certificates",
				"status",
				"reboot",
			]),
		).toEqual(["reboot", "manage_certificates"]);
		expect(refusedFor("device", ["reboot"])).toEqual([]);
	});

	test("code-running capabilities", () => {
		expect(runsCode(["status", "logs", "metrics", "stop"])).toBe(false);
		for (const capability of ["deploy", "start", "restart", "scale"] as const)
			expect(runsCode([capability])).toBe(true);
	});
});

describe("permission presets", () => {
	test("presets round-trip with their size", () => {
		expect(presetOf(PERMISSION_PRESETS.operator)).toEqual({
			preset: "operator",
			count: 7,
		});
		const counts = PERMISSION_PRESET_IDS.map((id) =>
			presetOf(PERMISSION_PRESETS[id]),
		);
		expect(counts).toEqual([
			{ preset: "viewer", count: 3 },
			{ preset: "operator", count: 7 },
			{ preset: "deployer", count: 9 },
			{ preset: "device_admin", count: 12 },
		]);
	});

	test("order and duplicates don't change the preset", () => {
		const shuffled: Capability[] = ["metrics", "logs", "status", "logs"];
		expect(presetOf(shuffled)).toEqual({ preset: "viewer", count: 3 });
	});

	test("anything else is custom", () => {
		expect(presetOf(["status", "deploy"])).toEqual({
			preset: "custom",
			count: 2,
		});
		expect(presetOf([])).toEqual({ preset: "custom", count: 0 });
	});

	test("Device admin is offered only for the whole device", () => {
		expect(presetsFor("device")).toEqual(PERMISSION_PRESET_IDS);
		expect(presetsFor({ kind: "project", project_id: "invoice-ai" })).toEqual([
			"viewer",
			"operator",
			"deployer",
		]);
	});
});
