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
	backend: { storageState: {} },
}));

vi.mock("../..", async () => ({
	storageDisplayName: (await import("../../lib/storage-tree"))
		.storageDisplayName,
	BulkUploadPartialFailureError: class extends Error {},
	useBackend: () => fixture.backend,
	useInvoke: () => ({
		data: fixture.items,
		isSuccess: true,
		refetch: fixture.refetch,
	}),
}));

vi.mock("../ui", () => {
	const Wrapper = ({ children }: { children?: ReactNode }) => <>{children}</>;
	return {
		Badge: Wrapper,
		Button: ({
			children,
			onClick,
		}: {
			children?: ReactNode;
			onClick?: () => void;
		}) => (
			<button type="button" onClick={onClick}>
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
	FileOrFolder: ({ file }: { file: IStorageItem }) => {
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
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	sessionStorage.clear();
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
