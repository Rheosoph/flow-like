import { describe, expect, test } from "vitest";

import { validateComponents } from "../flowpilot/validateComponents";
import { COMPONENT_PROPS } from "./component-prop-manifest";
import {
	getRegisteredTypes,
	registerComponentType,
} from "./component-type-registry";
import type { SurfaceComponent } from "./types";

describe("pure A2UI component type registry", () => {
	test("starts with every compile-time component manifest type", () => {
		// Renderer tests register their own types into this process-wide registry.
		expect(getRegisteredTypes()).toEqual(
			expect.arrayContaining(Object.keys(COMPONENT_PROPS)),
		);
	});

	test("makes a dynamically registered type immediately available to validation", () => {
		const type = "testDynamicComponent";
		registerComponentType(type);
		registerComponentType(type);

		expect(
			getRegisteredTypes().filter((candidate) => candidate === type),
		).toHaveLength(1);
		const result = validateComponents([
			{
				id: "dynamic",
				component: { type },
			},
		] as unknown as SurfaceComponent[]);
		expect(result.components).toHaveLength(1);
		expect(result.components[0]?.component.type).toBe(type);
	});
});
