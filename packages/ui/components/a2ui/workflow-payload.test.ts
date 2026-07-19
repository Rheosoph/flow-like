import { describe, expect, test } from "bun:test";
import { compactWorkflowPayload } from "./workflow-payload";

describe("compactWorkflowPayload", () => {
	test("preserves null values and array positions", () => {
		expect(
			compactWorkflowPayload({
				cleared: null,
				values: [1, null, undefined, 4],
				omitted: undefined,
			}),
		).toEqual({
			cleared: null,
			values: [1, null, null, 4],
		});
	});

	test("removes transient upload previews", () => {
		expect(
			compactWorkflowPayload({
				file: {
					name: "report.pdf",
					size: 42,
					type: "application/pdf",
					dataUrl: "data:application/pdf;base64,large-preview",
					backendUrl: "signed://report",
				},
			}),
		).toEqual({
			file: {
				name: "report.pdf",
				size: 42,
				type: "application/pdf",
				backendUrl: "signed://report",
			},
		});
	});

	test("keeps unrelated dataUrl fields", () => {
		expect(
			compactWorkflowPayload({
				custom: { dataUrl: "data:text/plain,meaningful" },
			}),
		).toEqual({ custom: { dataUrl: "data:text/plain,meaningful" } });
	});
});
