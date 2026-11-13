import { expect, test } from "vitest";
import { failureLabel } from "./admin-bot-failure";

const t = ((key: string, fallback?: string) => fallback ?? key) as Parameters<
	typeof failureLabel
>[0];

test("a turn without a usable model gets a label instead of its raw code", () => {
	expect(failureLabel(t, "BOT_TURN_NO_MODEL")).toBe("No language model");
});
