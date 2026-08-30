import { expect, test } from "vitest";
import { subprocessFailureMessage } from "../subprocess";

test("reports a silent subprocess failure with its exit status", () => {
	expect(
		subprocessFailureMessage(
			"Render helper",
			{ code: 17, signal: null },
			"",
			"",
		),
	).toBe("Render helper exited with code 17.");
	expect(
		subprocessFailureMessage("Render helper", {
			code: null,
			signal: "SIGTERM",
		}),
	).toBe("Render helper exited after signal SIGTERM.");
});
