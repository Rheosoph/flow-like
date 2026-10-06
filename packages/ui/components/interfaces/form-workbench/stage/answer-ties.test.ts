import { describe, expect, test } from "bun:test";
import { tieAnswer } from "./answer-ties";

const NB = "\u{a0}";

describe("tieAnswer", () => {
	test("a date, an amount and a percentage stay on one line", () => {
		expect(tieAnswer("Due 17 Oct 2026.")).toBe(`Due 17${NB}Oct${NB}2026.`);
		expect(tieAnswer("which is € 714.00 above")).toBe(
			`which is €${NB}714.00 above`,
		);
		expect(tieAnswer("VAT 19 % of the net")).toBe(
			`VAT 19${NB}% of the${NB}net`,
		);
		expect(tieAnswer("12,483.10 € gross")).toBe(`12,483.10${NB}€ gross`);
	});

	test("months in full, abbreviated with a dot, and 'Sept'", () => {
		expect(tieAnswer("on 17 October 2026")).toBe(`on 17${NB}October${NB}2026`);
		expect(tieAnswer("on 17 Sept 2026")).toBe(`on 17${NB}Sept${NB}2026`);
		expect(tieAnswer("on 3 Okt. 2026")).toBe(`on 3${NB}Okt.${NB}2026`);
	});

	test("several ties in one line, and a date at the start of it", () => {
		expect(tieAnswer("17 Sep 2026, 17 Oct 2026")).toBe(
			`17${NB}Sep${NB}2026, 17${NB}Oct${NB}2026`,
		);
	});

	test("the label of a date or amount that closes the line stays with it", () => {
		expect(tieAnswer("· Due: 17 Oct 2026")).toBe(
			`· Due:${NB}17${NB}Oct${NB}2026`,
		);
		expect(tieAnswer("**Due:** 17 Oct 2026")).toBe(
			`**Due:**${NB}17${NB}Oct${NB}2026`,
		);
		expect(tieAnswer("Gross total: € 12,483.10")).toBe(
			`Gross total:${NB}€${NB}12,483.10`,
		);
		expect(tieAnswer("Due: 17 Oct 2026.")).toBe(
			`Due:${NB}17${NB}Oct${NB}2026.`,
		);
		expect(tieAnswer("(**Due:** 17 Oct 2026)")).toBe(
			`(**Due:**${NB}17${NB}Oct${NB}2026)`,
		);
	});

	test("a label in the middle of a line is not tied: only the last value would be left alone on a line", () => {
		expect(tieAnswer("Invoice date: 17 Sep 2026 · Due: 17 Oct 2026")).toBe(
			`Invoice date: 17${NB}Sep${NB}2026${NB}· Due:${NB}17${NB}Oct${NB}2026`,
		);
		expect(tieAnswer("Due: 17 Oct 2026 and more")).toBe(
			`Due: 17${NB}Oct${NB}2026 and${NB}more`,
		);
		expect(tieAnswer("Gross: € 12,483.10 incl. VAT")).toBe(
			`Gross: €${NB}12,483.10 incl.${NB}VAT`,
		);
	});

	test("a dot between two items stays with the word before it, so no line starts with one", () => {
		expect(tieAnswer("Nordwind GmbH · Invoice date · Due")).toBe(
			`Nordwind GmbH${NB}· Invoice date${NB}· Due`,
		);
		expect(tieAnswer("a • b")).toBe(`a${NB}• b`);
		expect(tieAnswer("**Vendor:** Nordwind · **Due:** 17 Oct 2026")).toBe(
			`**Vendor:** Nordwind${NB}· **Due:**${NB}17${NB}Oct${NB}2026`,
		);
	});

	test("a dot that starts the text, a bullet list marker and a dot inside a word are left alone", () => {
		for (const text of ["· first", "• item", "3·4", "a ·b", "a·"])
			expect(tieAnswer(text)).toBe(text);
	});

	test("what is no date, no amount and no label gets no value tie, only its last two words kept together", () => {
		for (const [text, tied] of [
			["All 14 pages were read.", `All 14 pages were${NB}read.`],
			["Note: nothing to see 17", `Note: nothing to see${NB}17`],
			["2. October 2026 starts a list", `2. October 2026 starts a${NB}list`],
			["2017 Oct 2026", `2017 Oct${NB}2026`],
			["a time 12:30 and 17 Oct 20", `a time 12:30 and 17 Oct${NB}20`],
			["17 oct 2026", `17 oct${NB}2026`],
			["", ""],
		])
			expect(tieAnswer(text)).toBe(tied);
	});

	test("a line keeps its last two plain words together, so a paragraph never ends on one word alone", () => {
		expect(tieAnswer("The invoice can be approved.")).toBe(
			`The invoice can be${NB}approved.`,
		);
		expect(tieAnswer("Hard break at the end  ")).toBe(
			`Hard break at the${NB}end  `,
		);
		for (const text of [
			"Two words",
			"| Line | Net amount |",
			"    indented code line here",
			"run it with `npm run build`",
		])
			expect(tieAnswer(text)).toBe(text);
	});

	test("a value that closes the line is not tied to the word before it, so no long unbreakable run forms", () => {
		expect(tieAnswer("Paid on 17 Oct 2026")).toBe(
			`Paid on 17${NB}Oct${NB}2026`,
		);
		expect(tieAnswer("which is € 714.00")).toBe(`which is €${NB}714.00`);
	});

	test("partial text while the answer streams ties once the year is complete", () => {
		expect(tieAnswer("Due 17 Oct 20")).toBe(`Due 17 Oct${NB}20`);
		expect(tieAnswer("Due 17 Oct 2026")).toBe(`Due 17${NB}Oct${NB}2026`);
	});

	test("applying it twice changes nothing more", () => {
		const once = tieAnswer("**Due:** 17 Oct 2026 · € 714.00 · 19 %");
		expect(tieAnswer(once)).toBe(once);
	});

	test("inline code and fenced code keep their spaces", () => {
		expect(tieAnswer("`17 Oct 2026` and 17 Oct 2026")).toBe(
			`\`17 Oct 2026\` and 17${NB}Oct${NB}2026`,
		);
		const fenced = ["```", "17 Oct 2026 € 5 19 %", "```", "17 Oct 2026"].join(
			"\n",
		);
		expect(tieAnswer(fenced)).toBe(
			["```", "17 Oct 2026 € 5 19 %", "```", `17${NB}Oct${NB}2026`].join("\n"),
		);
		const tilde = ["~~~md", "€ 5", "~~~", "€ 5"].join("\n");
		expect(tieAnswer(tilde)).toBe(
			["~~~md", "€ 5", "~~~", `€${NB}5`].join("\n"),
		);
	});

	test("a fence that is still open while streaming keeps the rest as code", () => {
		const open = ["```", "17 Oct 2026"].join("\n");
		expect(tieAnswer(open)).toBe(open);
	});

	test("a shorter fence does not close a longer one", () => {
		const text = ["````", "```", "€ 5", "````", "€ 5"].join("\n");
		expect(tieAnswer(text)).toBe(
			["````", "```", "€ 5", "````", `€${NB}5`].join("\n"),
		);
	});

	test("table cells are tied and stay numbers for the numeric columns", () => {
		expect(tieAnswer("| 1 | Pallets | € 410.00 | 19 % |")).toBe(
			`| 1 | Pallets | €${NB}410.00 | 19${NB}% |`,
		);
	});
});
