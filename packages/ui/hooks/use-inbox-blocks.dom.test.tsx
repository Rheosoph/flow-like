// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	state: {
		listBlocks: vi.fn(),
		blockPrincipal: vi.fn(),
		unblockPrincipal: vi.fn(),
	},
}));
vi.mock("../state/backend-state", () => ({
	useBackend: () => ({ inboxState: mocks.state }),
}));
vi.mock("./use-inbox", () => ({
	inboxKeys: { all: ["inbox"] },
	INBOX_POLL_MS: { conversations: 15_000 },
}));

import {
	inboxBlockKeys,
	useBlockPrincipal,
	useInboxBlocks,
	useUnblockPrincipal,
} from "./use-inbox-blocks";

const clients: QueryClient[] = [];
const roots: ReturnType<typeof createRoot>[] = [];

beforeEach(() => {
	vi.resetAllMocks();
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	mocks.state.listBlocks.mockResolvedValue([]);
});
afterEach(async () => {
	await act(async () => {
		for (const root of roots.splice(0)) root.unmount();
	});
	for (const client of clients.splice(0)) client.clear();
	document.body.innerHTML = "";
	vi.unstubAllGlobals();
});

async function mount(enabled = false) {
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
	});
	clients.push(client);
	let hooks!: {
		blocks: ReturnType<typeof useInboxBlocks>;
		block: ReturnType<typeof useBlockPrincipal>;
		unblock: ReturnType<typeof useUnblockPrincipal>;
	};
	function Harness() {
		hooks = {
			blocks: useInboxBlocks(enabled),
			block: useBlockPrincipal(),
			unblock: useUnblockPrincipal(),
		};
		return null;
	}
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	roots.push(root);
	await act(async () => {
		root.render(
			createElement(QueryClientProvider, { client }, createElement(Harness)),
		);
	});
	return { client, hooks: () => hooks };
}

describe("block controls", () => {
	test("does not fetch while disabled", async () => {
		await mount(false);
		expect(mocks.state.listBlocks).not.toHaveBeenCalled();
	});

	test("fetches the account block list when enabled", async () => {
		const { client } = await mount(true);
		expect(mocks.state.listBlocks).toHaveBeenCalledOnce();
		expect(client.getQueryData(inboxBlockKeys.all)).toEqual([]);
	});

	test.each(["block", "unblock"] as const)(
		"%s refreshes blocks, invitations, and both person and bot conversations",
		async (action) => {
			const { client, hooks } = await mount();
			const keys = [
				inboxBlockKeys.all,
				["inbox", "invitations"],
				["inbox", "conversations", "me"],
				["inbox", "conversations", "bot-1"],
				["inbox", "conversation", "room-1", "me"],
			];
			for (const queryKey of keys) client.setQueryData(queryKey, []);
			client.setQueryData(["bots"], []);
			await act(async () => {
				await hooks()[action].mutateAsync("user-2");
			});
			expect(
				action === "block"
					? mocks.state.blockPrincipal
					: mocks.state.unblockPrincipal,
			).toHaveBeenCalledWith("user-2");
			for (const queryKey of keys) {
				expect(client.getQueryState(queryKey)?.isInvalidated).toBe(true);
			}
			expect(client.getQueryState(["bots"])?.isInvalidated).toBe(false);
		},
	);

	test("a failed block leaves the cached block list intact", async () => {
		const { client, hooks } = await mount();
		const previous = [{ principal_id: "user-3", kind: "user" }];
		client.setQueryData(inboxBlockKeys.all, previous);
		mocks.state.blockPrincipal.mockRejectedValue(new Error("Refused"));
		await act(async () => {
			await expect(hooks().block.mutateAsync("user-2")).rejects.toThrow(
				"Refused",
			);
		});
		expect(client.getQueryData(inboxBlockKeys.all)).toEqual(previous);
		expect(client.getQueryState(inboxBlockKeys.all)?.isInvalidated).toBe(false);
	});
});
