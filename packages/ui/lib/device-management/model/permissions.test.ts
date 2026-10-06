import { describe, expect, test } from "bun:test";
import type { Capability } from "../types";
import {
	CAPABILITIES,
	DEVICE_ONLY_CAPABILITIES,
	KNOWN_CAPABILITIES,
	PERMISSION_PRESETS,
	PERMISSION_PRESET_IDS,
	agentAccepts,
	capabilitiesAllowedFor,
	orderCapabilities,
	presetOf,
	presetsFor,
	refusedFor,
	runsCode,
	unknownCapabilities,
} from "./permissions";

describe("capabilities", () => {
	test("capabilities in display order, three of them whole-device only", () => {
		expect(CAPABILITIES).toEqual([
			"status",
			"logs",
			"metrics",
			"service_connect",
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
			expect(allowed).toHaveLength(10);
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
		for (const capability of [
			"deploy",
			"start",
			"restart",
			"scale",
			"service_connect",
		] as const)
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
			{ preset: "device_admin", count: 13 },
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

describe("model capabilities", () => {
	const host = { model_host: 1 } as const;

	test("are known and ordered but not offered to agents without model_host", () => {
		expect(KNOWN_CAPABILITIES.slice(-2)).toEqual(["model_use", "model_manage"]);
		expect(CAPABILITIES).not.toContain("model_use");
		expect(PERMISSION_PRESETS.device_admin).toEqual(CAPABILITIES);
		expect(
			orderCapabilities(["model_manage", "status", "model_use", "status"]),
		).toEqual(["status", "model_use", "model_manage"]);
		for (const features of [undefined, {}, { model_store: 1 } as const]) {
			expect(agentAccepts("model_use", features)).toBe(false);
			expect(capabilitiesAllowedFor("device", features)).toEqual(CAPABILITIES);
			expect(presetsFor("device", features)).toEqual(PERMISSION_PRESET_IDS);
		}
		expect(agentAccepts("status", undefined)).toBe(true);
	});

	test("model permissions need an explicit hub signal too", () => {
		expect(capabilitiesAllowedFor("device", host)).toEqual(CAPABILITIES);
		expect(presetsFor("device", host)).not.toContain("model_user");
		expect(
			capabilitiesAllowedFor("device", host, [
				...CAPABILITIES,
				"model_use",
				"future_capability",
			]),
		).toEqual([...CAPABILITIES, "model_use"]);
	});

	test("need the whole device on an agent with model_host", () => {
		expect(capabilitiesAllowedFor("device", host, KNOWN_CAPABILITIES)).toEqual(
			KNOWN_CAPABILITIES,
		);
		for (const scope of ["project", "placement"] as const) {
			expect(capabilitiesAllowedFor(scope, host)).toEqual(
				capabilitiesAllowedFor(scope),
			);
			expect(presetsFor(scope, host)).not.toContain("model_user");
		}
		expect(refusedFor("project", ["model_use", "status"])).toEqual([
			"model_use",
		]);
		expect(runsCode(["model_use", "model_manage"])).toBe(false);
	});

	test("Use models on this device is a preset of its own", () => {
		expect(presetsFor("device", host, KNOWN_CAPABILITIES)).toEqual([
			...PERMISSION_PRESET_IDS,
			"model_user",
		]);
		expect(presetOf(["model_use"])).toEqual({ preset: "model_user", count: 1 });
	});

	test("Device admin is every permission the agent accepts: all 15 where it hosts models", () => {
		expect(
			presetOf(capabilitiesAllowedFor("device", host, KNOWN_CAPABILITIES)),
		).toEqual({
			preset: "device_admin",
			count: 15,
		});
		expect(presetOf([...KNOWN_CAPABILITIES].reverse())).toEqual({
			preset: "device_admin",
			count: 15,
		});
		expect(presetOf([...CAPABILITIES, "model_use"])).toEqual({
			preset: "custom",
			count: 14,
		});
		expect(presetOf(["status", "model_use", "model_manage"])).toEqual({
			preset: "custom",
			count: 3,
		});
	});

	test("permissions a newer client gave are named, never dropped silently", () => {
		const held = ["status", "unsupported", "model_use"] as Capability[];
		expect(unknownCapabilities(held)).toEqual(["unsupported"]);
		expect(unknownCapabilities(KNOWN_CAPABILITIES)).toEqual([]);
		expect(orderCapabilities(held)).toEqual(["status", "model_use"]);
	});
});
