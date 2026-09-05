import { describe, expect, test } from "bun:test";
import {
	MAX_HOME_WIDGETS,
	moveHomeWidget,
	normalizeHomeLayout,
	resolveHomeLayout,
} from "./home-layout";
import type { IHomeDefaults, IHomeLayout } from "./types";

function page(prefix: string, count = 1): IHomeLayout {
	return {
		version: 1,
		widgets: Array.from({ length: count }, (_, index) => ({
			id: `${prefix}-${index}`,
			type: "data",
			title: `Widget ${index}`,
			size: { columns: 4, rows: 3 },
			appearance: { variant: "card", accent: "blue" },
			config: {
				appId: "app",
				filters: [{ field: "owner", valueType: "viewer" }],
			},
		})),
	};
}

describe("home layout inheritance and history invariants", () => {
	test("reset follows a newer published default while a personal layout remains independent", () => {
		const personal = page("personal");
		const defaults: IHomeDefaults = {
			main: { id: "main", revision: "1", layout: page("main-v1") },
			profile: { id: "profile", revision: "1", layout: page("profile-v1") },
		};
		const updated: IHomeDefaults = {
			...defaults,
			profile: { id: "profile", revision: "2", layout: page("profile-v2") },
		};
		expect(
			resolveHomeLayout(personal, updated, page("bundled")).layout.widgets[0]
				.id,
		).toBe("personal-0");
		expect(
			resolveHomeLayout(null, defaults, page("bundled")).layout.widgets[0].id,
		).toBe("profile-v1-0");
		expect(
			resolveHomeLayout(null, updated, page("bundled")).layout.widgets[0].id,
		).toBe("profile-v2-0");
		expect(personal.widgets[0].id).toBe("personal-0");
	});
	test("an intentionally empty profile default overrides main but a malformed default falls through", () => {
		const defaults: IHomeDefaults = {
			main: { id: "main", revision: "1", layout: page("main") },
			profile: { id: "profile", revision: "1", layout: page("blank", 0) },
		};
		expect(resolveHomeLayout(null, defaults, page("bundled"))).toMatchObject({
			source: "profile",
			layout: { widgets: [] },
		});
		const duplicate = page("invalid");
		duplicate.widgets.push(duplicate.widgets[0]);
		const result = resolveHomeLayout(
			{ version: 99, widgets: [] },
			{
				...defaults,
				profile: { id: "profile", revision: "2", layout: duplicate },
			},
			page("bundled"),
		);
		expect(result.source).toBe("main");
		expect(result.layout.widgets[0].id).toBe("main-0");
	});
	test("every move preserves configuration and frozen previous snapshots for undo", () => {
		const original = page("widget", 6);
		const snapshot = JSON.stringify(original);
		for (const widget of original.widgets) Object.freeze(widget);
		Object.freeze(original.widgets);
		Object.freeze(original);
		for (const from of original.widgets)
			for (const to of original.widgets) {
				const changed = moveHomeWidget(original, from.id, to.id);
				expect(new Set(changed.widgets.map((widget) => widget.id)).size).toBe(
					6,
				);
				expect(changed.widgets.map((widget) => widget.id).sort()).toEqual(
					original.widgets.map((widget) => widget.id).sort(),
				);
				for (const widget of changed.widgets)
					expect(widget.config).toBe(
						original.widgets.find((item) => item.id === widget.id)?.config,
					);
				expect(JSON.stringify(original)).toBe(snapshot);
			}
	});
	test("unknown or unchanged drag targets do not create a new history snapshot", () => {
		const original = page("widget", 3);
		expect(moveHomeWidget(original, "missing", "widget-1")).toBe(original);
		expect(moveHomeWidget(original, "widget-1", "missing")).toBe(original);
		expect(moveHomeWidget(original, "widget-1", "widget-1")).toBe(original);
	});
	test("oversized layouts fail as a whole while nonfinite sizes fall back predictably", () => {
		expect(
			normalizeHomeLayout(page("max", MAX_HOME_WIDGETS))?.widgets,
		).toHaveLength(MAX_HOME_WIDGETS);
		expect(
			normalizeHomeLayout(page("too-large", MAX_HOME_WIDGETS + 1)),
		).toBeNull();
		const original = page("size");
		original.widgets[0].size = {
			columns: Number.NaN,
			rows: Number.POSITIVE_INFINITY,
		};
		expect(normalizeHomeLayout(original)?.widgets[0].size).toEqual({
			columns: 6,
			rows: 3,
		});
	});
});
