import { describe, expect, test } from "bun:test";
import type { WorkbenchField } from "../contracts";
import {
	formatBytes,
	formatClock,
	formatTook,
	formatTypedNumber,
	withNumberText,
} from "./format";

describe("formatTook", () => {
	test("reads as the canvas receipt", () => {
		expect(formatTook(48_211)).toBe("48 s");
		expect(formatTook(72_000)).toBe("1 min 12 s");
		expect(formatTook(41_000)).toBe("41 s");
		expect(formatTook(2_000)).toBe("2 s");
	});

	test("drops a zero part and rounds to whole seconds", () => {
		expect(formatTook(60_000)).toBe("1 min");
		expect(formatTook(59_600)).toBe("1 min");
		expect(formatTook(400)).toBe("0 s");
		expect(formatTook(3_725_000)).toBe("1 h 2 min");
		expect(formatTook(7_200_000)).toBe("2 h");
	});

	test("never prints a negative or broken duration", () => {
		expect(formatTook(-5_000)).toBe("0 s");
		expect(formatTook(Number.NaN)).toBe("0 s");
	});
});

describe("formatClock", () => {
	test("reads as the live clock", () => {
		expect(formatClock(37_000)).toBe("0:37");
		expect(formatClock(31_999)).toBe("0:31");
		expect(formatClock(131_000)).toBe("2:11");
		expect(formatClock(0)).toBe("0:00");
	});

	test("adds hours past one hour", () => {
		expect(formatClock(3_725_000)).toBe("1:02:05");
		expect(formatClock(-1)).toBe("0:00");
	});
});

describe("formatBytes", () => {
	test("sizes of the canvas files", () => {
		expect(formatBytes(1_284_096)).toBe("1.2 MB");
		expect(formatBytes(6412)).toBe("6.3 KB");
		expect(formatBytes(1873)).toBe("1.8 KB");
		expect(formatBytes(48_640)).toBe("47.5 KB");
		expect(formatBytes(24_117_248)).toBe("23.0 MB");
		expect(formatBytes(455_680)).toBe("445.0 KB");
	});

	test("an unknown size is left out", () => {
		expect(formatBytes(0)).toBe("");
		expect(formatBytes(null)).toBe("");
		expect(formatBytes(undefined)).toBe("");
		expect(formatBytes(Number.NaN)).toBe("");
	});

	test("small, rounding and large sizes", () => {
		expect(formatBytes(1)).toBe("1 byte");
		expect(formatBytes(512)).toBe("512 bytes");
		expect(formatBytes(1_048_575)).toBe("1.0 MB");
		expect(formatBytes(1.5 * 1024 ** 3)).toBe("1.5 GB");
	});

	test("the viewer's decimal sign", () => {
		expect(formatBytes(1_284_096, ",")).toBe("1,2 MB");
	});
});

describe("formatTypedNumber", () => {
	test("the canvas amounts: grouped, the typed decimals kept", () => {
		expect(formatTypedNumber("11769.10", "en-GB", ".")).toBe("11,769.10");
		expect(formatTypedNumber("6188.00", "en-GB", ".")).toBe("6,188.00");
		expect(formatTypedNumber(" 2.5 ", "en-GB", ".")).toBe("2.5");
	});

	test("whole numbers below five digits stay as typed; larger ones are grouped", () => {
		expect(formatTypedNumber("20", "en-GB", ".")).toBe("20");
		expect(formatTypedNumber("1920", "en-GB", ".")).toBe("1920");
		expect(formatTypedNumber("-4400", "en-GB", ".")).toBe("-4400");
		expect(formatTypedNumber("120000", "en-GB", ".")).toBe("120,000");
	});

	test("the viewer's signs", () => {
		expect(formatTypedNumber("11769.10", "de-DE", ",")).toBe("11.769,10");
		expect(formatTypedNumber("6188,00", "de-DE", ",")).toBe("6.188,00");
	});

	test("anything that is not a plain number stays as typed", () => {
		expect(formatTypedNumber("", "en-GB", ".")).toBe("");
		expect(formatTypedNumber("12,5", "en-GB", ".")).toBe("12,5");
		expect(formatTypedNumber("007", "en-GB", ".")).toBe("007");
		expect(formatTypedNumber("1e5", "en-GB", ".")).toBe("1e5");
		expect(formatTypedNumber("12345678901234567", "en-GB", ".")).toBe(
			"12345678901234567",
		);
		expect(formatTypedNumber("••••", "en-GB", ".")).toBe("••••");
		expect(formatTypedNumber("empty", "en-GB", ".")).toBe("empty");
	});
});

describe("withNumberText", () => {
	const field = (
		name: string,
		kind: WorkbenchField["kind"],
		props: readonly WorkbenchField[] = [],
	) =>
		({
			key: name,
			name,
			label: name,
			kind,
			props,
		}) as unknown as WorkbenchField;
	const fields = [
		field("expected_total", "number"),
		field("order", "text"),
		field("terms", "group", [field("terms\u001fnet_days", "number")]),
	];
	const number = (text: string) => formatTypedNumber(text, "en-GB", ".");

	test("number fields and number properties read grouped; other inputs as they were", () => {
		const changes = [
			{
				name: "expected_total",
				label: "Expected total",
				from: "6188.00",
				to: "11769.10",
			},
			{ name: "order", label: "Order", from: "48213", to: "48214.50" },
			{
				name: "terms\u001fnet_days",
				label: "Net days",
				from: "14",
				to: "30000",
			},
		];
		expect(withNumberText(fields, changes, number)).toEqual([
			{
				name: "expected_total",
				label: "Expected total",
				from: "6,188.00",
				to: "11,769.10",
			},
			changes[1],
			{
				name: "terms\u001fnet_days",
				label: "Net days",
				from: "14",
				to: "30,000",
			},
		]);
	});
});
