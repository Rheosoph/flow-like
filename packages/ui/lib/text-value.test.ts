import { describe, expect, test } from "bun:test";
import {
	looksLikeMarkup,
	parseJsonContainer,
	readTextValue,
} from "./text-value";

const INVOICE_XML = `<?xml version="1.0" encoding="UTF-8"?>
<rsm:CrossIndustryInvoice xmlns:rsm="urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100">
  <ram:ID>RE-2026-0824</ram:ID>
</rsm:CrossIndustryInvoice>`;

describe("parseJsonContainer", () => {
	test("reads objects and arrays, padded or not", () => {
		expect(parseJsonContainer('{"a":1}')).toEqual({ a: 1 });
		expect(parseJsonContainer("  [1, 2]\n")).toEqual([1, 2]);
	});

	test("leaves scalars, broken JSON and prose alone", () => {
		expect(parseJsonContainer("42")).toBeUndefined();
		expect(parseJsonContainer('"text"')).toBeUndefined();
		expect(parseJsonContainer("null")).toBeUndefined();
		expect(parseJsonContainer('{"a":')).toBeUndefined();
		expect(parseJsonContainer("[link](https://flow-like.com)")).toBeUndefined();
		expect(parseJsonContainer("{ not json }")).toBeUndefined();
	});
});

describe("looksLikeMarkup", () => {
	test("recognizes XML documents, fragments and HTML", () => {
		expect(looksLikeMarkup(INVOICE_XML)).toBe(true);
		expect(looksLikeMarkup("<ram:ID>1</ram:ID>")).toBe(true);
		expect(looksLikeMarkup("<!DOCTYPE html><html></html>")).toBe(true);
	});

	test("leaves prose that merely mentions tags", () => {
		expect(looksLikeMarkup("Use <b> for bold")).toBe(false);
		expect(looksLikeMarkup("a < b > c")).toBe(false);
		expect(looksLikeMarkup("# Heading")).toBe(false);
	});
});

describe("readTextValue", () => {
	test("a payload holding JSON-in-JSON with XML inside reads as JSON", () => {
		const payload = JSON.stringify({
			review_state: "needs_review",
			draft_payload_json: JSON.stringify({ xml_text: INVOICE_XML }),
		});
		const reading = readTextValue(payload);
		expect(reading.kind).toBe("json");
		if (reading.kind !== "json") return;
		const nested = (reading.value as Record<string, string>).draft_payload_json;
		expect(parseJsonContainer(nested)).toEqual({ xml_text: INVOICE_XML });
	});

	test("XML stays verbatim and prose renders as markdown", () => {
		expect(readTextValue(INVOICE_XML).kind).toBe("markup");
		expect(readTextValue("**Bold** and a list:\n- one").kind).toBe("text");
	});
});
