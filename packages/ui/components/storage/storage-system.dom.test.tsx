import { CancelledError } from "@tanstack/react-query";
import { type ComponentProps, type ReactNode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { IStorageItem } from "../../lib/schema/storage/storage-item";
import { storageDisplayName } from "../../lib/storage-tree";
import { StorageSystem } from "./storage-system";

const fixture = vi.hoisted(() => ({
	items: [] as IStorageItem[],
	renderedLocations: [] as string[],
	refetch: vi.fn(),
	cancelQueries: vi.fn(),
	invalidateQueries: vi.fn(),
	fetchQuery: vi.fn(),
	invoke: undefined as (() => Promise<unknown>) | undefined,
	deleteFile: undefined as ((target: string) => Promise<void>) | undefined,
	backend: {
		storageState: {
			listStorageItems: vi.fn(),
			deleteStorageItems: vi.fn(),
			uploadStorageItems: vi.fn(),
		},
	},
}));

vi.mock("../..", async () => ({
	storageDisplayName: (await import("../../lib/storage-tree"))
		.storageDisplayName,
	BulkUploadPartialFailureError: class extends Error {},
	useBackend: () => fixture.backend,
	useQueryClient: () => ({
		cancelQueries: fixture.cancelQueries,
		invalidateQueries: fixture.invalidateQueries,
		fetchQuery: fixture.fetchQuery,
	}),
	useInvoke: (
		fn: (...args: unknown[]) => Promise<unknown>,
		context: unknown,
		args: unknown[],
	) => {
		fixture.invoke = () => fn.apply(context, args);
		return { data: fixture.items, isSuccess: true, refetch: fixture.refetch };
	},
}));

vi.mock("../ui", () => {
	const Wrapper = ({ children }: { children?: ReactNode }) => <>{children}</>;
	return {
		Badge: Wrapper,
		Button: ({ children, ...props }: ComponentProps<"button">) => (
			<button type="button" {...props}>
				{children}
			</button>
		),
		DropdownMenu: Wrapper,
		DropdownMenuContent: () => null,
		DropdownMenuItem: Wrapper,
		DropdownMenuTrigger: Wrapper,
		EmptyState: () => <p>No Files Found</p>,
		FilePreviewer: () => null,
		Input: (props: ComponentProps<"input">) => <input {...props} />,
		Progress: () => null,
		ResizableHandle: () => null,
		ResizablePanel: Wrapper,
		ResizablePanelGroup: Wrapper,
		Separator: () => null,
		Tooltip: Wrapper,
		TooltipContent: () => null,
		TooltipTrigger: Wrapper,
		isCode: () => false,
		isText: () => false,
	};
});

vi.mock("./storage-breadcrumbs", () => ({
	StorageBreadcrumbs: () => null,
}));

vi.mock("./storage-file-or-folder", () => ({
	FileOrFolder: ({
		file,
		deleteFile,
	}: { file: IStorageItem; deleteFile: (target: string) => Promise<void> }) => {
		fixture.deleteFile = deleteFile;
		fixture.renderedLocations.push(file.location);
		return (
			<div data-storage-location={file.location} data-folder={file.is_dir}>
				{storageDisplayName(file.location)}
			</div>
		);
	},
}));

const item = (location: string, is_dir = true): IStorageItem => ({
	location,
	is_dir,
	size: 0,
	last_modified: "2026-09-27T12:00:00Z",
});

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	sessionStorage.clear();
	fixture.items = [];
	fixture.renderedLocations = [];
	fixture.refetch.mockReset();
	fixture.refetch.mockImplementation(async () => ({
		data: await fixture.invoke?.(),
	}));
	fixture.cancelQueries.mockReset();
	fixture.cancelQueries.mockResolvedValue(undefined);
	fixture.invalidateQueries.mockReset();
	fixture.invalidateQueries.mockResolvedValue(undefined);
	fixture.fetchQuery.mockReset();
	fixture.fetchQuery.mockImplementation(({ queryFn }) => queryFn());
	fixture.invoke = undefined;
	fixture.deleteFile = undefined;
	fixture.backend.storageState.listStorageItems.mockReset();
	fixture.backend.storageState.listStorageItems.mockResolvedValue([]);
	fixture.backend.storageState.deleteStorageItems.mockReset();
	fixture.backend.storageState.deleteStorageItems.mockResolvedValue(undefined);
	fixture.backend.storageState.uploadStorageItems.mockReset();
	fixture.backend.storageState.uploadStorageItems.mockResolvedValue(undefined);
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	sessionStorage.clear();
	vi.restoreAllMocks();
});

async function renderStorage(storageScopeKey = "shared", prefix = "") {
	await act(async () => {
		root.render(
			<StorageSystem
				appId="app-a"
				prefix={prefix}
				storageScopeKey={storageScopeKey}
				updatePrefix={() => {}}
				fileToUrl={async () => ""}
			/>,
		);
	});
}

function visibleFolders() {
	return Array.from(container.querySelectorAll('[data-folder="true"]')).map(
		(row) => row.textContent,
	);
}

