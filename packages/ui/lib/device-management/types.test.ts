import { expect, test } from "bun:test";
import { agentFeatures, managementRejection } from "./types";

test("coded rejections expose their reason and retryability", () => {
	expect(
		managementRejection({
			state: "rejected",
			result: {
				error: "Host policy requires Linux isolation",
				code: "host_policy",
				retryable: false,
			},
		}),
	).toEqual({
		code: "host_policy",
		error: "Host policy requires Linux isolation",
		retryable: false,
	});
	expect(
		managementRejection({ state: "rejected", result: { code: "busy" } }),
	).toEqual({
		code: "busy",
		error: "The device rejected this operation.",
		retryable: true,
	});
	expect(
		managementRejection({
			state: "rejected",
			result: { code: "limit", error: "x".repeat(2000) },
		})?.error,
	).toHaveLength(1024);
});

test("older agents and other states carry no rejection reason", () => {
	for (const response of [
		{ state: "rejected", result: { error: "Command rejected" } },
		{ state: "rejected", result: { code: "Not A Code", retryable: false } },
		{ state: "rejected", result: null },
		{ state: "completed", result: { code: "invalid", retryable: false } },
	])
		expect(managementRejection(response)).toBeUndefined();
});

test("agent feature flags keep only well-formed enabled flags", () => {
	expect<unknown>(
		agentFeatures({
			placement_diagnostics: 1,
			task_health: 1,
			future_flag: 1,
			placement_events: 0,
			network_interfaces: true,
			"Bad Flag": 1,
		}),
	).toEqual({ placement_diagnostics: 1, task_health: 1, future_flag: 1 });
	const many = Object.fromEntries(
		Array.from({ length: 100 }, (_, index) => [`flag_${index}`, 1]),
	);
	expect(Object.keys(agentFeatures(many))).toHaveLength(64);
});

test("older agents without a features map have no flags", () => {
	for (const value of [undefined, null, "placement_events", [1], 1])
		expect(agentFeatures(value)).toEqual({});
});
