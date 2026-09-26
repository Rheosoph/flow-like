import { describe, expect, test } from "bun:test";

import {
	forwardedFilesManifest,
	mergeAttachments,
	resolveForwardFiles,
} from "./forwarded-attachments";

const sites = {
	name: "Sites.geojson",
	type: "application/geo+json",
	size: 2_621_440,
	url: "asset://localhost/cache/chat/abc.geojson?filename=Sites.geojson",
};
const notes = {
	name: "notes.txt",
	type: "text/plain",
	size: 512,
	url: "https://files.example/tmp/notes-1.txt?sig=1",
};

describe("resolveForwardFiles", () => {
	test("forwards nothing unless files are named", () => {
		expect(resolveForwardFiles([sites, notes], undefined)).toEqual({
			status: "ok",
			files: [],
		});
		expect(resolveForwardFiles([sites, notes], "Sites.geojson")).toEqual({
			status: "ok",
			files: [],
		});
	});

	test("matches names and URL basenames case-insensitively, once per file", () => {
		expect(
			resolveForwardFiles(
				[sites, notes],
				[" sites.GEOJSON ", "notes-1.txt", "Sites.geojson"],
			),
		).toEqual({ status: "ok", files: [sites, notes] });
	});

	test("reports unknown and ambiguous names with the call_app_chat error shapes", () => {
		expect(resolveForwardFiles([sites], ["missing.csv"])).toEqual({
			status: "error",
			code: "forward_file_not_found",
			message:
				"Attachment 'missing.csv' does not belong to this tool call's user turn.",
		});
		const twin = {
			...sites,
			url: "asset://localhost/cache/chat/other.geojson",
		};
		expect(resolveForwardFiles([sites, twin], ["sites.geojson"])).toMatchObject(
			{
				status: "error",
				code: "forward_file_name_ambiguous",
			},
		);
	});
});

describe("forwarded file bookkeeping", () => {
	test("merges files by URL without duplicates", () => {
		expect(mergeAttachments([sites], [notes, { ...sites }])).toEqual([
			sites,
			notes,
		]);
	});

	test("lists forwarded files like the attachment manifest", () => {
		expect(
			forwardedFilesManifest([sites, notes, "https://x.example/a.json"]),
		).toBe(
			[
				"FILES FORWARDED FOR IMPORT (use database_tool import_geojson with file_name):",
				"- Sites.geojson (application/geo+json, 2.5 MB)",
				"- notes.txt (text/plain, 512 B)",
				"- a.json",
			].join("\n"),
		);
	});
});
