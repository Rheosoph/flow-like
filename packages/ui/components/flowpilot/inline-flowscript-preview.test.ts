import { describe, expect, test } from "bun:test";
import {
	resolveDisplayedFlowScriptPreview,
	resolveLiveFlowScriptPreviewForMessage,
} from "./inline-flowscript-preview";

describe("inline FlowScript preview ownership", () => {
	test("gives the live draft only to the latest assistant message", () => {
		const preview = { source: "eventsSimple() {}", status: "drafting" };

		expect(
			resolveLiveFlowScriptPreviewForMessage({
				isLatestMessage: false,
				messageRole: "assistant",
				preview,
			}),
		).toBeUndefined();
		expect(
			resolveLiveFlowScriptPreviewForMessage({
				isLatestMessage: true,
				messageRole: "user",
				preview,
			}),
		).toBeUndefined();
		expect(
			resolveLiveFlowScriptPreviewForMessage({
				isLatestMessage: true,
				messageRole: "assistant",
				preview,
				workspaceStatus: "queued",
			}),
		).toEqual({ source: preview.source, status: "queued" });
	});

	test("keeps an older message's authoritative FlowScript on that message", () => {
		expect(
			resolveDisplayedFlowScriptPreview({
				messageRole: "assistant",
				messageWorkspace: 'eventsSimple() { log("finished") }',
			}),
		).toEqual({ source: 'eventsSimple() { log("finished") }' });
	});
});
