import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { type ReactNode, act } from "react";
import type { IBackendState } from "../../state/backend-state";
import { installStageDom } from "../interfaces/form-workbench/stage/dom-setup";

const dom = installStageDom();
const { BackendContext } = await import("../../state/backend-state");
const { TextEditor } = await import("../ui/text-editor");
const { StreamingTextEditor } = await import("../ui/streaming-text-editor");
const { BotFileAssets } = await import("./bot-file-assets");

const clients: QueryClient[] = [];
afterEach(async () => {
	await dom.cleanup();
	for (const client of clients.splice(0)) client.clear();
});
afterAll(dom.restore);

function harness(
	download: (appId: string, paths: string[]) => Promise<string>,
) {
	const workspaces: string[] = [];
	const downloads: [string, string[]][] = [];
	const backend = {
		botState: {
			getWorkspace: async (botId: string) => {
				workspaces.push(botId);
				return { app_id: `workspace_${botId}` };
			},
		},
		storageState: {
			downloadStorageItems: async (appId: string, paths: string[]) => {
				downloads.push([appId, paths]);
				return [{ prefix: paths[0], url: await download(appId, paths) }];
			},
		},
	} as unknown as IBackendState;
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	clients.push(client);
	return {
		workspaces,
		downloads,
		render(botId: string | undefined, child: ReactNode) {
			return (
				<QueryClientProvider client={client}>
					<BackendContext.Provider value={backend}>
						<BotFileAssets botId={botId}>{child}</BotFileAssets>
					</BackendContext.Provider>
				</QueryClientProvider>
			);
		},
	};
}

const storedImage = "bot/images/img-tgwmpdbs-1.png";
const markdown = `![Purple cloud](${storedImage})\n\n[Download report](bot/files/report.pdf)`;
const signed = (appId: string, paths: string[]) =>
	Promise.resolve(`https://files.test/${appId}/${paths[0]}?signature=read`);

describe("bot file references in chat", () => {
	test("resolves images and file links while streaming and after reopening", async () => {
		const h = harness(signed);
		const view = await dom.render(
			h.render("scout", <StreamingTextEditor content={markdown} />),
		);
		expect(document.querySelector("img")?.getAttribute("src")).toBe(
			await signed("workspace_scout", [storedImage]),
		);
		expect(document.querySelector("a")?.getAttribute("href")).toBe(
			await signed("workspace_scout", ["bot/files/report.pdf"]),
		);
		expect(h.workspaces).toEqual(["scout"]);
		expect(h.downloads).toContainEqual(["workspace_scout", [storedImage]]);

		await view.unmount();
		await dom.render(
			h.render("scout", <TextEditor initialContent={markdown} isMarkdown />),
		);
		expect(document.querySelector("img")?.getAttribute("src")).toBe(
			await signed("workspace_scout", [storedImage]),
		);
		expect(document.querySelector("a")?.getAttribute("href")).toBe(
			await signed("workspace_scout", ["bot/files/report.pdf"]),
		);
	});

	test("owned storage links resolve without an active bot", async () => {
		const h = harness(signed);
		const url = `storage://apps/workspace_scout/app/${storedImage}`;
		await dom.render(
			h.render(
				undefined,
				<TextEditor
					initialContent={`![Purple cloud](${url})\n\n[Download](${url})`}
					isMarkdown
				/>,
			),
		);
		expect(document.querySelector("img")?.getAttribute("src")).toBe(
			await signed("workspace_scout", [storedImage]),
		);
		expect(document.querySelector("a")?.getAttribute("href")).toBe(
			await signed("workspace_scout", [storedImage]),
		);
		expect(h.workspaces).toEqual([]);
	});

	test.each([
		"asset://localhost/%2Fworkspace%2Fbot%2Ffiles%2Freport.pdf",
		"data:application/pdf;base64,cGRm",
		"blob:https://app.test/file-1",
	])("supports local storage downloads: %s", async (url) => {
		const h = harness(async () => url);
		await dom.render(
			h.render(
				"scout",
				<TextEditor
					initialContent="[Report](bot/files/report.pdf)"
					isMarkdown
				/>,
			),
		);
		expect(document.querySelector("a")?.getAttribute("href")).toBe(url);
		expect(document.querySelector("a")?.hasAttribute("download")).toBe(true);
	});

	test("keeps raw native URLs and active storage responses out of links", async () => {
		const h = harness(async () => "javascript:alert(1)");
		await dom.render(
			h.render(
				"scout",
				<TextEditor
					initialContent="[Raw](asset://localhost/private.pdf)\n\n[File](bot/files/report.pdf)"
					isMarkdown
				/>,
			),
		);
		for (const link of document.querySelectorAll("a")) {
			expect(link.getAttribute("href")).toBeNull();
		}
	});

	test("pending files never use browser-relative URLs or a previous bot's result", async () => {
		let finishFirst!: (url: string) => void;
		const h = harness((appId, paths) =>
			appId === "workspace_scout"
				? new Promise((resolve) => {
						finishFirst = resolve;
					})
				: signed(appId, paths),
		);
		const image = (
			<TextEditor initialContent={`![Cloud](${storedImage})`} isMarkdown />
		);
		const view = await dom.render(h.render("scout", image));
		expect(document.querySelector("img")?.getAttribute("src")).toBeNull();
		await view.rerender(h.render("writer", image));
		const expected = await signed("workspace_writer", [storedImage]);
		expect(document.querySelector("img")?.getAttribute("src")).toBe(expected);
		await act(async () => finishFirst("https://files.test/old-bot.png"));
		expect(document.querySelector("img")?.getAttribute("src")).toBe(expected);
	});

	test("failed links stay unavailable and external URLs need no workspace", async () => {
		const h = harness(async () => {
			throw new Error("File unavailable");
		});
		const view = await dom.render(
			h.render("scout", <TextEditor initialContent={markdown} isMarkdown />),
		);
		expect(document.querySelector("img")?.getAttribute("src")).toBeNull();
		expect(document.querySelector("a")?.getAttribute("href")).toBeNull();
		expect(document.body.textContent).toContain("Download report");
		const before = h.downloads.length;
		await view.rerender(
			h.render(
				"writer",
				<TextEditor
					initialContent="![Web image](https://example.test/image.png)\n\n[Site](https://example.test)"
					isMarkdown
				/>,
			),
		);
		expect(document.querySelector("img")?.getAttribute("src")).toBe(
			"https://example.test/image.png",
		);
		expect(document.querySelector("a")?.getAttribute("href")).toBe(
			"https://example.test/",
		);
		expect(h.downloads).toHaveLength(before);
		expect(h.workspaces).toEqual(["scout"]);
	});
});
