import { describe, expect, test } from "vitest";
import { readReplyAttachment } from "./copilot-stream-steps";

describe("readReplyAttachment", () => {
	test("maps a link frame to a chip", () => {
		expect(
			readReplyAttachment({
				name: "x.pdf",
				mime: "application/pdf",
				bytes: 123,
				url: "https://a.test/x.pdf?sig=1",
			}),
		).toEqual({
			url: "https://a.test/x.pdf?sig=1",
			name: "x.pdf",
			type: "application/pdf",
			size: 123,
		});
	});

	test("keeps the storage url of a stored file", () => {
		expect(
			readReplyAttachment({
				name: "a b.pdf",
				url: "storage://apps/app1/app/a%20b.pdf",
				handle: { app_id: "app1", path: "a b.pdf" },
			}),
		).toEqual({ url: "storage://apps/app1/app/a%20b.pdf", name: "a b.pdf" });
	});

	test("ignores frames without a usable url", () => {
		expect(readReplyAttachment({ name: "x.pdf" })).toBeNull();
		expect(readReplyAttachment({ url: "javascript:alert(1)" })).toBeNull();
		expect(readReplyAttachment({ url: "" })).toBeNull();
		expect(readReplyAttachment("nope")).toBeNull();
		expect(readReplyAttachment(undefined)).toBeNull();
	});
});
