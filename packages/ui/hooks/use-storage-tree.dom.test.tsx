import {
	QueryClient,
	QueryClientProvider,
	useQuery,
} from "@tanstack/react-query";
import { type ReactNode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { IStorageItem } from "../lib/schema/storage/storage-item";
import { type IStorageTree, useStorageTree } from "./use-storage-tree";

const fixture = vi.hoisted(() => ({
	list: vi.fn(),
	userList: vi.fn(),
	ready: true,
}));
vi.mock("../state/backend-state", () => ({
	useBackendReady: () => fixture.ready,
	useBackend: () => ({
		storageState: {
			listStorageItems: fixture.list,
			listStorageItemsUser: fixture.userList,
		},
	}),
}));

let client: QueryClient;
let root: Root;
let container: HTMLDivElement;
let tree: IStorageTree;

function Harness({
	enabled = true,
	scope = "app",
}: {
	enabled?: boolean;
	scope?: "app" | "user";
}) {
	tree = useStorageTree({ appId: "app", scope, enabled });
	return <div>{tree.root.entries.map((entry) => entry.name).join(",")}</div>;
}

function OtherListingObserver() {
	useQuery({
		queryKey: [fixture.list.name || "backendFn", "app", ""],
		queryFn: () => fixture.list("app", ""),
		staleTime: 30_000,
	});
	return null;
}

function StoragePageObserver({
	scope = "shared",
	prefix = "",
}: {
	scope?: "shared" | "user";
	prefix?: string;
}) {
	const listing = useQuery({
		queryKey: ["listStorageItems", "app", prefix, scope],
		queryFn: () =>
			(scope === "user" ? fixture.userList : fixture.list)("app", prefix),
		staleTime: 30_000,
	});
	return (
		<div data-testid="storage-page">
			{listing.data?.map((value: IStorageItem) => value.location).join(",")}
		</div>
	);
}

const item = (name: string, is_dir = false): IStorageItem => ({
	location: `apps/app/upload/${name}`,
	is_dir,
	size: 1,
	last_modified: "",
});

async function settle() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}

async function render(children: ReactNode = <Harness />) {
	await act(async () => {
		root.render(
			<QueryClientProvider client={client}>{children}</QueryClientProvider>,
		);
	});
	await settle();
}

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	fixture.list.mockReset();
	fixture.userList.mockReset();
	Object.defineProperty(fixture.list, "name", { value: "listStorageItems" });
	Object.defineProperty(fixture.userList, "name", {
		value: "listStorageItemsUser",
	});
	fixture.ready = true;
	client = new QueryClient({
		defaultOptions: {
			queries: { retry: false, gcTime: Number.POSITIVE_INFINITY },
		},
	});
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	client.clear();
	container.remove();
});

it("starts a fresh LIST when a mutation finishes before the initial request", async () => {
	let finishOld!: (items: IStorageItem[]) => void;
	fixture.list.mockImplementationOnce(
		() =>
			new Promise<IStorageItem[]>((resolve) => {
				finishOld = resolve;
			}),
	);
	fixture.list.mockResolvedValueOnce([item("new.txt")]);
	await render();
	expect(fixture.list).toHaveBeenNthCalledWith(1, "app", "");
	await act(async () => {
		await tree.refetch();
	});
	expect(fixture.list).toHaveBeenNthCalledWith(2, "app", "", { refresh: true });
	finishOld([]);
	await settle();
	expect(container.textContent).toBe("new.txt");
	expect(fixture.list).toHaveBeenCalledTimes(2);
});

it("forces storage refresh when another observer shares the query key", async () => {
	let source = "before.txt";
	let cached: IStorageItem[] | undefined;
	fixture.list.mockImplementation(async (_app, _prefix, options) => {
		if (!cached || options?.refresh) cached = [item(source)];
		return cached;
	});
	await render(
		<>
			<Harness />
			<OtherListingObserver />
		</>,
	);
	source = "after.txt";
	fixture.list.mockClear();
	await act(async () => {
		await tree.refetch();
	});
	await settle();
	expect(fixture.list).toHaveBeenCalledExactlyOnceWith("app", "", {
		refresh: true,
	});
	expect(container.textContent).toBe("after.txt");
});

