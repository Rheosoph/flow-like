import { afterAll, afterEach, describe, expect, spyOn, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ComponentProps } from "react";
import { toast } from "sonner";
import type { FlowVersionState } from "../../../lib/device-management/latest-flows";
import {
	type IBoard,
	IExecutionStage,
	ILogLevel,
} from "../../../lib/schema/flow/board";
import type { BoardVersion } from "../../../lib/schema/flow/board-version";
import type { IBackendState } from "../../../state/backend-state";
import {
	byRole,
	click,
	installDom,
	settle,
} from "../../settings/devices/testing/dom-harness";

const dom = installDom();
const { BoardReleaseForm, BoardReleaseStatus } = await import(
	"./board-meta-controls"
);
const { useBackendStore } = await import("../../../state/backend-state");
const originalBackend = useBackendStore.getState().backend;
const successToast = spyOn(toast, "success");
const clients: QueryClient[] = [];

afterEach(async () => {
	await dom.cleanup();
	for (const client of clients.splice(0)) client.clear();
	useBackendStore.setState({ backend: originalBackend });
	successToast.mockClear();
});

afterAll(() => {
	successToast.mockRestore();
	dom.restore();
});

async function renderRelease(
	options: {
		version?: BoardVersion;
		state?: FlowVersionState;
		failStatusAfterPublish?: boolean;
		failStatus?: boolean;
		failVersions?: boolean;
		publishedState?: FlowVersionState;
	} = {},
) {
	let published = false;
	let state: FlowVersionState = options.state ?? {
		current: null,
		newest: [0, 0, 4],
	};
	let versions: BoardVersion[] = state.newest
		? [state.newest, [0, 0, 2], [0, 0, 1]]
		: [];
	const board = {
		id: "board",
		name: "Example",
		description: "",
		stage: IExecutionStage.Dev,
		log_level: ILogLevel.Info,
		version: options.version ?? [0, 0, 5],
	} as IBoard;
	useBackendStore.setState({
		backend: {
			boardState: {
				getBoard: async function getBoard() {
					return board;
				},
				getBoardVersions: async function getBoardVersions() {
					if (options.failVersions) throw new Error("Version list unavailable");
					return versions;
				},
				getBoardVersionCurrent: async function getBoardVersionCurrent() {
					if (
						options.failStatus ||
						(published && options.failStatusAfterPublish)
					)
						throw new Error("Publication status unavailable");
					return state;
				},
				getBoardVersionInfos: async function getBoardVersionInfos() {
					return [];
				},
				createBoardVersion: async function createBoardVersion() {
					published = true;
					state = options.publishedState ?? {
						current: [0, 0, 5],
						newest: [0, 0, 5],
					};
					versions = [[0, 0, 5], ...versions];
					return [0, 0, 6];
				},
			},
			eventState: {},
		} as unknown as IBackendState,
	});
	const client = new QueryClient({
		defaultOptions: {
			queries: { retry: false, staleTime: Number.POSITIVE_INFINITY },
		},
	});
	clients.push(client);
	client.setQueryData(["getBoard", "app", "board"], board);
	client.setQueryData(["getBoardVersionInfos", "app", "board"], []);
	const selected: (BoardVersion | undefined)[] = [];
	const props: ComponentProps<typeof BoardReleaseForm> = {
		appId: "app",
		boardId: "board",
		board,
		version: options.version,
		selectVersion: (version) => selected.push(version),
	};
	const rendered = await dom.render(
		<QueryClientProvider client={client}>
			<div data-testid="status">
				<BoardReleaseStatus
					appId="app"
					boardId="board"
					stage={board.stage}
					version={options.version}
				/>
			</div>
			<BoardReleaseForm {...props} />
		</QueryClientProvider>,
	);
	await settle();
	return { ...rendered, client, selected };
}

describe("board release versions", () => {
	test("distinguishes the draft from published snapshots in the editor", async () => {
		const { container } = await renderRelease();
		expect(container.textContent).toContain("Draft (unpublished)");
		expect(container.textContent).toContain("Published v0.0.4");
		expect(container.textContent).toContain("Newest published: v0.0.4");
		expect(container.textContent).not.toContain("0.0.5");
		await click(byRole("combobox", "Version"));
		expect(byRole("option", "v0.0.4")).toBeDefined();
		expect(byRole("option", "Draft (unpublished)")).toBeDefined();
	});

	test("keeps an older pin separate from the newest release and the draft", async () => {
		const { container, selected } = await renderRelease({ version: [0, 0, 2] });
		expect(
			container.querySelector('[data-testid="status"]')?.textContent,
		).toContain("v0.0.2");
		expect(container.textContent).toContain("Newest published: v0.0.4");
		await click(byRole("combobox", "Version"));
		await click(byRole("option", "Draft (unpublished)"));
		expect(selected).toEqual([undefined]);
	});

	test("confirms the published snapshot and refreshes its status after creation", async () => {
		const { container, client } = await renderRelease();
		await click(byRole("button", "Patch"));
		await settle();
		expect(successToast).toHaveBeenCalledWith("Published v0.0.5");
		expect(container.textContent).toContain("Published v0.0.5");
		expect(container.textContent).toContain("Newest published: v0.0.5");
		expect(container.textContent).not.toContain("0.0.6");
		expect(
			client.getQueryState(["getBoard", "app", "board"])?.isInvalidated,
		).toBe(true);
		expect(
			client.getQueryState(["getBoardVersionInfos", "app", "board"])
				?.isInvalidated,
		).toBe(true);
		await click(byRole("combobox", "Version"));
		expect(byRole("option", "v0.0.5")).toBeDefined();
	});

	test("shows an unpublished flow without inventing a release", async () => {
		const { container } = await renderRelease({
			state: { current: null, newest: null },
		});
		expect(container.textContent).toContain("No published versions");
		expect(container.textContent).toContain("Draft (unpublished)");
		expect(container.textContent).not.toContain("0.0.5");
	});

	test("does not confirm a stale version when publication status cannot refresh", async () => {
		const { container } = await renderRelease({ failStatusAfterPublish: true });
		await click(byRole("button", "Patch"));
		await settle();
		expect(successToast).toHaveBeenCalledWith("Version created");
		expect(
			container.querySelector('[data-testid="status"]')?.textContent,
		).not.toContain("v0.0.4");
		expect(container.textContent).toContain("Newest published: v0.0.5");
	});

	test("does not mistake the newest snapshot for a matching publication", async () => {
		await renderRelease({
			publishedState: { current: null, newest: [0, 0, 5] },
		});
		await click(byRole("button", "Patch"));
		await settle();
		expect(successToast).toHaveBeenCalledWith("Version created");
	});

	test("does not report no publications when metadata fails and the list is empty", async () => {
		const { container } = await renderRelease({
			state: { current: null, newest: null },
			failStatus: true,
		});
		expect(container.textContent).toContain("Published versions unavailable");
		expect(container.textContent).not.toContain("No published versions");
	});

	test("reports unavailable publications when both reads fail", async () => {
		const { container } = await renderRelease({
			failStatus: true,
			failVersions: true,
		});
		expect(container.textContent).toContain("Published versions unavailable");
		expect(container.textContent).not.toContain("Loading published versions");
	});
});
