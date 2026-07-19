import { describe, expect, test } from "bun:test";
import {
	limitUploadBatch,
	mergeSuccessfulUploadBatch,
} from "./upload-input-state";

describe("upload input state", () => {
	test("never uploads more files than the remaining capacity", () => {
		expect(limitUploadBatch(["a", "b"], 0, true, 1)).toEqual(["a"]);
		expect(limitUploadBatch(["b", "c"], 1, true, 2)).toEqual(["b"]);
	});

	test("preserves prior batches and commits only successful uploads", () => {
		const current = [{ name: "a", url: "signed://a" }];
		const results = [
			{ name: "b", url: "signed://b" },
			{ name: "c", url: undefined },
		];

		expect(
			mergeSuccessfulUploadBatch(current, results, true, 3, (file) =>
				Boolean(file.url),
			),
		).toEqual([current[0], results[0]]);
	});

	test("keeps the previous single value when replacement fails", () => {
		const current = [{ name: "a", url: "signed://a" }];
		const failed = [{ name: "b", url: undefined }];

		expect(
			mergeSuccessfulUploadBatch(current, failed, false, 1, (file) =>
				Boolean(file.url),
			),
		).toEqual(current);
	});
});
