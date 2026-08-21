import type { LessonAssetView } from "../../lib/learn/types";

const PLATE_JSON_PREFIX = "plate_json::";
const MARKDOWN_REF_RE = /(^|[^\w\\])@([A-Za-z_][A-Za-z0-9_-]{0,63})/g;
const MARKDOWN_H1_RE = /^\s{0,3}#\s+([^\s][^\r\n]*)\r*(?:\n|$)/;

interface PlateNode {
	readonly type?: string;
	readonly value?: string;
	readonly text?: string;
	readonly children?: ReadonlyArray<PlateNode>;
	readonly [key: string]: unknown;
}

export function lessonAssetLabel(name: string): string {
	return name
		.replace(/([a-z\d])([A-Z])/g, "$1 $2")
		.replace(/[_-]+/g, " ")
		.replace(/\s+/g, " ")
		.trim();
}

function normalizedTitle(value: string): string {
	return value
		.replace(/[`*_~]/g, "")
		.replace(/\s+/g, " ")
		.trim()
		.toLocaleLowerCase();
}

function headingText(line: string): string {
	let end = line.trimEnd().length;
	while (end > 0 && line[end - 1] === "#") end--;
	return line.slice(0, end).trimEnd() || line.slice(0, 1);
}

function plateText(node: PlateNode): string {
	if (typeof node.text === "string") return node.text;
	return node.children?.map(plateText).join("") ?? "";
}

/**
 * Course Markdown commonly starts with the same H1 as the lesson record. The
 * page already renders that title in its header, so keeping both creates a
 * distracting duplicate heading. A genuinely different opening H1 is kept.
 */
export function removeDuplicateLessonTitle(
	content: string,
	title: string,
): string {
	if (!content || !title) return content;
	const expected = normalizedTitle(title);

	if (content.startsWith(PLATE_JSON_PREFIX)) {
		try {
			const parsed = JSON.parse(content.slice(PLATE_JSON_PREFIX.length));
			if (!Array.isArray(parsed) || parsed.length === 0) return content;
			const first = parsed[0] as PlateNode;
			if (
				first.type !== "h1" ||
				normalizedTitle(plateText(first)) !== expected
			) {
				return content;
			}
			const remaining = parsed.slice(1);
			return `${PLATE_JSON_PREFIX}${JSON.stringify(
				remaining.length > 0
					? remaining
					: [{ type: "p", children: [{ text: "" }] }],
			)}`;
		} catch {
			return content;
		}
	}

	const heading = content.match(MARKDOWN_H1_RE);
	if (!heading || normalizedTitle(headingText(heading[1] ?? "")) !== expected) {
		return content;
	}
	const rest = content.slice(heading[0].length);
	return rest.trim() ? rest.replace(/^\s*\r?\n/, "") : "";
}

function nodeForAsset(asset: LessonAssetView): PlateNode {
	const label = lessonAssetLabel(asset.name);
	switch (asset.kind) {
		case "IMAGE":
			return {
				type: "img",
				url: asset.signed_url,
				alt: label,
				caption: [{ text: label }],
				children: [{ text: "" }],
			};
		case "VIDEO":
			return {
				type: "video",
				url: asset.signed_url,
				children: [{ text: "" }],
			};
		case "AUDIO":
			return {
				type: "audio",
				url: asset.signed_url,
				children: [{ text: "" }],
			};
		default:
			return {
				type: "a",
				url: asset.signed_url,
				children: [{ text: asset.name }],
			};
	}
}

function markdownForAsset(asset: LessonAssetView): string {
	const url = asset.signed_url
		.replaceAll(" ", "%20")
		.replaceAll("(", "%28")
		.replaceAll(")", "%29");
	const label = lessonAssetLabel(asset.name)
		.replaceAll("[", "\\[")
		.replaceAll("]", "\\]");
	return asset.kind === "IMAGE" ? `![${label}](${url})` : `[${label}](${url})`;
}

function walkPlateNodes(
	nodes: ReadonlyArray<PlateNode>,
	byName: Map<string, LessonAssetView>,
): PlateNode[] {
	const out: PlateNode[] = [];
	for (const node of nodes) {
		if (node && typeof node === "object") {
			// Author-inserted asset node: refresh stale signed URL while keeping
			// any user-applied width, alignment, or caption.
			const assetName = node.assetName;
			if (typeof assetName === "string") {
				const asset = byName.get(assetName);
				if (asset) {
					out.push({
						...node,
						url: asset.signed_url,
						alt: node.alt ?? lessonAssetLabel(asset.name),
					});
					continue;
				}
			}
			// Legacy mention shape — replace with a media node.
			if (node.type === "mention" && typeof node.value === "string") {
				const asset = byName.get(node.value);
				if (asset) {
					out.push(nodeForAsset(asset));
					continue;
				}
			}
		}
		if (Array.isArray(node?.children)) {
			out.push({
				...node,
				children: walkPlateNodes(node.children, byName),
			});
			continue;
		}
		out.push(node);
	}
	return out;
}

export function resolveAssetReferences(
	content: string,
	assets: ReadonlyArray<LessonAssetView>,
): string {
	if (!content || assets.length === 0) return content;
	const byName = new Map(assets.map((asset) => [asset.name, asset]));

	if (content.startsWith(PLATE_JSON_PREFIX)) {
		try {
			const parsed = JSON.parse(content.slice(PLATE_JSON_PREFIX.length));
			if (!Array.isArray(parsed)) return content;
			const transformed = walkPlateNodes(parsed as PlateNode[], byName);
			return `${PLATE_JSON_PREFIX}${JSON.stringify(transformed)}`;
		} catch {
			return content;
		}
	}

	return content.replace(MARKDOWN_REF_RE, (whole, prefix, name) => {
		const asset = byName.get(name);
		if (!asset) return whole;
		return `${prefix}${markdownForAsset(asset)}`;
	});
}
