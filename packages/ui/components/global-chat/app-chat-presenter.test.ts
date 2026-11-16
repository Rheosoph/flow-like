import { describe, expect, test } from "bun:test";
import {
	appChatOutcomeFields,
	summarizeAppChatFiles,
} from "./app-chat-presenter";

describe("app chat presenter outcome", () => {
	test("omits empty fields and reports pushed elements", () => {
		expect(
			appChatOutcomeFields({
				files: [],
				attachments: [],
				text: "",
				widgetCount: 0,
				surfaceCount: 0,
			}),
		).toEqual({
			attachments: undefined,
			pushed_widget_count: undefined,
			pushed_surface_count: undefined,
		});
		const attachments = [{ url: "u", name: "n", type: "t" }];
		expect(
			appChatOutcomeFields({
				files: [],
				attachments,
				text: "",
				widgetCount: 2,
				surfaceCount: 1,
			}),
		).toEqual({ attachments, pushed_widget_count: 2, pushed_surface_count: 1 });
	});

	test("summarizes string and object attachments", () => {
		expect(
			summarizeAppChatFiles([
				"https://x/y",
				{ url: "a", name: "n", type: "t" },
			] as never),
		).toEqual([{ url: "https://x/y" }, { url: "a", name: "n", type: "t" }]);
	});
});
