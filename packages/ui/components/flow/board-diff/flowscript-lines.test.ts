import { describe, expect, it } from "bun:test";
import { projectFlowScript } from "./flowscript-lines";

const SOURCE = [
	"use ai::*",
	"",
	"eventsInboundEmail onInvoiceEmail(from: string) {   //@n:ev",
	'    const invoice = extract({ model: "claude-sonnet-5" })   //@n:ext',
	"    if (duplicate) {   //@n:brD",
	'        reply({ to: from, body: "dup" })   //@n:dupReply',
	"    } else {",
	'        insert({ table: "invoices_v2" })   //@n:ins',
	"    }",
	"}",
].join("\n");

describe("projectFlowScript", () => {
	it("strips anchors without shifting lines", () => {
		const { text } = projectFlowScript(SOURCE);
		const lines = text.split("\n");
		expect(lines).toHaveLength(10);
		expect(lines[3]).toBe(
			'    const invoice = extract({ model: "claude-sonnet-5" })',
		);
		expect(text).not.toContain("//@");
	});

	it("gives closing braces to the statement that opened the block", () => {
		const { owners, firstLine } = projectFlowScript(SOURCE);
		expect(owners).toEqual([
			undefined,
			undefined,
			"ev",
			"ext",
			"brD",
			"dupReply",
			"brD",
			"ins",
			"brD",
			"ev",
		]);
		expect(firstLine.get("brD")).toBe(5);
	});

	it("leaves anchor-shaped text inside strings alone", () => {
		const { text, owners } = projectFlowScript(
			'log({ message: "see //@n:fake" })   //@n:real',
		);
		expect(text).toBe('log({ message: "see //@n:fake" })');
		expect(owners).toEqual(["real"]);
	});
});
