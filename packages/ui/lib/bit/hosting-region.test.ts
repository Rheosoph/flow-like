import { expect, test } from "bun:test";
import {
	bitHostingRegion,
	isEuHosted,
	parseHostingRegion,
	withHostingRegion,
} from "./hosting-region";

test("only declared, known regions count", () => {
	expect(parseHostingRegion("eu")).toBe("eu");
	for (const value of ["EU", " eu", "us", "", null, undefined, true, ["eu"]])
		expect(parseHostingRegion(value)).toBeUndefined();
	expect(isEuHosted({ parameters: { hosting_region: "eu" } })).toBe(true);
	expect(isEuHosted({ parameters: { hosting_region: "us" } })).toBe(false);
	expect(isEuHosted({ parameters: undefined })).toBe(false);
	expect(bitHostingRegion({ parameters: {} })).toBeUndefined();
});

test("setting a region keeps other parameters and clearing removes the key", () => {
	const parameters = { context_length: 4096, hosting_region: "us" };
	expect(withHostingRegion(parameters, "eu")).toEqual({
		context_length: 4096,
		hosting_region: "eu",
	});
	const cleared = withHostingRegion(parameters, undefined);
	expect(cleared).toEqual({ context_length: 4096 });
	expect(Object.hasOwn(cleared, "hosting_region")).toBe(false);
});
