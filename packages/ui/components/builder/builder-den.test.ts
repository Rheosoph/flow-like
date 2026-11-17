import { describe, expect, it } from "bun:test";
import {
	builderComponentTarget,
	builderSurfaceTarget,
	componentDisplayName,
} from "./builder-den";

describe("builder den targets", () => {
	it("names a page and the components on it", () => {
		const scope = { appId: "a", pageId: "p", surfaceLabel: "Home" };
		expect(builderSurfaceTarget(scope)).toEqual({
			kind: "page",
			app_id: "a",
			id: "p",
			label: "Home",
		});
		expect(builderComponentTarget(scope, "c1", "button")).toEqual({
			kind: "component",
			app_id: "a",
			ids: ["c1"],
			page_id: "p",
			label: "button",
		});
	});

	it("names a widget and the components in it", () => {
		const scope = { appId: "a", widgetId: "w", surfaceLabel: "Card" };
		expect(builderSurfaceTarget(scope)).toMatchObject({
			kind: "widget",
			id: "w",
		});
		expect(builderComponentTarget(scope, "c1", "text")).toMatchObject({
			widget_id: "w",
		});
	});

	it("labels components the way the hierarchy does", () => {
		const none = () => undefined;
		expect(componentDisplayName({ type: "row" }, none)).toBe("row");
		expect(
			componentDisplayName(
				{ type: "widgetInstance", instanceId: "i" },
				() => "Hero",
			),
		).toBe("Hero");
		expect(componentDisplayName({ type: "widgetInstance" }, none)).toBe(
			"Widget",
		);
		expect(componentDisplayName({ type: "microWidgetInstance" }, none)).toBe(
			"Package Widget",
		);
	});
});
