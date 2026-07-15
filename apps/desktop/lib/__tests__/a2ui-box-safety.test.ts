import { DataProvider } from "@flow-like/flow-like-ui/components/a2ui/DataContext";
import { A2UIBox } from "@flow-like/flow-like-ui/components/a2ui/layout/Box";
import {
	SEMANTIC_BOX_TAGS,
	normalizeSemanticBoxTag,
} from "@flow-like/flow-like-ui/components/a2ui/semantic-box-tags";
import type {
	BoundValue,
	BoxComponent,
	DataEntry,
	SurfaceComponent,
} from "@flow-like/flow-like-ui/components/a2ui/types";
import { validateComponents } from "@flow-like/flow-like-ui/components/flowpilot/validateComponents";
import {
	type ComponentType,
	type PropsWithChildren,
	createElement,
} from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, test } from "vitest";

const TestDataProvider = DataProvider as ComponentType<
	PropsWithChildren<{ initialData: DataEntry[] }>
>;

function boxSurface(as: unknown): SurfaceComponent[] {
	return [
		{
			id: "root",
			component: { id: "root", type: "box", as } as BoxComponent,
		},
	];
}

function renderBox(as: BoundValue, initialData: DataEntry[] = []): string {
	const component: BoxComponent = {
		id: "root",
		type: "box",
		as,
		children: { explicitList: [] },
	};

	return renderToStaticMarkup(
		createElement(
			TestDataProvider,
			{ initialData },
			createElement(A2UIBox, {
				component,
				componentId: "root",
				surfaceId: "test-surface",
				renderChild: () => null,
			}),
		),
	);
}

describe("A2UI box tag safety", () => {
	test("has one shared semantic tag allowlist", () => {
		for (const tag of SEMANTIC_BOX_TAGS) {
			expect(normalizeSemanticBoxTag(tag)).toBe(tag);
		}
		expect(normalizeSemanticBoxTag("script")).toBe("div");
		expect(normalizeSemanticBoxTag("style")).toBe("div");
		expect(normalizeSemanticBoxTag(null)).toBe("div");
	});

	test("normalizes generated literal script tags during validation", () => {
		const result = validateComponents(boxSurface({ literalString: "script" }));
		const box = result.components[0].component as BoxComponent;

		expect(box.as).toEqual({ literalString: "div" });
		expect(result.warnings).toContain(
			'root: replaced unsafe box tag "script" with "div"',
		);
	});

	test("preserves safe literals and path bindings", () => {
		const safe = validateComponents(boxSurface("section"));
		const bound = validateComponents(
			boxSurface({ path: "layout.containerTag", defaultValue: "article" }),
		);

		expect((safe.components[0].component as BoxComponent).as).toEqual({
			literalString: "section",
		});
		expect((bound.components[0].component as BoxComponent).as).toEqual({
			path: "layout.containerTag",
			defaultValue: "article",
		});
		expect(safe.warnings).toEqual([]);
		expect(bound.warnings).toEqual([]);
	});

	test("guards literal and path-resolved tags again at render time", () => {
		const literalMarkup = renderBox({ literalString: "script" });
		const boundUnsafeMarkup = renderBox({ path: "layout.containerTag" }, [
			{ path: "layout.containerTag", value: "script" },
		]);
		const boundSafeMarkup = renderBox({ path: "layout.containerTag" }, [
			{ path: "layout.containerTag", value: "section" },
		]);

		expect(literalMarkup).toBe('<div class=""></div>');
		expect(boundUnsafeMarkup).toBe('<div class=""></div>');
		expect(boundSafeMarkup).toBe('<section class=""></section>');
	});
});
