import { describe, expect, test } from "bun:test";
import { createI18n } from "@flow-like/locales";
import { formatTypedNumber } from "../run/format";
import { shortWords } from "../stage/test-words";
import { fixture } from "../testing/fixtures";
import { keepIds, previewText, runChangesOf } from "./run-text";

type T = Parameters<typeof previewText>[0];

const t = ((_key: string, fallback: string) => fallback) as unknown as T;
const english = createI18n({ language: "en" }).getFixedT("en", "interfaces");

describe("keepIds", () => {
	test("a hyphen between letters or digits becomes a non-breaking hyphen", () => {
		expect(keepIds("invoice-RE-2026-0917.pdf")).toBe(
			"invoice\u{2011}RE\u{2011}2026\u{2011}0917.pdf",
		);
		expect(keepIds("29 match PO-47977.")).toBe("29 match PO\u{2011}47977.");
	});

	test("a dash that is not inside an identifier stays", () => {
		for (const text of [
			"Off - On",
			"- first",
			"last -",
			"a -- b",
			"6 –> 7",
			"",
		])
			expect(keepIds(text)).toBe(text);
	});
});

describe("previewText: where a run stopped", () => {
	const step = { number: 2, title: "Run OCR" };

	test("a failed run names the step in quotes, as the canvas does", () => {
		expect(previewText(english, { kind: "failedAt", step })).toBe(
			"Failed at “Run OCR”.",
		);
	});

	test("a stopped run reads alike", () => {
		expect(previewText(english, { kind: "stoppedAt", step })).toBe(
			"Stopped at “Run OCR”.",
		);
	});
});

describe("runChangesOf", () => {
	const state = fixture("failed");
	const context = {
		fields: state.form.fields,
		words: shortWords(),
		isSecret: () => false,
	};
	const number = (text: string) => formatTypedNumber(text, "en-GB", ".");

	test("a changed amount reads grouped, as in the chips", () => {
		const [run, older] = state.runs;
		const changes = runChangesOf(run, older, context, number);
		expect(changes.kind).toBe("diff");
		if (changes.kind !== "diff") return;
		expect(
			changes.items.find((item) => item.label === "Expected total"),
		).toMatchObject({ from: "6,188.00", to: "11,769.10" });
	});

	test("the first run, and a run with the same inputs as the one before", () => {
		const [run] = state.runs;
		expect(runChangesOf(run, undefined, context, number)).toEqual({
			kind: "first",
		});
		expect(runChangesOf(run, run, context, number)).toEqual({
			kind: "same",
			n: run.n,
		});
	});
});

describe("previewText: a result nobody knows", () => {
	test("a saved run was going when the page closed", () => {
		expect(previewText(t, { kind: "unknown" }, "history")).toBe(
			"This run was going when the page closed.",
		);
		expect(previewText(t, { kind: "unknown" })).toBe(
			"This run was going when the page closed.",
		);
	});

	test("a run of this session lost its connection; the form was never closed", () => {
		expect(previewText(t, { kind: "unknown" }, "session")).toBe(
			"The connection ended before this run reported a result.",
		);
	});

	test("other previews do not depend on where the run comes from", () => {
		expect(previewText(t, { kind: "nothing" }, "session")).toBe(
			"This run returned nothing.",
		);
	});
});
