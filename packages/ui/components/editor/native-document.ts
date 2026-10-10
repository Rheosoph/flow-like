import {
	PLATE_JSON_PREFIX,
	type PlateDocument,
	parsePlateDocument,
	serializePlateDocument,
} from "../../lib/plate-document";

export const MAX_NATIVE_DOCUMENT_BYTES = 20 * 1024 * 1024;

/** Files are JSON; the prefix belongs only to the application's stored string. */
export function serializeNativeDocument(document: PlateDocument): string {
	return serializePlateDocument(document).slice(PLATE_JSON_PREFIX.length);
}

export async function deserializeNativeDocumentFile(
	file: Pick<File, "size" | "text">,
): Promise<PlateDocument> {
	if (file.size > MAX_NATIVE_DOCUMENT_BYTES)
		throw new Error("Editable document files must be smaller than 20 MiB.");
	const content = (await file.text()).trim();
	const document = parsePlateDocument(
		content.startsWith(PLATE_JSON_PREFIX)
			? content
			: `${PLATE_JSON_PREFIX}${content}`,
	);
	if (!document) throw new Error("Select an exported .plate.json document.");
	let count = 0;
	const validNodes = (nodes: unknown, depth = 0): boolean =>
		depth <= 64 &&
		Array.isArray(nodes) &&
		nodes.every((node) => {
			if (
				++count > 100_000 ||
				!node ||
				typeof node !== "object" ||
				Array.isArray(node)
			)
				return false;
			if ("children" in node) return validNodes(node.children, depth + 1);
			return typeof node.text === "string";
		});
	if (!validNodes(document.children))
		throw new Error(
			"The document contains invalid or excessively nested editor nodes.",
		);
	return document;
}