it.each([false, true])(
	"refreshes a collapsed folder before it is reopened (whole tree=%s)",
	async (wholeTree) => {
		let source = "before.txt";
		const cached = new Map<string, IStorageItem[]>();
		fixture.list.mockImplementation(async (_app, prefix, options) => {
			if (!cached.has(prefix) || options?.refresh) {
				cached.set(
					prefix,
					prefix === "" ? [item("docs", true)] : [item(`docs/${source}`)],
				);
			}
			return cached.get(prefix);
		});
		await render();
		await act(async () => tree.expand("docs"));
		await settle();
		expect(tree.directories.get("docs")?.entries[0]?.name).toBe("before.txt");
		await act(async () => tree.collapse("docs"));
		source = "after.txt";
		await act(async () => {
			await tree.refetch(wholeTree ? undefined : "/docs/");
		});
		await act(async () => tree.expand("docs"));
		await settle();
		expect(tree.directories.get("docs")?.entries[0]?.name).toBe("after.txt");
		expect(fixture.list).toHaveBeenCalledWith("app", "docs", { refresh: true });
	},
);

it("refreshes an upload destination that has never been expanded", async () => {
	fixture.list.mockResolvedValue([item("new.txt")]);
	await render();
	fixture.list.mockClear();
	await act(async () => {
		await tree.refetch("docs");
	});
	expect(fixture.list).toHaveBeenCalledExactlyOnceWith("app", "docs", {
		refresh: true,
	});
});

it("does not replace differently scoped browser data during a refresh", async () => {
	fixture.list.mockResolvedValue([item("shared.txt")]);
	const privateKey = [fixture.list.name || "backendFn", "app", "", "user"];
	const privateItems = [item("private.txt")];
	client.setQueryData(privateKey, privateItems);
	const customKey = ["listStorageItems", "app", "", "custom-provider"];
	client.setQueryData(customKey, privateItems);
	await render();
	fixture.list.mockClear();
	await act(async () => {
		await tree.refetch();
	});
	expect(fixture.list).toHaveBeenCalledExactlyOnceWith("app", "", {
		refresh: true,
	});
	expect(client.getQueryData(privateKey)).toEqual(privateItems);
	expect(client.getQueryData(customKey)).toEqual(privateItems);
});

it.each(["shared", "user"] as const)(
	"updates an inactive %s storage page before returning to it",
	async (scope) => {
		const list = scope === "user" ? fixture.userList : fixture.list;
		let source = [item("deleted.txt")];
		let cached: IStorageItem[] | undefined;
		list.mockImplementation(async (_app, _prefix, options) => {
			if (!cached || options?.refresh) cached = source;
			return cached;
		});
		const pageKey = ["listStorageItems", "app", "", scope];
		await client.fetchQuery({
			queryKey: pageKey,
			queryFn: () => list("app", ""),
		});
		const oppositeKey = [
			"listStorageItems",
			"app",
			"",
			scope === "user" ? "shared" : "user",
		];
		client.setQueryData(oppositeKey, [item("keep.txt")]);
		await render(<Harness scope={scope === "user" ? "user" : "app"} />);
		source = [];
		await act(async () => {
			await tree.refetch();
		});
		expect(client.getQueryData(pageKey)).toEqual([]);
		expect(client.getQueryData(oppositeKey)).toEqual([item("keep.txt")]);
		list.mockClear();
		await render(<StoragePageObserver scope={scope} />);
		expect(
			container.querySelector('[data-testid="storage-page"]')?.textContent,
		).toBe("");
		expect(list).not.toHaveBeenCalled();
	},
);

