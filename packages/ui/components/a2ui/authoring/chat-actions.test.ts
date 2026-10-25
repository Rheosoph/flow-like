import { describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import type { SurfaceComponent } from "../types";
import {
	MAX_CHAT_MESSAGE_CHARS,
	authoredInputValue,
	changedInputValue,
	formatChatInputValue,
	interpolateChatMessage,
	isAllowedChatAssetSource,
	mergeChatData,
	openExternalUrl,
	parseHttpUrl,
	readDataPath,
} from "./chat-actions";

const input = (props: Record<string, unknown>): SurfaceComponent => ({
	id: "field",
	component: props as unknown as SurfaceComponent["component"],
});

describe("URL policy", () => {
	test("links accept absolute http(s) URLs only", () => {
		expect(parseHttpUrl(" https://example.com/a b ")).toBe(
			"https://example.com/a%20b",
		);
		expect(parseHttpUrl("http://example.com")).toBe("http://example.com/");
		for (const url of [
			"javascript:alert(1)",
			"/settings",
			"tauri://localhost",
			"file:///etc/passwd",
			"mailto:a@b.c",
			42,
		]) {
			expect(parseHttpUrl(url)).toBeUndefined();
		}
	});

	test("media accepts https and inline images only", () => {
		expect(isAllowedChatAssetSource("https://cdn.test/a.png")).toBe(true);
		expect(isAllowedChatAssetSource("data:image/png;base64,AAAA")).toBe(true);
		for (const source of [
			"http://cdn.test/a.png",
			"asset://localhost/a.png",
			"blob:https://x/1",
			"data:text/html,<script>",
			"apps/123/storage/a.png",
			undefined,
		]) {
			expect(isAllowedChatAssetSource(source)).toBe(false);
		}
	});
});

describe("send_message interpolation", () => {
	test("replaces known ids and leaves unknown placeholders", () => {
		const values: Record<string, string> = { servings: "4", diet: "" };
		expect(
			interpolateChatMessage(
				"Make {{ servings }} servings, {{diet}} diet, {{ghost}}",
				(id) => values[id],
			),
		).toBe("Make 4 servings,  diet, {{ghost}}");
	});

	test("caps the message length", () => {
		expect(
			interpolateChatMessage("{{x}}", () => "y".repeat(10_000)),
		).toHaveLength(MAX_CHAT_MESSAGE_CHARS);
	});

	test("formats input values", () => {
		expect(formatChatInputValue(3)).toBe("3");
		expect(formatChatInputValue(true)).toBe("true");
		expect(formatChatInputValue(["a", 2])).toBe("a, 2");
		expect(formatChatInputValue(null)).toBe("");
		expect(formatChatInputValue({ a: 1 })).toBe('{"a":1}');
	});

	test("reads change events from inputs and toggles", () => {
		expect(changedInputValue({ value: "x" })).toBe("x");
		expect(changedInputValue({ checked: false })).toBe(false);
	});

	test("falls back to the authored literal or the bound data", () => {
		const data = { form: { size: 3 } };
		expect(
			authoredInputValue(input({ value: { literalString: "a" } }), data),
		).toBe("a");
		expect(
			authoredInputValue(input({ checked: { literalBool: true } }), data),
		).toBe(true);
		expect(
			authoredInputValue(input({ value: { path: "$.form.size" } }), data),
		).toBe(3);
		expect(
			authoredInputValue(
				input({ value: { path: "/form/missing", defaultValue: "d" } }),
				data,
			),
		).toBe("d");
		expect(authoredInputValue(undefined, data)).toBeUndefined();
	});
});

describe("data helpers", () => {
	test("readDataPath understands every path form and refuses prototype keys", () => {
		const data = { a: { b: [10, 20] } };
		expect(readDataPath(data, "$.a.b.1")).toBe(20);
		expect(readDataPath(data, "/a/b/0")).toBe(10);
		expect(readDataPath(data, "a.missing")).toBeUndefined();
		expect(readDataPath(data, "a.__proto__")).toBeUndefined();
	});

	test("mergeChatData merges objects deeply and replaces the rest", () => {
		const merged = mergeChatData(
			{ a: { x: 1 }, list: [1] },
			JSON.parse('{"a":{"y":2},"list":[2],"__proto__":{"polluted":true}}'),
		);
		expect(merged).toEqual({ a: { x: 1, y: 2 }, list: [2] });
		expect(Object.getPrototypeOf(merged)).toBe(Object.prototype);
	});
});

describe("openExternalUrl", () => {
	test("clicks a temporary target=_blank anchor for http(s) URLs", () => {
		const window = new Window({ url: "https://app.test/chat" });
		Object.assign(window, { SyntaxError });
		const document = window.document as unknown as Document;
		const clicks: { href: string; target: string; rel: string }[] = [];
		document.addEventListener(
			"click",
			(event) => {
				const anchor = event.target as HTMLAnchorElement;
				clicks.push({
					href: anchor.href,
					target: anchor.target,
					rel: anchor.rel,
				});
				event.preventDefault();
			},
			true,
		);
		expect(openExternalUrl("https://example.com/docs", document)).toBe(true);
		expect(openExternalUrl("javascript:alert(1)", document)).toBe(false);
		expect(clicks).toEqual([
			{
				href: "https://example.com/docs",
				target: "_blank",
				rel: "noopener noreferrer",
			},
		]);
		expect(document.querySelectorAll("a")).toHaveLength(0);
		window.close();
	});
});
