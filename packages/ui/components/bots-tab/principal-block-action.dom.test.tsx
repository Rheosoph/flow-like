// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { IConversation } from "../../state/backend-state/inbox-state";
import type { CrewDirectory } from "./use-crew-directory";

const mocks = vi.hoisted(() => ({
	block: vi.fn(),
	unblock: vi.fn(),
	blocks: [] as {
		principal_id: string;
		kind: "user" | "bot";
		created_at: string;
	}[],
}));
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (_key: string, fallback: string, options?: Record<string, string>) =>
			fallback.replace(/\{\{(\w+)\}\}/g, (_, key) => options?.[key] ?? ""),
	}),
}));
vi.mock("../../hooks/use-inbox-blocks", () => ({
	useBlockPrincipal: () => ({ mutateAsync: mocks.block, isPending: false }),
	useUnblockPrincipal: () => ({ mutateAsync: mocks.unblock, isPending: false }),
	useInboxBlocks: () => ({ data: mocks.blocks }),
}));

import {
	PrincipalBlockButton,
	PrincipalBlockDialog,
	conversationBlockTarget,
	principalBlock,
} from "./principal-block-action";

let root: ReturnType<typeof createRoot>;
const bot = {
	id: "other-bot",
	name: "Scout",
	kind: "bot" as const,
	ownerId: "other",
};
beforeEach(() => {
	vi.clearAllMocks();
	mocks.blocks = [];
	mocks.block.mockResolvedValue(undefined);
	mocks.unblock.mockResolvedValue(undefined);
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	const container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});
afterEach(async () => {
	await act(async () => root.unmount());
	document.body.innerHTML = "";
	vi.unstubAllGlobals();
});

function button(label: string) {
	const element = [...document.querySelectorAll("button")].find(
		(button) => button.textContent === label,
	);
	if (!element) throw new Error(`Missing button ${label}`);
	return element;
}

describe("principal blocking", () => {
	test("requires confirmation and explains shared-room behavior", async () => {
		await act(async () =>
			root.render(createElement(PrincipalBlockButton, { principal: bot })),
		);
		await act(async () => button("Block").click());
		expect(mocks.block).not.toHaveBeenCalled();
		expect(document.body.textContent).toContain(
			"The bot will remain in shared rooms.",
		);
		await act(async () => button("Cancel").click());
		expect(mocks.block).not.toHaveBeenCalled();
	});

	test("blocks another user's bot and closes only after success", async () => {
		const close = vi.fn();
		await act(async () =>
			root.render(
				createElement(PrincipalBlockDialog, {
					principal: bot,
					onOpenChange: close,
				}),
			),
		);
		await act(async () => button("Block").click());
		expect(mocks.block).toHaveBeenCalledWith("other-bot");
		expect(mocks.unblock).not.toHaveBeenCalled();
		expect(close).toHaveBeenCalledWith(false);
	});

	test("keeps a failed block open with an error, then permits retry", async () => {
		const close = vi.fn();
		mocks.block.mockRejectedValueOnce(new Error("offline"));
		await act(async () =>
			root.render(
				createElement(PrincipalBlockDialog, {
					principal: bot,
					onOpenChange: close,
				}),
			),
		);
		await act(async () => button("Block").click());
		expect(close).not.toHaveBeenCalled();
		expect(document.querySelector('[role="alert"]')).not.toBeNull();
		await act(async () => button("Block").click());
		expect(close).toHaveBeenCalledWith(false);
	});

	test("unblocks an explicitly blocked principal", async () => {
		const close = vi.fn();
		await act(async () =>
			root.render(
				createElement(PrincipalBlockDialog, {
					principal: bot,
					blocked: true,
					onOpenChange: close,
				}),
			),
		);
		await act(async () => button("Unblock").click());
		expect(mocks.unblock).toHaveBeenCalledWith("other-bot");
		expect(mocks.block).not.toHaveBeenCalled();
	});

	test("a background block-list refresh does not reverse the confirmed action", async () => {
		const close = vi.fn();
		await act(async () =>
			root.render(
				createElement(PrincipalBlockDialog, {
					principal: bot,
					onOpenChange: close,
				}),
			),
		);
		await act(async () =>
			root.render(
				createElement(PrincipalBlockDialog, {
					principal: bot,
					blocked: true,
					onOpenChange: close,
				}),
			),
		);
		await act(async () => button("Block").click());
		expect(mocks.block).toHaveBeenCalledWith(bot.id);
		expect(mocks.unblock).not.toHaveBeenCalled();
	});

	test("does not offer a misleading unblock for an owner's inherited block", async () => {
		mocks.blocks = [{ principal_id: "other", kind: "user", created_at: "" }];
		await act(async () =>
			root.render(createElement(PrincipalBlockButton, { principal: bot })),
		);
		expect(button("Owner blocked").disabled).toBe(true);
		expect(principalBlock(mocks.blocks, bot)?.principal_id).toBe("other");
		expect(
			principalBlock(
				[
					...mocks.blocks,
					{ principal_id: bot.id, kind: "bot", created_at: "" },
				],
				bot,
			)?.principal_id,
		).toBe(bot.id);
	});

	test("offers blocks for people, foreign bots and pending partners, never own bots or local chats", () => {
		const directory = ((id: string) => ({
			name: id,
			ownerId: id === "own-bot" ? "me" : "other",
		})) as CrewDirectory;
		const direct = {
			kind: "direct",
			members: [
				{ principal_id: "me", kind: "user" },
				{ principal_id: "other-bot", kind: "bot" },
			],
		} as IConversation;
		expect(conversationBlockTarget(direct, directory, "me")?.id).toBe(
			"other-bot",
		);
		expect(
			conversationBlockTarget({ ...direct, local: true }, directory, "me"),
		).toBeUndefined();
		expect(
			conversationBlockTarget({ ...direct, kind: "group" }, directory, "me"),
		).toBeUndefined();
		expect(conversationBlockTarget(direct, directory)).toBeUndefined();
		expect(
			conversationBlockTarget(
				{
					...direct,
					members: [
						direct.members[0],
						{ ...direct.members[1], principal_id: "own-bot" },
					],
				},
				directory,
				"me",
			),
		).toBeUndefined();
		expect(
			conversationBlockTarget(
				{
					...direct,
					members: [direct.members[0]],
					pending: [{ principal_id: "other", kind: "user", status: "pending" }],
				} as IConversation,
				directory,
				"me",
			)?.id,
		).toBe("other");
	});
});