it("refreshes a storage page folder that was never expanded in the tree", async () => {
	fixture.list.mockResolvedValue([]);
	const pageKey = ["listStorageItems", "app", "docs", "shared"];
	client.setQueryData(pageKey, [item("docs/deleted.txt")]);
	await render();
	await act(async () => {
		await tree.refetch();
	});
	expect(client.getQueryData(pageKey)).toEqual([]);
	expect(fixture.list).toHaveBeenCalledWith("app", "docs", { refresh: true });
});

it("cancels pending storage page reads before refreshing their cached result", async () => {
	fixture.list.mockResolvedValue([]);
	await render();
	let finishOld!: (items: IStorageItem[]) => void;
	fixture.list.mockImplementationOnce(
		() =>
			new Promise<IStorageItem[]>((resolve) => {
				finishOld = resolve;
			}),
	);
	await render(
		<>
			<Harness />
			<StoragePageObserver />
		</>,
	);
	await act(async () => {
		await tree.refetch();
	});
	finishOld([item("deleted.txt")]);
	await settle();
	expect(
		client.getQueryData(["listStorageItems", "app", "", "shared"]),
	).toEqual([]);
	expect(
		container.querySelector('[data-testid="storage-page"]')?.textContent,
	).toBe("");
});

it.each(["disabled", "not ready"])(
	"does not fetch while the tree is %s",
	async (state) => {
		fixture.ready = state !== "not ready";
		await render(<Harness enabled={state !== "disabled"} />);
		await act(async () => {
			await tree.refetch();
			await tree.refetch("docs");
		});
		expect(fixture.list).not.toHaveBeenCalled();
	},
);

it("shows refresh errors without rejecting the refresh callback", async () => {
	fixture.list.mockResolvedValueOnce([item("before.txt")]);
	await render();
	fixture.list.mockRejectedValueOnce(new Error("storage unavailable"));
	await act(async () => {
		await expect(tree.refetch()).resolves.toBeUndefined();
	});
	await settle();
	expect(tree.root.error?.message).toBe("storage unavailable");
});

it("keeps the newest result when another refresh cancels an earlier one", async () => {
	fixture.list.mockResolvedValueOnce([item("before.txt")]);
	await render();
	let finishOld!: (items: IStorageItem[]) => void;
	fixture.list.mockImplementationOnce(
		() =>
			new Promise<IStorageItem[]>((resolve) => {
				finishOld = resolve;
			}),
	);
	let first!: Promise<void>;
	await act(async () => {
		first = tree.refetch();
	});
	expect(fixture.list).toHaveBeenCalledTimes(2);
	fixture.list.mockResolvedValueOnce([item("newest.txt")]);
	await act(async () => {
		await tree.refetch();
		await first;
	});
	finishOld([item("outdated.txt")]);
	await settle();
	expect(container.textContent).toBe("newest.txt");
});

it("bounds concurrent requests when refreshing many cached folders", async () => {
	fixture.list.mockResolvedValue([]);
	await render();
	for (let index = 0; index < 9; index++) {
		client.setQueryData(
			[fixture.list.name || "backendFn", "app", `folder-${index}`],
			[],
		);
	}
	fixture.list.mockClear();
	const pending: (() => void)[] = [];
	fixture.list.mockImplementation(
		() =>
			new Promise<IStorageItem[]>((resolve) => {
				pending.push(() => resolve([]));
			}),
	);
	let refresh!: Promise<void>;
	await act(async () => {
		refresh = tree.refetch();
	});
	expect(pending).toHaveLength(4);
	for (const expectedBatch of [4, 4, 2]) {
		expect(pending).toHaveLength(expectedBatch);
		await act(async () => {
			for (const finish of pending.splice(0)) finish();
		});
	}
	await refresh;
	expect(fixture.list).toHaveBeenCalledTimes(10);
});
