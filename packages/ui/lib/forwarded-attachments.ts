import type { IAttachment } from "../components/interfaces/chat-default/chat-db";

export type ForwardFilesResolution =
	| { status: "ok"; files: IAttachment[] }
	| {
			status: "error";
			code: "forward_file_not_found" | "forward_file_name_ambiguous";
			message: string;
	  };

function attachmentUrl(file: IAttachment): string {
	return typeof file === "string" ? file : file.url;
}

/** Lower-cased names a file answers to: its name, its URL and the URL's basename. */
export function attachmentMatchLabels(file: IAttachment): string[] {
	const raw =
		typeof file === "string"
			? [file]
			: [file.url, file.name].filter((value): value is string =>
					Boolean(value),
				);
	const withBasenames = raw.flatMap((value) => {
		const basename = value.split("?")[0]?.split("/").pop();
		return basename && basename !== value ? [value, basename] : [value];
	});
	return withBasenames.map((value) => value.toLowerCase());
}

export function attachmentDisplayName(file: IAttachment): string {
	const url = attachmentUrl(file);
	const name = typeof file === "string" ? undefined : file.name?.trim();
	return name || url.split("?")[0]?.split("/").pop() || url;
}

/**
 * Select the turn's files named by a model-authored `forward_files` list. Fail-closed: anything but
 * a list of names forwards nothing, and a name must match exactly one file of the turn.
 */
export function resolveForwardFiles(
	turnFiles: readonly IAttachment[],
	forwardFiles: unknown,
): ForwardFilesResolution {
	const requestedNames = Array.isArray(forwardFiles)
		? forwardFiles
				.filter((value): value is string => typeof value === "string")
				.map((value) => value.trim().toLowerCase())
				.filter((value) => value.length > 0)
		: [];
	const files: IAttachment[] = [];
	const selectedIndexes = new Set<number>();
	for (const requestedName of new Set(requestedNames)) {
		const matches = turnFiles
			.map((file, index) => ({ file, index }))
			.filter(({ file }) =>
				attachmentMatchLabels(file).includes(requestedName),
			);
		if (matches.length !== 1) {
			return matches.length === 0
				? {
						status: "error",
						code: "forward_file_not_found",
						message: `Attachment '${requestedName}' does not belong to this tool call's user turn.`,
					}
				: {
						status: "error",
						code: "forward_file_name_ambiguous",
						message: `Attachment name '${requestedName}' matches more than one file in this user turn. Ask the user to rename or reattach the intended file; no file was forwarded.`,
					};
		}
		const [{ file, index }] = matches;
		if (!selectedIndexes.has(index)) {
			selectedIndexes.add(index);
			files.push(file);
		}
	}
	return { status: "ok", files };
}

export function mergeAttachments(
	existing: readonly IAttachment[],
	added: readonly IAttachment[],
): IAttachment[] {
	const urls = new Set(existing.map(attachmentUrl));
	const merged = [...existing];
	for (const file of added) {
		const url = attachmentUrl(file);
		if (urls.has(url)) continue;
		urls.add(url);
		merged.push(file);
	}
	return merged;
}

/** Same units as the Rust FILES ATTACHED THIS TURN manifest, so both lists read alike. */
function formatAttachmentSize(bytes: number): string {
	const units = [
		["GB", 1024 ** 3],
		["MB", 1024 ** 2],
		["KB", 1024],
	] as const;
	for (const [unit, size] of units) {
		if (bytes >= size) return `${(bytes / size).toFixed(1)} ${unit}`;
	}
	return `${bytes} B`;
}

export function forwardedFilesManifest(files: readonly IAttachment[]): string {
	const lines = files.map((file) => {
		const type =
			typeof file === "string" ? undefined : file.type?.trim() || undefined;
		const size =
			typeof file !== "string" && typeof file.size === "number"
				? formatAttachmentSize(file.size)
				: undefined;
		const meta = [type, size].filter(Boolean).join(", ");
		return `- ${attachmentDisplayName(file)}${meta ? ` (${meta})` : ""}`;
	});
	return [
		"FILES FORWARDED FOR IMPORT (use database_tool import_geojson with file_name):",
		...lines,
	].join("\n");
}