describe("Storage listing freshness", () => {
	it("does not report a refresh superseded by another mutation as a failure", async () => {
		const errorLog = vi.spyOn(console, "error").mockImplementation(() => {});
		fixture.fetchQuery.mockRejectedValueOnce(new CancelledError());
		await renderStorage();
		await act(async () => {
			container
				.querySelector<HTMLButtonElement>('[aria-label="Refresh files"]')
				?.click();
		});
		expect(fixture.fetchQuery).toHaveBeenCalledOnce();
		expect(errorLog).not.toHaveBeenCalled();
	});

	it("browses normally and cancels pending reads before an explicit fresh listing", async () => {
		await renderStorage();
		await fixture.invoke?.();
		expect(
			fixture.backend.storageState.listStorageItems,
		).toHaveBeenLastCalledWith("app-a", "", undefined);
		await act(async () => {
			container
				.querySelector<HTMLButtonElement>('[aria-label="Refresh files"]')
				?.click();
		});
		expect(fixture.cancelQueries).toHaveBeenCalledWith({
			queryKey: ["listStorageItems", "app-a", "", "shared"],
			exact: true,
		});
		expect(
			fixture.backend.storageState.listStorageItems,
		).toHaveBeenLastCalledWith("app-a", "", { refresh: true });
		await fixture.invoke?.();
		expect(
			fixture.backend.storageState.listStorageItems,
		).toHaveBeenLastCalledWith("app-a", "", undefined);
	});

	it.each([false, true])(
		"refreshes storage after deletion settles (failure=%s)",
		async (failed) => {
			if (failed) {
				vi.spyOn(console, "error").mockImplementation(() => {});
				fixture.backend.storageState.deleteStorageItems.mockRejectedValueOnce(
					new Error("partial deletion"),
				);
			}
			fixture.items = [item("apps/app-a/upload/file.txt", false)];
			await renderStorage();
			await act(async () => {
				await fixture.deleteFile?.("file.txt");
			});
			expect(
				fixture.backend.storageState.deleteStorageItems,
			).toHaveBeenCalledOnce();
			expect(
				fixture.backend.storageState.listStorageItems,
			).toHaveBeenLastCalledWith("app-a", "", { refresh: true });
		},
	);

	it.each([false, true])(
		"refreshes after direct upload settles (failure=%s)",
		async (failed) => {
			if (failed) {
				vi.spyOn(console, "error").mockImplementation(() => {});
				fixture.backend.storageState.uploadStorageItems.mockRejectedValueOnce(
					new Error("partial upload"),
				);
			}
			await renderStorage();
			const input =
				container.querySelector<HTMLInputElement>('input[type="file"]');
			expect(input).not.toBeNull();
			Object.defineProperty(input, "files", {
				value: [new File(["content"], "file.txt")],
			});
			await act(async () => {
				input?.dispatchEvent(new Event("change", { bubbles: true }));
			});
			expect(
				fixture.backend.storageState.uploadStorageItems,
			).toHaveBeenCalledOnce();
			expect(
				fixture.backend.storageState.listStorageItems,
			).toHaveBeenLastCalledWith("app-a", "", { refresh: true });
		},
	);
});

describe("Storage folder placeholders", () => {
	it.each([
		["shared", "docs", "apps/app-a/upload/docs/"],
		["user", "docs", "users/viewer/apps/app-a/docs/"],
		["shared", "Q1%20draft", "apps/app-a/upload/Q1 draft/"],
		["user", "Q1%20draft", "users/viewer/apps/app-a/Q1 draft/"],
	])(
		"retires the %s placeholder %s when the backend lists the folder",
		async (scope, name, location) => {
			const key = `vfolders:${scope}:app-a`;
			const otherKey = `vfolders:${scope === "shared" ? "user" : "shared"}:app-a`;
			const map = {
				"": [name, "empty"],
				[name]: ["empty-child"],
				unrelated: ["keep"],
			};
			sessionStorage.setItem(key, JSON.stringify(map));
			sessionStorage.setItem(otherKey, JSON.stringify(map));
			await renderStorage(scope);
			expect(visibleFolders()).toContain(storageDisplayName(name));
			expect(visibleFolders()).toHaveLength(2);

			fixture.items = [item(location)];
			await renderStorage(scope);

			expect(
				visibleFolders().filter(
					(folder) => folder === storageDisplayName(name),
				),
			).toHaveLength(1);
			expect(visibleFolders()).toContain("empty");
			expect(JSON.parse(sessionStorage.getItem(key) ?? "null")).toEqual({
				...map,
				"": ["empty"],
			});
			expect(JSON.parse(sessionStorage.getItem(otherKey) ?? "null")).toEqual(
				map,
			);

			fixture.items = [];
			await renderStorage(scope);
			expect(visibleFolders()).toEqual(["empty"]);
		},
	);

	it("keeps a folder placeholder when only a file with the same name appears", async () => {
		const key = "vfolders:shared:app-a";
		sessionStorage.setItem(key, JSON.stringify({ "": ["docs"] }));
		fixture.items = [item("apps/app-a/upload/docs", false)];
		await renderStorage();
		expect(JSON.parse(sessionStorage.getItem(key) ?? "null")).toEqual({
			"": ["docs"],
		});
	});

	it("does not render the previous parent's placeholders after navigation", async () => {
		sessionStorage.setItem(
			"vfolders:shared:app-a",
			JSON.stringify({ "": ["root-empty"], nested: ["nested-empty"] }),
		);
		await renderStorage();
		fixture.renderedLocations = [];
		await renderStorage("shared", "nested");
		expect(visibleFolders()).toEqual(["nested-empty"]);
		expect(fixture.renderedLocations).not.toContain("nested/root-empty");
	});

	it("does not render another storage scope's placeholders while switching", async () => {
		sessionStorage.setItem(
			"vfolders:shared:app-a",
			JSON.stringify({ "": ["shared-empty"] }),
		);
		sessionStorage.setItem(
			"vfolders:user:app-a",
			JSON.stringify({ "": ["user-empty"] }),
		);
		await renderStorage();
		fixture.renderedLocations = [];
		await renderStorage("user");
		expect(visibleFolders()).toEqual(["user-empty"]);
		expect(fixture.renderedLocations).not.toContain("shared-empty");
	});
});
