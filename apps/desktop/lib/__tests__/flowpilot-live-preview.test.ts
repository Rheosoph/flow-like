import { PendingCommandsView } from "@flow-like/flow-like-ui/components/flowpilot/PendingCommandsView";
import {
	InlineFlowScriptPreview,
	flowScriptWorkspaceOwnsApply,
	isDraftingFlowScriptWorkspace,
} from "@flow-like/flow-like-ui/components/flowpilot/inline-flowscript-preview";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, test } from "vitest";

describe("FlowPilot live FlowScript preview", () => {
	test("renders an incomplete drafting snapshot directly in the assistant artifact", () => {
		const source = "function buildReply() {\n  emailSmtpSend({";
		const markup = renderToStaticMarkup(
			createElement(InlineFlowScriptPreview, {
				preview: { source, status: "drafting" },
			}),
		);

		expect(markup).toContain("Generated FlowScript source preview");
		expect(markup).toContain("Writing");
		expect(markup).toContain("function buildReply()");
		expect(markup).toContain("emailSmtpSend");
	});

	test("only lets the authoritative queued source own the apply path", () => {
		expect(isDraftingFlowScriptWorkspace("drafting")).toBe(true);
		expect(flowScriptWorkspaceOwnsApply("eventsSimple() {}", "drafting")).toBe(
			false,
		);
		expect(flowScriptWorkspaceOwnsApply("eventsSimple() {}", "submitted")).toBe(
			false,
		);
		expect(flowScriptWorkspaceOwnsApply("eventsSimple() {}", "stale")).toBe(
			false,
		);
		expect(flowScriptWorkspaceOwnsApply("eventsSimple() {}", "queued")).toBe(
			true,
		);
	});

	test("shows a dismiss-only control for a stale retained review", () => {
		const markup = renderToStaticMarkup(
			createElement(PendingCommandsView, {
				commands: [],
				dismissOnly: true,
				onExecute: () => undefined,
				onExecuteSingle: () => undefined,
				onDismiss: () => undefined,
			}),
		);

		expect(markup).toContain("Review is stale");
		expect(markup).toContain("Dismiss only");
		expect(markup).not.toContain("Apply All");
	});
});
