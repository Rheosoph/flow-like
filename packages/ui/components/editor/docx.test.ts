import { expect, test } from "bun:test";
import { Window } from "happy-dom";
import { strToU8, unzipSync, strFromU8, zipSync } from "fflate";
import { exportDocx, importDocx } from "./docx";

const window = new Window();
Object.assign(globalThis, { DOMParser: window.DOMParser });
const text = (nodes: any[]): string =>
	nodes.map((node) => node.text ?? text(node.children ?? [])).join("");

test("Word document round trip preserves headings, marks, links, lists and tables", () => {
	const value: any[] = [
		{ type: "h1", children: [{ text: "Morning edition" }] },
		{
			type: "p",
			children: [
				{ text: "Breaking ", bold: true },
				{
					type: "a",
					url: "https://example.test/source",
					children: [{ text: "source" }],
				},
			],
		},
		{
			type: "p",
			listStyleType: "decimal",
			indent: 2,
			children: [{ text: "Report item" }],
		},
		{
			type: "table",
			children: [
				{
					type: "tr",
					children: [
						{
							type: "td",
							colSpan: 2,
							children: [{ type: "p", children: [{ text: "Total 42" }] }],
						},
					],
				},
			],
		},
	];
	const archive = exportDocx(value);
	const entries = unzipSync(archive);
	expect(entries["[Content_Types].xml"]).toBeDefined();
	expect(strFromU8(entries["word/document.xml"])).toContain(
		'w:pStyle w:val="Heading1"',
	);
	const imported = importDocx(archive);
	expect(imported.warnings).toEqual([]);
	expect(imported.value[0].type).toBe("h1");
	expect(text(imported.value)).toBe(
		"Morning editionBreaking sourceReport itemTotal 42",
	);
	expect(imported.value[1].children[0].bold).toBe(true);
	expect(imported.value[1].children[1].url).toBe("https://example.test/source");
	expect(imported.value[2]).toMatchObject({
		listStyleType: "decimal",
		indent: 2,
	});
	expect(imported.value[3]).toMatchObject({
		children: [{ children: [{ colSpan: 2 }] }],
	});
});

test("Word import strips unsafe hyperlink targets", () => {
	const archive = exportDocx([
		{
			type: "p",
			children: [
				{
					type: "a",
					url: "https://example.test",
					children: [{ text: "source" }],
				},
			],
		},
	]);
	const entries = unzipSync(archive);
	entries["word/_rels/document.xml.rels"] = strToU8(
		strFromU8(entries["word/_rels/document.xml.rels"]).replace(
			"https://example.test",
			"javascript:alert(1)",
		),
	);
	const imported = importDocx(zipSync(entries));
	expect(text(imported.value)).toBe("source");
	expect(JSON.stringify(imported.value)).not.toContain("javascript:");
});

test("Word archive embeds image bytes and preserves alternative text", () => {
	const src =
		"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/l9sAAAAASUVORK5CYII=";
	const archive = exportDocx([
		{
			type: "img",
			url: src,
			alt: "The harbor",
			caption: [{ text: "At dawn" }],
			children: [{ text: "" }],
		},
	]);
	const imported = importDocx(archive);
	const image = imported.value.find((node) => node.type === "img");
	expect(image?.url).toBe(src);
	expect(image?.alt).toBe("The harbor");
	expect(text(imported.value)).toContain("At dawn");
});

test("Word export and import retain footnote references and definitions", () => {
	const value: any[] = [
		{
			type: "p",
			children: [
				{ text: "Evidence" },
				{
					type: "footnoteReference",
					identifier: "evidence-note",
					children: [{ text: "" }],
				},
			],
		},
		{
			type: "footnoteDefinition",
			identifier: "evidence-note",
			children: [
				{
					type: "p",
					children: [
						{
							type: "a",
							url: "https://example.test/evidence",
							children: [{ text: "Source document" }],
						},
					],
				},
			],
		},
	];
	const archive = exportDocx(value);
	const entries = unzipSync(archive);
	expect(strFromU8(entries["word/footnotes.xml"])).toContain(
		'w:footnote w:id="1"',
	);
	expect(strFromU8(entries["word/footnotes.xml"])).toContain("w:footnoteRef");
	const imported = importDocx(archive);
	expect(JSON.stringify(imported.value)).toContain("footnoteReference");
	expect(imported.value.at(-1)?.type).toBe("footnoteDefinition");
	expect(text(imported.value)).toContain("Source document");
	expect(JSON.stringify(imported.value)).toContain(
		"https://example.test/evidence",
	);
});
