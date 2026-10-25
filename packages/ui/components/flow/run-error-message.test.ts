import { expect, test } from "bun:test";
import { runErrorToShow } from "./run-error-message";

test("a stored failure reason is shown trimmed", () => {
	expect(runErrorToShow("  executor crashed\n")).toBe("executor crashed");
	expect(runErrorToShow("executor crashed", "node timed out")).toBe(
		"executor crashed",
	);
});

test("nothing is shown without a reason", () => {
	expect(runErrorToShow(undefined)).toBeUndefined();
	expect(runErrorToShow(null, "node timed out")).toBeUndefined();
	expect(runErrorToShow("   ")).toBeUndefined();
});

test("a reason the first logged error already states is not repeated", () => {
	expect(
		runErrorToShow(
			"payload does not match the event schema",
			"payload does not match the event schema ",
		),
	).toBeUndefined();
});
