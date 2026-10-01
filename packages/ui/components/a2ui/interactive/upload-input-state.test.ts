import { describe, expect, test } from "bun:test";
import {
	limitUploadBatch,
	mergeSuccessfulUploadBatch,
	settleUploadBatch,
} from "./upload-input-state";

interface UploadResult {
	name: string;
	url?: string;
}

describe("upload input state", () => {
	test("never uploads more files than the remaining capacity", () => {
		expect(limitUploadBatch(["a", "b"], 0, true, 1)).toEqual(["a"]);
		expect(limitUploadBatch(["b", "c"], 1, true, 2)).toEqual(["b"]);
	});

	test("preserves prior batches and commits only successful uploads", () => {
		const current: UploadResult[] = [{ name: "a", url: "signed://a" }];
		const results: UploadResult[] = [
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
		const current: UploadResult[] = [{ name: "a", url: "signed://a" }];
		const failed: UploadResult[] = [{ name: "b", url: undefined }];

		expect(
			mergeSuccessfulUploadBatch(current, failed, false, 1, (file) =>
				Boolean(file.url),
			),
		).toEqual(current);
	});

	test("a failed single upload stays visible without being committed", () => {
		const failed: UploadResult[] = [{ name: "b", url: undefined }];
		const uploaded = (file: UploadResult) => Boolean(file.url);

		expect(settleUploadBatch([], failed, false, 1, uploaded)).toEqual({
			committed: [],
			display: failed,
		});

		const current: UploadResult[] = [{ name: "a", url: "signed://a" }];
		expect(settleUploadBatch(current, failed, false, 1, uploaded)).toEqual({
			committed: current,
			display: [...current, ...failed],
		});
	});

	test("failures in a multiple batch follow the committed files", () => {
		const current: UploadResult[] = [{ name: "a", url: "signed://a" }];
		const results: UploadResult[] = [
			{ name: "b", url: undefined },
			{ name: "c", url: "signed://c" },
		];

		expect(
			settleUploadBatch(current, results, true, 5, (file) => Boolean(file.url)),
		).toEqual({
			committed: [current[0], results[1]],
			display: [current[0], results[1], results[0]],
		});
	});
});
