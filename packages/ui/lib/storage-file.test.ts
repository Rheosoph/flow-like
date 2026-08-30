import { describe, expect, test } from "bun:test";
import { looksLikeFileColumnName, resolveStorageFile } from "./storage-file";

const APP = "yijglxtpg0q3owkcx3aiw0nd";
const KEY = `apps/${APP}/upload/parsed/MP20150117.lowres/0/p000-img003.jpeg`;

describe("looksLikeFileColumnName", () => {
	test("accepts names anchored on a file word", () => {
		for (const name of [
			"file",
			"file_path",
			"filePath",
			"imagePath",
			"source_document",
			"thumbnail",
			"storage_key",
			"attachment",
		]) {
			expect(looksLikeFileColumnName(name)).toBe(true);
		}
	});

	test("leaves names that only contain a file word alone", () => {
		for (const name of [
			"key_value",
			"path_segments",
			"source_code",
			"created_at",
			"description",
			"",
		]) {
			expect(looksLikeFileColumnName(name)).toBe(false);
		}
	});
});

describe("resolveStorageFile", () => {
	test("reads an app storage key whatever the column is called", () => {
		expect(resolveStorageFile("anything", KEY, APP)).toEqual({
			scope: "app",
			path: "parsed/MP20150117.lowres/0/p000-img003.jpeg",
			directory: "parsed/MP20150117.lowres/0",
			fileName: "p000-img003.jpeg",
			extension: "jpeg",
		});
	});

	test("refuses a key that names another app", () => {
		expect(resolveStorageFile("file", KEY, "someotherapp")).toBeNull();
	});

	test("reads a user scoped key", () => {
		expect(
			resolveStorageFile("file", `users/sub-1/apps/${APP}/notes/a.md`, APP),
		).toEqual({
			scope: "user",
			path: "notes/a.md",
			directory: "notes",
			fileName: "a.md",
			extension: "md",
		});
	});

	test("believes a bare relative path only where the column promises one", () => {
		expect(resolveStorageFile("image_path", "parsed/a.png", APP)).toEqual({
			scope: "app",
			path: "parsed/a.png",
			directory: "parsed",
			fileName: "a.png",
			extension: "png",
		});
		expect(resolveStorageFile("description", "parsed/a.png", APP)).toBeNull();
	});

	test("rejects values that point outside app storage", () => {
		for (const value of [
			"https://example.com/a.png",
			"data:image/png;base64,AAAA",
			"/Users/felix/a.png",
			"C:\\temp\\a.png",
			"../../etc/passwd.txt",
			"apps/other/upload/a.png",
			`apps/${APP}/a.png`,
			"a folder\nwith a newline.txt",
			42,
			null,
		]) {
			expect(resolveStorageFile("file_path", value, APP)).toBeNull();
		}
	});

	test("rejects paths without a file name, which may be folders", () => {
		expect(
			resolveStorageFile("file_path", `apps/${APP}/upload/parsed`, APP),
		).toBeNull();
		expect(resolveStorageFile("file_path", "parsed/subfolder", APP)).toBeNull();
	});

	test("needs an app to resolve against", () => {
		expect(resolveStorageFile("file_path", KEY, undefined)).toBeNull();
	});
});
