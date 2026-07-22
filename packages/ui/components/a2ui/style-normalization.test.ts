import { describe, expect, test } from "bun:test";
import {
	normalizePageForPersistence,
	normalizeStyleForPersistence,
} from "./style-normalization";
import type { SurfaceComponent } from "./types";

describe("A2UI persistence style normalization", () => {
	test("converts Rust compatibility shapes to the frontend wire shape", () => {
		expect(
			normalizeStyleForPersistence({
				background: {
					gradient: {
						gradientType: "linear",
						direction: "45deg",
						stops: [
							{ color: "red", position: 0 },
							{ color: "blue", position: 1 },
						],
					},
				},
				margin: { value: "8px 16px" },
				width: { value: "100%" },
				position: { positionType: "absolute", top: "1rem" },
				responsive: { md: { width: { value: "50%" } } },
			}),
		).toEqual({
			background: {
				gradient: {
					type: "linear",
					direction: "45deg",
					angle: 45,
					stops: [
						{ color: "red", position: 0 },
						{ color: "blue", position: 100 },
					],
				},
			},
			margin: { top: "8px", right: "16px", bottom: "8px", left: "16px" },
			width: "100%",
			position: { type: "absolute", top: "1rem" },
			responsiveOverrides: { md: { width: "50%" } },
		});
	});

	test("does not reinterpret canonical percentage stops near zero", () => {
		expect(
			normalizeStyleForPersistence({
				background: {
					gradient: {
						type: "linear",
						stops: [
							{ color: "red", position: 0 },
							{ color: "blue", position: 1 },
						],
					},
				},
			}),
		).toMatchObject({
			background: {
				gradient: {
					stops: [
						{ color: "red", position: 0 },
						{ color: "blue", position: 1 },
					],
				},
			},
		});
	});

	test("normalizes nested page, widget, and component styles without mutation", () => {
		const originalPage = {
			components: [
				{
					id: "root",
					style: { padding: { value: "1rem" } },
					component: {
						type: "widgetInstance" as const,
						style: { position: { positionType: "relative" } },
						inlineWidgetDef: {
							components: [
								{
									id: "nested",
									style: { width: { value: "20px" } },
									component: { type: "spacer" as const },
								},
							],
						},
					},
				},
			],
			content: [],
			widgetRefs: {},
		};
		const page = originalPage as unknown as {
			components: SurfaceComponent[];
			content: unknown[];
			widgetRefs: Record<string, { components: SurfaceComponent[] }>;
		};

		const normalized = normalizePageForPersistence(page);
		expect(normalized).not.toBe(page);
		expect(normalized.components[0]?.style).toEqual({
			padding: { top: "1rem", right: "1rem", bottom: "1rem", left: "1rem" },
		});
		const component = normalized.components[0]?.component as unknown as Record<
			string,
			unknown
		>;
		expect(component.style).toEqual({ position: { type: "relative" } });
		const inlineWidgetDef = component.inlineWidgetDef as Record<
			string,
			unknown
		>;
		const nestedComponents = inlineWidgetDef.components as Array<
			Record<string, unknown>
		>;
		expect(nestedComponents[0]?.style).toEqual({ width: "20px" });
		expect(originalPage.components[0]?.style).toEqual({
			padding: { value: "1rem" },
		});
	});
});
