import { describe, expect, test } from "bun:test";
import { pdfViewerFragment } from "../../ui/file-previewer";
import type { ProcessedAttachment } from "./attachment";
import { canPreviewFile } from "./attachment-dialog";

function attachment(
	overrides: Partial<ProcessedAttachment> & { url: string },
): ProcessedAttachment {
	const name = overrides.name ?? "file";
	return {
		name,
		displayName: name,
		ext: name.split(".").slice(1).pop() ?? "",
		type: "other",
		isDataUrl: false,
		...overrides,
	};
}

describe("canPreviewFile", () => {
	test("a pdf stays previewable when its url carries no extension", () => {
		expect(
			canPreviewFile(
				attachment({
					url: "blob:https://app.test/9f6c1b2e-0000-4000-8000-000000000000",
					name: "contract.pdf",
					type: "pdf",
				}),
			),
		).toBe(true);
	});

	test("an extension-less signed url falls back to the attachment name", () => {
		expect(
			canPreviewFile(
				attachment({
					url: "https://cdn.test/objects/abc123?sig=xyz",
					name: "notes.md",
					type: "document",
				}),
			),
		).toBe(true);
	});

	test("a file nothing can render still routes to the download", () => {
		expect(
			canPreviewFile(
				attachment({
					url: "https://cdn.test/report.docx",
					name: "report.docx",
					type: "document",
				}),
			),
		).toBe(false);
	});
});

describe("pdfViewerFragment", () => {
	// Chrome truncates the fragment at the second `#`, so a second one silently
	// drops every parameter behind it.
	test("keeps every parameter behind a single hash", () => {
		expect(pdfViewerFragment(3)).toBe("#page=3&toolbar=1&view=FitH");
		expect(pdfViewerFragment()).toBe("#toolbar=1&view=FitH");
		expect(pdfViewerFragment(3).indexOf("#")).toBe(0);
		expect(pdfViewerFragment(3).lastIndexOf("#")).toBe(0);
	});

	test("ignores a page number the viewer cannot honour", () => {
		expect(pdfViewerFragment(0)).toBe("#toolbar=1&view=FitH");
		expect(pdfViewerFragment(-2)).toBe("#toolbar=1&view=FitH");
		expect(pdfViewerFragment(2.7)).toBe("#page=2&toolbar=1&view=FitH");
	});
});
