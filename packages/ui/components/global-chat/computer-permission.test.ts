import { describe, expect, test } from "bun:test";
import { buildToolActivity } from "../../state/global-chat/tool-step-activity";
import {
	RPA_PERMISSIONS_REQUIRED,
	computerPermissionNeed,
	requestComputerPermission,
} from "./computer-permission";

/** The `tool_end` frame a desktop bot turn emits for a failed computer tool (pretty JSON preview). */
function toolEnd(envelope: Record<string, unknown>) {
	return {
		tool_call_id: "call_1",
		tool: "computer_screenshot",
		status: "error",
		result_summary: "error",
		result_preview: JSON.stringify(envelope, null, 2),
	};
}

const missing = (
	permission: string,
	restart = false,
	code = "permission_missing",
) => ({
	status: "error",
	tool: "computer_screenshot",
	permission,
	restart_required: restart,
	error: {
		code,
		message: "Flow Like has no Screen & System Audio Recording permission.",
	},
	truncated: false,
	saved: [],
	warnings: [],
});

describe("computerPermissionNeed", () => {
	test("reads the permission and the restart flag of a permission_missing envelope", () => {
		expect(computerPermissionNeed(missing("screen_recording"))).toEqual({
			permission: "screen_recording",
			restartRequired: false,
		});
		expect(computerPermissionNeed(missing("accessibility", true))).toEqual({
			permission: "accessibility",
			restartRequired: true,
		});
	});

	test("ignores other codes, unknown permissions and plain errors", () => {
		expect(
			computerPermissionNeed(missing("screen_recording", false, "failed")),
		).toBeUndefined();
		expect(computerPermissionNeed(missing("camera"))).toBeUndefined();
		expect(
			computerPermissionNeed({ error: "permission_missing" }),
		).toBeUndefined();
	});
});

describe("tool activity of a failed computer tool", () => {
	test("keeps the permission need from the tool_end preview", () => {
		const detail = buildToolActivity(
			"computer_screenshot",
			toolEnd(missing("screen_recording")),
		);
		expect(detail.permission).toEqual({
			permission: "screen_recording",
			restartRequired: false,
		});
	});

	test("a call with another error code has no permission need", () => {
		const detail = buildToolActivity(
			"computer_screenshot",
			toolEnd(missing("screen_recording", false, "no_display")),
		);
		expect(detail.permission).toBeUndefined();
		expect(detail.outcome).toContain("Screen & System Audio Recording");
	});
});

describe("requestComputerPermission", () => {
	const dispatched = (permission: "screen_recording" | "accessibility") => {
		const target = new EventTarget();
		const seen: unknown[] = [];
		target.addEventListener(RPA_PERMISSIONS_REQUIRED, (event) =>
			seen.push((event as CustomEvent).detail),
		);
		requestComputerPermission(permission, target);
		return seen;
	};

	test("asks the permission dialog for the matching capability with a request id", () => {
		const [screen] = dispatched("screen_recording");
		expect(screen).toMatchObject({ required: ["screen_capture"] });
		expect(typeof (screen as { requestId: unknown }).requestId).toBe("string");
		expect(dispatched("accessibility")[0]).toMatchObject({
			required: ["accessibility"],
		});
	});
});
