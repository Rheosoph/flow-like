import { strFromU8, strToU8, unzipSync, zipSync } from "fflate";
import type { Value } from "platejs";
import { isPublicationUrl } from "./publication";

interface DocxMarks {
	bold?: boolean;
	italic?: boolean;
	underline?: boolean;
	strikethrough?: boolean;
	superscript?: boolean;
	subscript?: boolean;
}

/** The document fields supported by the Word content converter. */
interface DocxNode extends DocxMarks {
	[key: string]: unknown;
	type?: string;
	text?: string;
	children?: DocxNode[];
	url?: string;
	identifier?: string;
	alt?: string;
	name?: string;
	caption?: DocxNode[];
	credit?: string;
	license?: string;
	indent?: number;
	listStyleType?: string;
	align?: string;
	colSpan?: number;
}

interface DocxText extends DocxNode {
	text: string;
}

interface DocxElement extends DocxNode {
	type: string;
	children: DocxDescendant[];
}

type DocxDescendant = DocxText | DocxElement;

const W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const xml = (value: string) =>
	value.replace(
		/[&<>"']/g,
		(char) =>
			({
				"&": "&amp;",
				"<": "&lt;",
				">": "&gt;",
				'"': "&quot;",
				"'": "&apos;",
			})[char] ?? char,
	);
const attr = (element: Element | undefined | null, name: string) =>
	element?.getAttributeNS(W, name) ?? element?.getAttribute(`w:${name}`) ?? "";
const child = (element: Element, name: string) =>
	[...element.children].find((node) => node.localName === name);
const descendants = (element: Element | Document, name: string) =>
	[...element.getElementsByTagName("*")].filter(
		(node) => node.localName === name,
	);
const plain = (node: DocxNode): string =>
	typeof node.text === "string"
		? node.text
		: (node.children ?? []).map(plain).join("");
const bytesToBase64 = (bytes: Uint8Array) => {
	let binary = "";
	for (let offset = 0; offset < bytes.length; offset += 8192)
		binary += String.fromCharCode(...bytes.subarray(offset, offset + 8192));
	return btoa(binary);
};

function imageSize(bytes: Uint8Array, mime: string): [number, number] {
	const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
	if (mime === "image/png" && bytes.length >= 24)
		return [view.getUint32(16), view.getUint32(20)];
	if (mime === "image/gif" && bytes.length >= 10)
		return [view.getUint16(6, true), view.getUint16(8, true)];
	if (mime === "image/jpeg") {
		let offset = 2;
		while (offset + 8 < bytes.length) {
			if (bytes[offset] !== 255) {
				offset++;
				continue;
			}
			const marker = bytes[offset + 1];
			const length = view.getUint16(offset + 2);
			if (
				[
					192, 193, 194, 195, 197, 198, 199, 201, 202, 203, 205, 206, 207,
				].includes(marker)
			)
				return [view.getUint16(offset + 7), view.getUint16(offset + 5)];
			if (length < 2) break;
			offset += length + 2;
		}
	}
	if (
		mime === "image/webp" &&
		bytes.length >= 30 &&
		String.fromCharCode(...bytes.subarray(12, 16)) === "VP8X"
	)
		return [
			1 + bytes[24] + (bytes[25] << 8) + (bytes[26] << 16),
			1 + bytes[27] + (bytes[28] << 8) + (bytes[29] << 16),
		];
	return [4, 3];
}

export interface DocxImport {
	value: Value;
	warnings: string[];
}

/** Imports document content; page headers, tracked changes and drawings are reported separately. */
export function importDocx(bytes: Uint8Array): DocxImport {
	if (bytes.length > 20 * 1024 * 1024)
		throw new Error("DOCX files must be smaller than 20 MiB.");
	let total = 0;
	let oversized = false;
	const files = unzipSync(bytes, {
		filter: (file) => {
			total += file.originalSize;
			if (file.originalSize > 20 * 1024 * 1024 || total > 60 * 1024 * 1024) {
				oversized = true;
				return false;
			}
			return /^(word\/|\[Content_Types\])/.test(file.name);
		},
	});
	if (oversized)
		throw new Error("The expanded DOCX file exceeds the import size limit.");
	const parse = (name: string) => {
		if (!files[name]) return undefined;
		const document = new DOMParser().parseFromString(
			strFromU8(files[name]),
			"application/xml",
		);
		if (document.getElementsByTagName("parsererror").length)
			throw new Error(`Invalid DOCX XML: ${name}`);
		return document;
	};
	const document = parse("word/document.xml");
	if (!document) throw new Error("This file has no Word document content.");
	const relationships = new Map(
		descendants(
			parse("word/_rels/document.xml.rels") ?? document,
			"Relationship",
		).map((node) => [
			node.getAttribute("Id"),
			node.getAttribute("Target") ?? "",
		]),
	);
	const warnings: string[] = [];
	if (files["word/comments.xml"])
		warnings.push(
			"Word comments were omitted. Keep the original file for its review history.",
		);
	if (Object.keys(files).some((name) => /^word\/(header|footer)/.test(name)))
		warnings.push("Page headers and footers were omitted.");
	if (
		descendants(document, "ins").length ||
		descendants(document, "del").length
	)
		warnings.push(
			"Word tracked changes were imported as the current visible text.",
		);
	const numbering = parse("word/numbering.xml");
	const numberKinds = new Map<string, string>();
	if (numbering)
		for (const num of descendants(numbering, "num")) {
			const abstractId = attr(child(num, "abstractNumId"), "val");
			const abstract = descendants(numbering, "abstractNum").find(
				(node) => attr(node, "abstractNumId") === abstractId,
			);
			numberKinds.set(
				attr(num, "numId"),
				abstract && attr(descendants(abstract, "numFmt")[0], "val") === "bullet"
					? "disc"
					: "decimal",
			);
		}
	const inline = (element: Element): DocxDescendant[] => {
		if (element.localName === "del") return [];
		if (element.localName === "hyperlink") {
			const url =
				relationships.get(
					element.getAttributeNS(R, "id") ?? element.getAttribute("r:id"),
				) || (attr(element, "anchor") ? `#${attr(element, "anchor")}` : "");
			const children = [...element.children].flatMap(inline);
			return url && isPublicationUrl(url, "a")
				? [
						{
							type: "a",
							url,
							children: children.length ? children : [{ text: url }],
						},
					]
				: children;
		}
		if (element.localName !== "r") return [...element.children].flatMap(inline);
		const properties = child(element, "rPr");
		const mark: DocxMarks = {};
		for (const [tag, key] of [
			["b", "bold"],
			["i", "italic"],
			["u", "underline"],
			["strike", "strikethrough"],
		] as const) {
			const value = properties && child(properties, tag);
			if (value && !["0", "false", "none"].includes(attr(value, "val")))
				mark[key] = true;
		}
		const vertical = properties && attr(child(properties, "vertAlign"), "val");
		if (vertical === "superscript" || vertical === "subscript")
			mark[vertical] = true;
		return [...element.children].flatMap((item): DocxDescendant[] => {
			if (item.localName === "t")
				return [{ ...mark, text: item.textContent ?? "" }];
			if (item.localName === "tab") return [{ ...mark, text: "\t" }];
			if (item.localName === "br" || item.localName === "cr")
				return [{ ...mark, text: "\n" }];
			if (item.localName === "footnoteReference")
				return [
					{
						type: "footnoteReference",
						identifier: attr(item, "id"),
						children: [{ text: "" }],
					},
				];
			return [];
		});
	};
	const paragraph = (element: Element): DocxElement[] => {
		const properties = child(element, "pPr");
		const style = properties && attr(child(properties, "pStyle"), "val");
		const heading = /^Heading([1-6])$/i.exec(style || "");
		const children = [...element.children]
			.filter((item) => item.localName !== "pPr")
			.flatMap(inline);
		const node: DocxElement = {
			type: heading ? `h${heading[1]}` : "p",
			children: children.length ? children : [{ text: "" }],
		};
		const list = properties && child(properties, "numPr");
		if (list) {
			node.indent = Number(attr(child(list, "ilvl"), "val") || 0) + 1;
			node.listStyleType =
				numberKinds.get(attr(child(list, "numId"), "val")) ?? "decimal";
		}
		const alignment = properties && attr(child(properties, "jc"), "val");
		if (alignment) node.align = alignment === "both" ? "justify" : alignment;
		const images = descendants(element, "blip").flatMap((blip) => {
			const target = relationships.get(
				blip.getAttributeNS(R, "embed") ?? blip.getAttribute("r:embed"),
			);
			const filename = target?.startsWith("/")
				? target.slice(1)
				: `word/${target}`;
			const content = files[filename];
			const ext = filename.split(".").pop()?.toLowerCase();
			const mime = (
				{
					png: "image/png",
					jpg: "image/jpeg",
					jpeg: "image/jpeg",
					gif: "image/gif",
					webp: "image/webp",
				} as Record<string, string>
			)[ext ?? ""];
			if (!content || !mime) {
				warnings.push("An unsupported or linked image was omitted.");
				return [];
			}
			let drawing: Element | null = blip;
			while (drawing && drawing.localName !== "drawing")
				drawing = drawing.parentElement;
			const alt =
				descendants(drawing ?? element, "docPr")[0]?.getAttribute("descr") ??
				"";
			return [
				{
					type: "img",
					url: `data:${mime};base64,${bytesToBase64(content)}`,
					alt,
					children: [{ text: "" }],
				},
			];
		});
		return [
			...(children.length || images.length === 0 ? [node] : []),
			...images,
		];
	};
	const blocks = (element: Element): DocxElement[] =>
		[...element.children].flatMap((item): DocxElement[] => {
			if (item.localName === "p") return paragraph(item);
			if (item.localName === "tbl")
				return [
					{
						type: "table",
						children: [...item.children]
							.filter((row) => row.localName === "tr")
							.map((row) => ({
								type: "tr",
								children: [...row.children]
									.filter((cell) => cell.localName === "tc")
									.map((cell) => ({
										type: "td",
										children: blocks(cell),
										colSpan: Number(
											attr(descendants(cell, "gridSpan")[0], "val") || 1,
										),
									})),
							})),
					},
				];
			return [];
		});
	const body = descendants(document, "body")[0];
	const value = body ? blocks(body) : [];
	const footnotes = parse("word/footnotes.xml");
	if (footnotes) {
		const footnoteRelationships = parse("word/_rels/footnotes.xml.rels");
		relationships.clear();
		if (footnoteRelationships)
			for (const item of descendants(footnoteRelationships, "Relationship")) {
				relationships.set(
					item.getAttribute("Id"),
					item.getAttribute("Target") ?? "",
				);
			}
		for (const definition of descendants(footnotes, "footnote")) {
			if (Number(attr(definition, "id")) <= 0) continue;
			value.push({
				type: "footnoteDefinition",
				identifier: attr(definition, "id"),
				children: blocks(definition),
			});
		}
	}
	if (descendants(document, "txbxContent").length)
		warnings.push(
			"Text boxes were omitted. Check the imported document against the original.",
		);
	return {
		value: value.length ? value : [{ type: "p", children: [{ text: "" }] }],
		warnings: [...new Set(warnings)],
	};
}

/** Exports the editable document body as WordprocessingML with embedded image bytes. */
export function exportDocx(value: Value): Uint8Array {
	const files: Record<string, Uint8Array> = {};
	const relationships: string[] = [];
	const types = new Map<string, string>();
	const footnoteIds = new Map<string, number>();
	const footnoteId = (identifier: unknown) => {
		const key = String(identifier ?? "1");
		const existingId = footnoteIds.get(key);
		if (existingId !== undefined) return existingId;
		const id = footnoteIds.size + 1;
		footnoteIds.set(key, id);
		return id;
	};
	let relationshipId = 0;
	const relate = (type: string, target: string, external = false) => {
		const id = `rId${++relationshipId}`;
		relationships.push(
			`<Relationship Id="${id}" Type="${R}/${type}" Target="${xml(target)}"${external ? ' TargetMode="External"' : ""}/>`,
		);
		return id;
	};
	const run = (node: DocxNode): string => {
		if (node.type === "footnoteReference")
			return `<w:r><w:footnoteReference w:id="${footnoteId(node.identifier)}"/></w:r>`;
		if (node.type === "a") {
			const text = (node.children ?? []).map(run).join("");
			const url = node.url ?? "";
			if (!isPublicationUrl(url, "a")) return text;
			return `<w:hyperlink r:id="${relate("hyperlink", url, true)}">${text}</w:hyperlink>`;
		}
		if (typeof node.text !== "string")
			return (node.children ?? []).map(run).join("");
		const marks = [
			node.bold ? "<w:b/>" : "",
			node.italic ? "<w:i/>" : "",
			node.underline ? '<w:u w:val="single"/>' : "",
			node.strikethrough ? "<w:strike/>" : "",
			node.superscript ? '<w:vertAlign w:val="superscript"/>' : "",
			node.subscript ? '<w:vertAlign w:val="subscript"/>' : "",
		].join("");
		return `<w:r>${marks ? `<w:rPr>${marks}</w:rPr>` : ""}${node.text
			.split("\n")
			.map((line: string) => `<w:t xml:space="preserve">${xml(line)}</w:t>`)
			.join("<w:br/>")}</w:r>`;
	};
	const image = (node: DocxNode): string => {
		const match = /^data:(image\/(?:png|jpeg|gif|webp));base64,(.+)$/s.exec(
			node.url ?? "",
		);
		if (!match)
			return paragraph({
				type: "p",
				children: [
					{
						type: "a",
						url: node.url,
						children: [{ text: node.alt || "Image" }],
					},
				],
			});
		const extension =
			match[1].split("/")[1] === "jpeg" ? "jpg" : match[1].split("/")[1];
		const filename = `image${relationshipId + 1}.${extension}`;
		files[`word/media/${filename}`] = Uint8Array.from(
			atob(match[2]),
			(character) => character.charCodeAt(0),
		);
		types.set(extension, match[1]);
		const id = relate("image", `media/${filename}`);
		const [pixelWidth, pixelHeight] = imageSize(
			files[`word/media/${filename}`],
			match[1],
		);
		const ratio =
			pixelWidth > 0 && pixelHeight > 0 ? pixelHeight / pixelWidth : 0.75;
		const width = Math.round(Math.min(5486400, 7315200 / ratio));
		const height = Math.round(width * ratio);
		const drawing = `<w:p><w:r><w:drawing><wp:inline><wp:extent cx="${width}" cy="${height}"/><wp:docPr id="${relationshipId}" name="${filename}" descr="${xml(node.alt ?? "")}"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="0" name="${filename}"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="${id}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="${width}" cy="${height}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>`;
		const caption = [
			...(node.caption ?? []).map(plain),
			node.credit,
			node.license,
		]
			.filter(Boolean)
			.join(" · ");
		return (
			drawing + (caption ? paragraph({ children: [{ text: caption }] }) : "")
		);
	};
	const paragraph = (node: DocxNode): string => {
		const heading = /^h([1-6])$/.exec(node.type ?? "");
		const properties = [
			heading ? `<w:pStyle w:val="Heading${heading[1]}"/>` : "",
			node.align
				? `<w:jc w:val="${xml(node.align === "justify" ? "both" : node.align)}"/>`
				: "",
			node.listStyleType
				? `<w:numPr><w:ilvl w:val="${Math.max(0, (node.indent ?? 1) - 1)}"/><w:numId w:val="${node.listStyleType === "decimal" ? 2 : 1}"/></w:numPr>`
				: "",
		].join("");
		return `<w:p>${properties ? `<w:pPr>${properties}</w:pPr>` : ""}${(node.children ?? []).map(run).join("")}</w:p>`;
	};
	const block = (node: DocxNode): string => {
		if (node.type === "footnoteDefinition") return "";
		if (node.type === "img") return image(node);
		if (node.type === "table")
			return `<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr>${(node.children ?? []).map((row) => `<w:tr>${(row.children ?? []).map((cell) => `<w:tc><w:tcPr>${(cell.colSpan ?? 1) > 1 ? `<w:gridSpan w:val="${cell.colSpan}"/>` : ""}</w:tcPr>${(cell.children ?? []).map(block).join("") || "<w:p/>"}</w:tc>`).join("")}</w:tr>`).join("")}</w:tbl>`;
		if (["file", "video", "audio", "media_embed"].includes(node.type ?? ""))
			return paragraph({
				children: [
					{
						type: "a",
						url: node.url,
						children: [
							{
								text:
									node.name ||
									node.alt ||
									(node.url?.startsWith("data:")
										? "Media attachment"
										: node.url) ||
									"Media",
							},
						],
					},
				],
			});
		if (
			(node.children ?? []).some(
				(child) =>
					child.type &&
					!["a", "footnoteReference", "mention", "inline_equation"].includes(
						child.type,
					),
			)
		)
			return (node.children ?? []).map(block).join("");
		return paragraph(node);
	};
	const body = value.map(block).join("");
	const footnotes = value.filter((node) => node.type === "footnoteDefinition");
	if (footnotes.length) {
		relate("footnotes", "footnotes.xml");
		files["word/footnotes.xml"] = strToU8(
			`<?xml version="1.0" encoding="UTF-8"?><w:footnotes xmlns:w="${W}" xmlns:r="${R}">${footnotes.map((node) => `<w:footnote w:id="${footnoteId(node.identifier)}">${(node.children ?? []).map(block).join("").replace("<w:p>", "<w:p><w:r><w:footnoteRef/></w:r>")}</w:footnote>`).join("")}</w:footnotes>`,
		);
	}
	relate("numbering", "numbering.xml");
	files["word/numbering.xml"] = strToU8(
		`<?xml version="1.0" encoding="UTF-8"?><w:numbering xmlns:w="${W}">${["bullet", "decimal"].map((kind, index) => `<w:abstractNum w:abstractNumId="${index}">${Array.from({ length: 9 }, (_, level) => `<w:lvl w:ilvl="${level}"><w:start w:val="1"/><w:numFmt w:val="${kind}"/><w:lvlText w:val="${kind === "bullet" ? "•" : `%${level + 1}.`}"/></w:lvl>`).join("")}</w:abstractNum><w:num w:numId="${index + 1}"><w:abstractNumId w:val="${index}"/></w:num>`).join("")}</w:numbering>`,
	);
	relate("styles", "styles.xml");
	files["word/styles.xml"] = strToU8(
		`<?xml version="1.0" encoding="UTF-8"?><w:styles xmlns:w="${W}">${Array.from({ length: 6 }, (_, index) => `<w:style w:type="paragraph" w:styleId="Heading${index + 1}"><w:name w:val="heading ${index + 1}"/><w:pPr><w:outlineLvl w:val="${index}"/></w:pPr><w:rPr><w:b/><w:sz w:val="${48 - index * 4}"/></w:rPr></w:style>`).join("")}</w:styles>`,
	);
	files["word/document.xml"] = strToU8(
		`<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="${W}" xmlns:r="${R}" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><w:body>${body}<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1134" w:right="1134" w:bottom="1134" w:left="1134"/></w:sectPr></w:body></w:document>`,
	);
	files["word/_rels/document.xml.rels"] = strToU8(
		`<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">${relationships.join("")}</Relationships>`,
	);
	if (footnotes.length)
		files["word/_rels/footnotes.xml.rels"] =
			files["word/_rels/document.xml.rels"];
	files["_rels/.rels"] = strToU8(
		`<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="${R}/officeDocument" Target="word/document.xml"/></Relationships>`,
	);
	files["[Content_Types].xml"] = strToU8(
		`<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>${[...types].map(([extension, mime]) => `<Default Extension="${extension}" ContentType="${mime}"/>`).join("")}<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/>${footnotes.length ? '<Override PartName="/word/footnotes.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml"/>' : ""}</Types>`,
	);
	return zipSync(files, { level: 6 });
}
