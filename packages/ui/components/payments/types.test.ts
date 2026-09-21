import { describe, expect, it } from "bun:test";
import { paymentPromptRef } from "./payment-events";
import {
	amountInput,
	parseEuroAmount,
	paymentUrl,
	pendingOrder,
} from "./types";

describe("payment amount input", () => {
	it("converts decimal EUR to integer cents without floating point arithmetic", () => {
		expect(parseEuroAmount("3.29")).toBe(329);
		expect(parseEuroAmount("9,99")).toBe(999);
		expect(parseEuroAmount("0.01")).toBe(1);
		expect(parseEuroAmount("25")).toBe(2500);
		expect(amountInput(329)).toBe("3.29");
	});
	it("rejects rounding, exponent notation and unsafe integers", () => {
		for (const value of [
			"",
			"3.291",
			"-1",
			"1e3",
			"NaN",
			"Infinity",
			"01.00",
			"9007199254740992",
		])
			expect(parseEuroAmount(value)).toBeNull();
	});
});

describe("payment navigation", () => {
	it("accepts HTTPS links without embedded credentials", () => {
		expect(paymentUrl("https://checkout.stripe.com/c/pay/cs_test")).toBe(
			"https://checkout.stripe.com/c/pay/cs_test",
		);
		for (const value of [
			"javascript:alert(1)",
			"http://example.com",
			"https://buyer:secret@example.com",
			"/checkout",
		])
			expect(paymentUrl(value)).toBeUndefined();
	});
	it("requires complete opaque scope before fetching a payment prompt", () => {
		expect(
			paymentPromptRef({ id: "req_1", appId: "app_1", runId: "run-1" }),
		).toEqual({ id: "req_1", appId: "app_1", runId: "run-1" });
		expect(paymentPromptRef({ id: "req_1", appId: "app_1" })).toBeNull();
		expect(
			paymentPromptRef({
				id: "req_1?payer=other",
				appId: "app_1",
				runId: "run-1",
			}),
		).toBeNull();
	});
	it("keeps servicing pending cancellation and stops terminal order polling", () => {
		expect(pendingOrder("CREATED")).toBe(true);
		expect(pendingOrder("CANCEL_PENDING")).toBe(true);
		expect(pendingOrder("COMPLETED")).toBe(false);
		expect(pendingOrder("CANCELED")).toBe(false);
	});
});
