import {
	composeDelegatedRawUserPrompt,
	releaseReturnedFlowIrCommitBeforeStaleResponse,
} from "@flow-like/flow-like-ui/components/flowpilot/copilot-request-context";
import type { FlowIrCommitToken } from "@flow-like/flow-like-ui/lib/schema/copilot";
import { describe, expect, test, vi } from "vitest";

const token: FlowIrCommitToken = {
	board_id: "old-board",
	draft_id: "draft",
	revision: 3,
	base_fingerprint: "base",
	claim_id: "claim",
};

describe("copilot request ownership", () => {
	test("delegated acceptance keeps the top-level user request and specialist instruction only", () => {
		const raw = composeDelegatedRawUserPrompt(
			"Bau den sechs-stufigen Support-Mail-Ablauf",
			"Implementiere IMAP, Freigabe und SMTP vollständig",
		);

		expect(raw).toBe(
			"Bau den sechs-stufigen Support-Mail-Ablauf\n\nDelegated specialist request:\nImplementiere IMAP, Freigabe und SMTP vollständig",
		);
		expect(raw).not.toContain("Execute the change NOW");
	});

	test("a stale response releases its returned typed token before rejection continues", async () => {
		const order: string[] = [];
		const dismiss = vi.fn(async (received: FlowIrCommitToken) => {
			order.push(`dismiss:${received.claim_id}`);
			return { status: "dismissed", message: "released" } as const;
		});

		await releaseReturnedFlowIrCommitBeforeStaleResponse(false, token, dismiss);
		order.push("throw-stale");

		expect(order).toEqual(["dismiss:claim", "throw-stale"]);
		expect(dismiss).toHaveBeenCalledWith(token);
	});

	test("an active response does not release its typed token", async () => {
		const dismiss = vi.fn();
		await releaseReturnedFlowIrCommitBeforeStaleResponse(true, token, dismiss);
		expect(dismiss).not.toHaveBeenCalled();
	});
});
