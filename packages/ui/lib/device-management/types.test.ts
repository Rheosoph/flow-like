import { expect, test } from "bun:test";
import { managementRejection } from "./types";

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
