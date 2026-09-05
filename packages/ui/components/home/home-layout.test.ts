import { describe, expect, it } from "bun:test";
import {
	homeWidgetSpan,
	moveHomeWidget,
	normalizeHomeLayout,
	resolveHomeLayout,
	responsiveHomeColumns,
} from "./home-layout";
import type { IHomeLayout } from "./types";

const layout = (id: string): IHomeLayout => ({
	version: 1,
	widgets: [
		{
			id,
			type: "information",
			size: { columns: 6, rows: 3 },
			appearance: { variant: "card", accent: "neutral" },
			config: {},
		},
	],
});
describe("profile home layouts", () => {
	it("preserves responsive row, content-only, and fixed height choices on reload", () => {
		for (const heightMode of ["auto", "content", "fixed"] as const) {
			const page = layout(heightMode);
			page.widgets[0].size = {
				columns: 4,
				rows: 3,
				heightMode,
				...(heightMode === "fixed" ? { height: 360 } : {}),
			};
			expect(
				normalizeHomeLayout(JSON.parse(JSON.stringify(page)))?.widgets[0].size,
			).toEqual(page.widgets[0].size);
		}
	});
	it("inherits latest defaults and treats an empty personal page as intentional", () => {
		const defaults = {
			main: { id: "main", revision: "2", layout: layout("main") },
			profile: { id: "work", revision: "3", layout: layout("work") },
		};
		expect(resolveHomeLayout(null, defaults, layout("fallback")).source).toBe(
			"profile",
		);
		expect(
			resolveHomeLayout(
				{ version: 1, widgets: [] },
				defaults,
				layout("fallback"),
			).layout.widgets,
		).toHaveLength(0);
		expect(
			resolveHomeLayout(
				null,
				{ ...defaults, profile: null },
				layout("fallback"),
			).layout.widgets[0].id,
		).toBe("main");
		expect(resolveHomeLayout(null, undefined, layout("fallback")).source).toBe(
			"bundled",
		);
	});
	it("rejects ambiguous identity and unknown schema versions but preserves unknown widgets", () => {
		const page = layout("one");
		expect(normalizeHomeLayout({ ...page, version: 2 })).toBeNull();
		expect(
			normalizeHomeLayout({
				...page,
				widgets: [...page.widgets, ...page.widgets],
			}),
		).toBeNull();
		expect(
			normalizeHomeLayout({
				...page,
				widgets: [{ ...page.widgets[0], type: "future-widget" }],
			})?.widgets[0].type,
		).toBe("future-widget");
	});
	it("keeps spans within the container at all supported widths", () => {
		for (const width of [320, 599, 600, 768, 1049, 1050, 1920]) {
			const cols = responsiveHomeColumns(width);
			for (let span = 1; span <= 12; span++)
				expect(homeWidgetSpan(span, cols)).toBeLessThanOrEqual(cols);
		}
		expect(homeWidgetSpan(8, 6)).toBe(3);
		expect(homeWidgetSpan(4, 6)).toBe(3);
		expect(homeWidgetSpan(12, 6)).toBe(6);
		expect(homeWidgetSpan(3, 1)).toBe(1);
	});
	it("reorders without modifying widget identity or embedded configuration", () => {
		const page = {
			...layout("a"),
			widgets: [
				...layout("a").widgets,
				...layout("b").widgets,
				...layout("c").widgets,
			],
		};
		const moved = moveHomeWidget(page, "a", "c");
		expect(moved.widgets.map((widget) => widget.id)).toEqual(["b", "c", "a"]);
		expect(moved.widgets[2]).toBe(page.widgets[0]);
		expect(page.widgets[0].id).toBe("a");
	});
});
