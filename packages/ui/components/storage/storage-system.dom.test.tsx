import { CancelledError } from "@tanstack/react-query";
import { type ComponentProps, type ReactNode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { buildUploadPath } from "../../lib/bulk-upload";
import type { IStorageItem } from "../../lib/schema/storage/storage-item";
import { storageDisplayName } from "../../lib/storage-tree";
import type {
	IListLockedItemsResponse,
	ILockedItem,
	ILockedStorageStatus,
} from "../../state/backend-state/locked-storage-state";
import { StorageSystem } from "./storage-system";

type DocxProps = ComponentProps<
	typeof import("./wordcraft-document").WordcraftDocument
>;

type CraftProps = ComponentProps<
	typeof import("./craft-document").CraftDocument
>;

const fixture = vi.hoisted(() => ({
	items: [] as IStorageItem[],
	renderedLocations: [] as string[],
	refetch: vi.fn(),
	cancelQueries: vi.fn(),
	invalidateQueries: vi.fn(),
	fetchQuery: vi.fn(),
	setQueryData: vi.fn(),
	cachedQueries: [] as { queryKey: unknown[] }[],
	invoke: undefined as (() => Promise<unknown>) | undefined,
	deleteFile: undefined as ((target: string) => Promise<void>) | undefined,
	loadFile: undefined as ((target: string) => Promise<void>) | undefined,
	changePrefix: undefined as ((prefix: string) => void) | undefined,
	closeDialog: undefined as (() => void) | undefined,
	docxProps: undefined as DocxProps | undefined,
	docxRender: vi.fn(),
	craftProps: undefined as CraftProps | undefined,
	craftRender: vi.fn(),
	previewable: new Map<string, boolean | undefined>(),
	updatePrefix: vi.fn(),
	lockedStatus: undefined as ILockedStorageStatus | undefined,
	lockedListing: undefined as IListLockedItemsResponse | undefined,
	invokeArgs: new Map<string, unknown[]>(),
	lockedRows: [] as ILockedItem[],
	backend: {
		lockedStorageState: {},
		storageState: {
			listStorageItems: vi.fn(),
			listStorageItemsUser: vi.fn(),
			deleteStorageItems: vi.fn(),
			renameStorageItem: vi.fn(),
			downloadStorageItems: vi.fn(),
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
		setQueryData: fixture.setQueryData,
		getQueryCache: () => ({
			findAll: ({
				predicate,
			}: { predicate: (query: { queryKey: unknown[] }) => boolean }) =>
				fixture.cachedQueries.filter(predicate),
		}),
	}),
	useAppPermissions: () => ({ can: () => true, canStrict: () => true }),
	useBackendReady: () => true,
	useInvoke: (
		fn: (...args: unknown[]) => Promise<unknown>,
		context: unknown,
		args: unknown[],
	) => {
		fixture.invokeArgs.set(fn.name, args);
		if (fn.name === "lockedStorageStatus")
			return { data: fixture.lockedStatus };
		if (fn.name === "listLockedFolder") return { data: fixture.lockedListing };
		if (fn.name === "lockedItemDetail") return { data: undefined };
		fixture.invoke = () => fn.apply(context, args);
		return { data: fixture.items, isSuccess: true, refetch: fixture.refetch };
	},
}));

vi.mock("./locked-storage-dialogs", () => ({ LockedStorageLayer: () => null }));

vi.mock("./locked-file-row", () => ({
	LockedFileRow: ({ item }: { item: ILockedItem }) => {
		fixture.lockedRows.push(item);
		return <div data-locked-row={item.name} />;
	},
}));

vi.mock("../ui/monaco-file-editor", () => ({ MonacoFileEditor: () => null }));
vi.mock("../ui/text-editor", () => ({ TextEditor: () => null }));

vi.mock("../ui", async () => {
	const { canPreview, isPdf, isImage, isAudio, isVideo } = await import(
		"../ui/file-previewer"
	);
	const Wrapper = ({ children }: { children?: ReactNode }) => <>{children}</>;
	return {
		Badge: Wrapper,
		Button: ({ children, ...props }: ComponentProps<"button">) => (
			<button type="button" {...props}>
				{children}
			</button>
		),
		Dialog: ({
			children,
			onOpenChange,
		}: { children: ReactNode; onOpenChange: (open: boolean) => void }) => {
			fixture.closeDialog = () => onOpenChange(false);
			return <>{children}</>;
		},
		DialogContent: Wrapper,
		DialogTitle: Wrapper,
		DropdownMenu: Wrapper,
		DropdownMenuContent: () => null,
		DropdownMenuItem: Wrapper,
		DropdownMenuTrigger: Wrapper,
		EmptyState: () => <p>No Files Found</p>,
		FilePreviewer: ({ url, filename }: { url: string; filename: string }) => (
			<div data-inline-preview={filename} data-preview-url={url} />
		),
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
		canPreview,
		isPdf,
		isImage,
		isAudio,
		isVideo,
	};
});

vi.mock("./storage-breadcrumbs", () => ({
	StorageBreadcrumbs: ({
		updatePrefix,
	}: { updatePrefix: (prefix: string) => void }) => {
		fixture.changePrefix = updatePrefix;
		return null;
	},
}));

vi.mock("./wordcraft-document", () => ({
	WordcraftDocument: (props: DocxProps) => {
		fixture.docxProps = props;
		fixture.docxRender(props);
		return <div data-docx-document={props.filename} />;
	},
}));

vi.mock("./craft-document", () => ({
	CraftDocument: (props: CraftProps) => {
		fixture.craftProps = props;
		fixture.craftRender(props);
		return <div data-craft-document={props.filename} />;
	},
}));

vi.mock("./storage-file-or-folder", () => ({
	FileOrFolder: ({
		file,
		deleteFile,
		loadFile,
		renameFile,
		changePrefix,
		previewable,
	}: {
		file: IStorageItem;
		deleteFile: (target: string) => Promise<void>;
		loadFile: (target: string) => Promise<void>;
		renameFile?: (file: IStorageItem) => void;
		changePrefix?: (prefix: string) => void;
		previewable?: boolean;
	}) => {
		fixture.deleteFile = deleteFile;
		fixture.loadFile = loadFile;
		fixture.previewable.set(file.location, previewable);
		fixture.renderedLocations.push(file.location);
		return (
			<div data-storage-location={file.location} data-folder={file.is_dir}>
				{storageDisplayName(file.location)}
				{renameFile && (
					<button
						type="button"
						aria-label={`Rename ${storageDisplayName(file.location)}`}
						onClick={() => renameFile(file)}
					/>
				)}
				{file.is_dir && (
					<button
						type="button"
						aria-label={`Open ${storageDisplayName(file.location)}`}
						onClick={() => changePrefix?.(file.location.split("/").pop() ?? "")}
					/>
				)}
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

function docxSave() {
	const save = fixture.docxProps?.onSave;
	if (!save) throw new Error("WordcraftDocument rendered without onSave");
	return save;
}

function craftSave() {
	const save = fixture.craftProps?.onSave;
	if (!save) throw new Error("CraftDocument rendered without onSave");
	return save;
}

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	vi.stubGlobal("confirm", vi.fn().mockReturnValue(true));
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
	fixture.setQueryData.mockReset();
	fixture.cachedQueries = [];
	fixture.invoke = undefined;
	fixture.deleteFile = undefined;
	fixture.loadFile = undefined;
	fixture.changePrefix = undefined;
	fixture.closeDialog = undefined;
	fixture.docxProps = undefined;
	fixture.craftProps = undefined;
	fixture.craftRender.mockReset();
	fixture.docxRender.mockReset();
	fixture.previewable.clear();
	fixture.updatePrefix.mockReset();
	fixture.lockedStatus = undefined;
	fixture.lockedListing = undefined;
	fixture.invokeArgs.clear();
	fixture.lockedRows = [];
	fixture.backend.storageState.listStorageItems.mockReset();
	fixture.backend.storageState.listStorageItems.mockResolvedValue([]);
	fixture.backend.storageState.listStorageItemsUser.mockReset();
	fixture.backend.storageState.listStorageItemsUser.mockResolvedValue([]);
	fixture.backend.storageState.deleteStorageItems.mockReset();
	fixture.backend.storageState.deleteStorageItems.mockResolvedValue(undefined);
	fixture.backend.storageState.renameStorageItem.mockReset();
	fixture.backend.storageState.renameStorageItem.mockResolvedValue(undefined);
	fixture.backend.storageState.downloadStorageItems.mockReset();
	fixture.backend.storageState.downloadStorageItems.mockImplementation(
		async (_appId: string, paths: string[]) =>
			paths.map((prefix) => ({
				prefix,
				url: `https://storage.example/${prefix}`,
			})),
	);
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
	vi.unstubAllGlobals();
});

async function renderStorage(
	storageScopeKey = "shared",
	prefix = "",
	props: Partial<ComponentProps<typeof StorageSystem>> = {},
) {
	await act(async () => {
		root.render(
			<StorageSystem
				appId="app-a"
				prefix={prefix}
				storageScopeKey={storageScopeKey}
				updatePrefix={fixture.updatePrefix}
				fileToUrl={async () => ""}
				{...props}
			/>,
		);
	});
}

async function openRename(name: string) {
	const button = container.querySelector<HTMLButtonElement>(
		`button[aria-label="Rename ${name}"]`,
	);
	expect(button).not.toBeNull();
	await act(async () => button?.click());
}

async function enterRename(name: string) {
	const input = container.querySelector<HTMLInputElement>("form input");
	expect(input).not.toBeNull();
	await act(async () => {
		Object.getOwnPropertyDescriptor(
			HTMLInputElement.prototype,
			"value",
		)?.set?.call(input, name);
		input?.dispatchEvent(new Event("input", { bubbles: true }));
	});
}

async function submitRename() {
	await act(async () => {
		container
			.querySelector("form")
			?.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
	});
}

describe("Storage rename", () => {
	it("renames encoded cloud files relative to the listed folder and closes their preview", async () => {
		fixture.items = [
			item("apps/app-a/upload/docs/%C3%9Cbersicht%231.pdf", false),
		];
		await renderStorage("shared", "docs");
		await act(async () => {
			await fixture.loadFile?.(fixture.items[0].location);
		});
		await openRename("Übersicht#1.pdf");
		expect(container.querySelector<HTMLInputElement>("form input")?.value).toBe(
			"Übersicht#1.pdf",
		);
		await enterRename("Report%2Ffinal.pdf");
		await submitRename();
		expect(fixture.backend.storageState.renameStorageItem).toHaveBeenCalledWith(
			"app-a",
			"docs/%C3%9Cbersicht%231.pdf",
			"Report%2Ffinal.pdf",
			false,
		);
		expect(fixture.backend.storageState.listStorageItems).toHaveBeenCalledWith(
			"app-a",
			"docs",
			{ refresh: true },
		);
		expect(container.querySelector("form")).toBeNull();
		expect(container.querySelector("[data-inline-preview]")).toBeNull();
	});

	it("uses user storage operations and invalidates only that scope's cached paths", async () => {
		fixture.items = [item("users/user-a/apps/app-a/docs/reports", true)];
		const rename = vi.fn().mockResolvedValue(undefined);
		const userTree = [
			fixture.backend.storageState.listStorageItemsUser.name,
			"app-a",
			"docs",
		];
		const oldTree = [
			fixture.backend.storageState.listStorageItemsUser.name,
			"app-a",
			"docs/reports/sub",
		];
		const sharedBrowser = ["listStorageItems", "app-a", "docs", "shared"];
		fixture.cachedQueries = [
			{ queryKey: userTree },
			{ queryKey: oldTree },
			{ queryKey: sharedBrowser },
			{ queryKey: ["listStorageItems", "other-app", "docs", "user"] },
		];
		await renderStorage("user", "docs", {
			operations: {
				listStorageItems: fixture.backend.storageState.listStorageItemsUser,
				renameStorageItem: rename,
				deleteStorageItems: vi.fn(),
				downloadStorageItems: vi.fn(),
				uploadStorageItems: vi.fn(),
			},
		});
		await openRename("reports");
		await enterRename("archive");
		await submitRename();
		expect(rename).toHaveBeenCalledWith(
			"app-a",
			"docs/reports",
			"archive",
			true,
		);
		expect(
			fixture.backend.storageState.renameStorageItem,
		).not.toHaveBeenCalled();
		expect(fixture.cancelQueries).toHaveBeenCalledWith({
			queryKey: userTree,
			exact: true,
		});
		expect(fixture.setQueryData).toHaveBeenCalledWith(oldTree, []);
		expect(fixture.cancelQueries).not.toHaveBeenCalledWith({
			queryKey: sharedBrowser,
			exact: true,
		});
		expect(
			fixture.backend.storageState.listStorageItemsUser,
		).toHaveBeenCalledWith("app-a", "docs", { refresh: true });
	});

	it("renames a virtual empty folder and its nested placeholders without a backend move", async () => {
		sessionStorage.setItem(
			"vfolders:shared:app-a",
			JSON.stringify({
				docs: ["drafts"],
				"docs/drafts": ["empty"],
				"docs/drafts/empty": ["nested"],
				"docs/drafts-other": ["keep"],
			}),
		);
		await renderStorage("shared", "docs");
		await openRename("drafts");
		await enterRename("archive");
		await submitRename();
		expect(
			fixture.backend.storageState.renameStorageItem,
		).not.toHaveBeenCalled();
		expect(
			JSON.parse(sessionStorage.getItem("vfolders:shared:app-a") ?? "{}"),
		).toEqual({
			docs: ["archive"],
			"docs/archive": ["empty"],
			"docs/archive/empty": ["nested"],
			"docs/drafts-other": ["keep"],
		});
		expect(
			container.querySelector('[data-storage-location="docs/archive"]'),
		).not.toBeNull();
		expect(
			container.querySelector('[data-storage-location="docs/drafts"]'),
		).toBeNull();
	});

	it("moves nested placeholders to the backend's encoded destination path", async () => {
		fixture.items = [item("apps/app-a/upload/docs/reports")];
		fixture.backend.storageState.listStorageItems.mockResolvedValue([
			item("apps/app-a/upload/docs/%C3%9Cbersicht%23final"),
		]);
		sessionStorage.setItem(
			"vfolders:shared:app-a",
			JSON.stringify({ "docs/reports": ["empty"] }),
		);
		await renderStorage("shared", "docs");
		await openRename("reports");
		await enterRename("Übersicht#final");
		await submitRename();
		expect(
			JSON.parse(sessionStorage.getItem("vfolders:shared:app-a") ?? "{}"),
		).toEqual({
			"docs/%C3%9Cbersicht%23final": ["empty"],
		});
	});

	it("respects unsaved document changes before renaming", async () => {
		fixture.items = [item("report.docx", false)];
		await renderStorage("shared", "", { enableDocx: true });
		await act(async () => {
			await fixture.loadFile?.("report.docx");
		});
		fixture.docxProps?.onDirtyChange?.(true);
		vi.spyOn(window, "confirm").mockReturnValue(false);
		await openRename("report.docx");
		await enterRename("archive.docx");
		await submitRename();
		expect(window.confirm).toHaveBeenCalledWith(
			"Discard unsaved document changes?",
		);
		expect(
			fixture.backend.storageState.renameStorageItem,
		).not.toHaveBeenCalled();
		expect(
			container.querySelector('[data-docx-document="report.docx"]'),
		).not.toBeNull();
	});

	it("does not display a stale preview that finishes loading after rename", async () => {
		fixture.items = [item("report.pdf", false)];
		let finish = (_items: { prefix: string; url: string }[]) => {};
		fixture.backend.storageState.downloadStorageItems.mockImplementation(
			() =>
				new Promise((resolve) => {
					finish = resolve;
				}),
		);
		await renderStorage();
		let loading: Promise<void> | undefined;
		await act(async () => {
			loading = fixture.loadFile?.("report.pdf");
		});
		await openRename("report.pdf");
		await enterRename("archive.pdf");
		await submitRename();
		await act(async () => {
			finish([
				{ prefix: "report.pdf", url: "https://storage.example/report.pdf" },
			]);
			await loading;
		});
		expect(container.querySelector("[data-inline-preview]")).toBeNull();
	});

	it.each(["", "  ", ".", "..", "nested/name", "nested\\name"])(
		"rejects invalid name %j without calling storage",
		async (name) => {
			fixture.items = [item("report.txt", false)];
			await renderStorage();
			await openRename("report.txt");
			await enterRename(name);
			await submitRename();
			expect(container.querySelector('[role="alert"]')).not.toBeNull();
			expect(
				fixture.backend.storageState.renameStorageItem,
			).not.toHaveBeenCalled();
		},
	);

	it("rejects collisions with existing files and virtual folders", async () => {
		fixture.items = [item("report.txt", false), item("taken", false)];
		sessionStorage.setItem(
			"vfolders:shared:app-a",
			JSON.stringify({ "": ["empty"] }),
		);
		await renderStorage();
		await openRename("report.txt");
		for (const name of ["taken", "empty"]) {
			await enterRename(name);
			await submitRename();
			expect(container.querySelector('[role="alert"]')?.textContent).toContain(
				"already exists",
			);
		}
		expect(
			fixture.backend.storageState.renameStorageItem,
		).not.toHaveBeenCalled();
	});

	it("keeps the original file and preview after an error and allows retry", async () => {
		fixture.items = [item("report.pdf", false)];
		fixture.backend.storageState.renameStorageItem.mockRejectedValueOnce(
			new Error("Destination already exists"),
		);
		await renderStorage();
		await act(async () => {
			await fixture.loadFile?.("report.pdf");
		});
		await openRename("report.pdf");
		await enterRename("archive.pdf");
		await submitRename();
		expect(container.querySelector('[role="alert"]')?.textContent).toBe(
			"Destination already exists",
		);
		expect(
			container.querySelector('[data-inline-preview="report.pdf"]'),
		).not.toBeNull();
		expect(
			container.querySelector('[data-storage-location="report.pdf"]'),
		).not.toBeNull();
		expect(
			container.querySelector<HTMLInputElement>("form input")?.disabled,
		).toBe(false);
		expect(fixture.backend.storageState.listStorageItems).toHaveBeenCalledWith(
			"app-a",
			"",
			{ refresh: true },
		);
		await submitRename();
		expect(
			fixture.backend.storageState.renameStorageItem,
		).toHaveBeenCalledTimes(2);
		expect(container.querySelector("form")).toBeNull();
	});

	it("refreshes partial folder destinations after failure without clearing source descendants", async () => {
		fixture.items = [item("docs/reports")];
		const sourceTree = [
			fixture.backend.storageState.listStorageItems.name,
			"app-a",
			"docs/reports/sub",
		];
		fixture.cachedQueries = [{ queryKey: sourceTree }];
		const failure =
			"Rename incomplete. Some files were copied to archive; review both folders before retrying.";
		fixture.backend.storageState.renameStorageItem.mockRejectedValueOnce(
			new Error(failure),
		);
		fixture.backend.storageState.listStorageItems.mockImplementation(
			async () => {
				fixture.items = [item("docs/reports"), item("docs/archive")];
				return fixture.items;
			},
		);
		await renderStorage("shared", "docs");
		await openRename("reports");
		await enterRename("archive");
		await submitRename();
		expect(container.querySelector('[role="alert"]')?.textContent).toBe(
			failure,
		);
		expect(
			container.querySelector('[data-storage-location="docs/reports"]'),
		).not.toBeNull();
		expect(
			container.querySelector('[data-storage-location="docs/archive"]'),
		).not.toBeNull();
		expect(container.querySelector<HTMLInputElement>("form input")?.value).toBe(
			"archive",
		);
		expect(fixture.backend.storageState.listStorageItems).toHaveBeenCalledWith(
			"app-a",
			"docs",
			{ refresh: true },
		);
		expect(fixture.setQueryData).not.toHaveBeenCalled();
		expect(fixture.cancelQueries).not.toHaveBeenCalledWith({
			queryKey: sourceTree,
			exact: true,
		});
	});

	it("retains the rename error when the recovery listing also fails", async () => {
		fixture.items = [item("report.txt", false)];
		fixture.backend.storageState.renameStorageItem.mockRejectedValueOnce(
			"Rename incomplete; inspect destination files",
		);
		fixture.backend.storageState.listStorageItems.mockRejectedValueOnce(
			new Error("Listing unavailable"),
		);
		vi.spyOn(console, "error").mockImplementation(() => {});
		await renderStorage();
		await openRename("report.txt");
		await enterRename("archive.txt");
		await submitRename();
		expect(container.querySelector('[role="alert"]')?.textContent).toBe(
			"Rename incomplete; inspect destination files",
		);
		expect(
			container.querySelector<HTMLInputElement>("form input")?.disabled,
		).toBe(false);
		expect(fixture.backend.storageState.listStorageItems).toHaveBeenCalledWith(
			"app-a",
			"",
			{ refresh: true },
		);
	});

	it("prevents duplicate submissions and dismissal while a rename is pending", async () => {
		fixture.items = [item("report.txt", false)];
		let finish = () => {};
		fixture.backend.storageState.renameStorageItem.mockImplementation(
			() =>
				new Promise<void>((resolve) => {
					finish = resolve;
				}),
		);
		await renderStorage();
		await openRename("report.txt");
		await enterRename("archive.txt");
		await submitRename();
		expect(
			container.querySelector<HTMLInputElement>("form input")?.disabled,
		).toBe(true);
		expect(container.querySelector('button[type="submit"]')?.textContent).toBe(
			"Renaming...",
		);
		await submitRename();
		await act(async () => fixture.closeDialog?.());
		expect(container.querySelector("form")).not.toBeNull();
		expect(
			fixture.backend.storageState.renameStorageItem,
		).toHaveBeenCalledTimes(1);
		await act(async () => finish());
		expect(container.querySelector("form")).toBeNull();
	});

	it("hides rename without write permission and discards dialogs when storage scope changes", async () => {
		fixture.items = [item("report.txt", false), item("folder")];
		await renderStorage("shared", "", { canWrite: false });
		expect(container.querySelector('button[aria-label^="Rename "]')).toBeNull();
		await renderStorage();
		await openRename("report.txt");
		await renderStorage("user");
		expect(container.querySelector("form")).toBeNull();
		await renderStorage();
		expect(container.querySelector("form")).toBeNull();
		expect(
			fixture.backend.storageState.renameStorageItem,
		).not.toHaveBeenCalled();
	});
});

describe("Storage DOCX documents", () => {
	it("enables only DOCX previews on opted-in storage pages and loads the editor on selection", async () => {
		fixture.items = [
			item("docs/report.DOCX", false),
			item("docs/legacy.doc", false),
		];
		await renderStorage("shared", "docs");
		expect(fixture.previewable.get("docs/report.DOCX")).toBeUndefined();
		expect(fixture.docxRender).not.toHaveBeenCalled();
		await renderStorage("shared", "docs", { enableDocx: true });
		expect(fixture.previewable.get("docs/report.DOCX")).toBe(true);
		expect(fixture.previewable.get("docs/legacy.doc")).toBeUndefined();
		expect(fixture.docxRender).not.toHaveBeenCalled();
		await act(async () => {
			await fixture.loadFile?.("docs/report.DOCX");
		});
		expect(fixture.docxProps?.filename).toBe("report.DOCX");
		expect(fixture.docxProps?.url).toBe(
			"https://storage.example/docs/report.DOCX",
		);
	});

	it("overwrites the selected DOCX in its source folder and refreshes storage", async () => {
		fixture.items = [item("apps/app-a/upload/docs/report.docx", false)];
		await renderStorage("shared", "docs", { enableDocx: true });
		await act(async () => {
			await fixture.loadFile?.(fixture.items[0].location);
		});
		const exported = new File(
			[new Uint8Array([0x50, 0x4b, 0, 0xff])],
			"document.docx",
		);
		const onProgress = vi.fn();
		fixture.backend.storageState.uploadStorageItems.mockImplementationOnce(
			async (_appId, _directory, _files, reportProgress) => {
				reportProgress?.(42);
			},
		);
		await act(async () => {
			await docxSave()(exported, onProgress);
		});
		expect(onProgress).toHaveBeenCalledWith(42);
		const [appId, directory, savedFiles] =
			fixture.backend.storageState.uploadStorageItems.mock.calls[0];
		expect(appId).toBe("app-a");
		expect(directory).toBe("docs");
		expect(savedFiles[0].name).toBe("report.docx");
		expect(savedFiles[0].type).toBe(
			"application/vnd.openxmlformats-officedocument.wordprocessingml.document",
		);
		expect(new Uint8Array(await savedFiles[0].arrayBuffer())).toEqual(
			new Uint8Array(await exported.arrayBuffer()),
		);
		expect(
			fixture.backend.storageState.listStorageItems,
		).toHaveBeenLastCalledWith("app-a", "docs", { refresh: true });
	});

	it.each([
		["docs", "apps/app-a/upload/docs/report%2Fpart.docx", "report%2Fpart.docx"],
		["docs", "docs/literal%252Fname.docx", "literal%252Fname.docx"],
		[
			"Q1%20draft",
			"Q1%20draft/%C3%9Cbersicht%231.DOCX",
			"%C3%9Cbersicht%231.DOCX",
		],
		["docs", "docs/100%25.docx", "100%25.docx"],
		["", "REPORT.DOCX", "REPORT.DOCX"],
	])(
		"preserves the exact storage address when saving %s / %s",
		async (prefix, location, name) => {
			fixture.items = [item(location, false)];
			await renderStorage("shared", prefix, { enableDocx: true });
			await act(async () => {
				await fixture.loadFile?.(location);
			});
			const exported = new File(["updated"], "document.docx");
			await act(async () => {
				await docxSave()(exported);
				await docxSave()(exported);
			});
			expect(
				fixture.backend.storageState.uploadStorageItems,
			).toHaveBeenCalledTimes(2);
			for (const [appId, directory, savedFiles] of fixture.backend.storageState
				.uploadStorageItems.mock.calls) {
				expect(appId).toBe("app-a");
				expect(directory).toBe(prefix);
				expect(savedFiles).toHaveLength(1);
				expect(savedFiles[0].name).toBe(name);
				expect(buildUploadPath(directory, savedFiles[0])).toBe(
					prefix ? `${prefix}/${name}` : name,
				);
			}
		},
	);

	it("keeps read-only DOCX files viewable and prevents their save callback from uploading", async () => {
		fixture.items = [item("report.docx", false)];
		await renderStorage("shared", "", { enableDocx: true, canWrite: false });
		await act(async () => {
			await fixture.loadFile?.("report.docx");
		});
		expect(fixture.docxProps?.editable).toBe(false);
		await expect(
			docxSave()(new File(["content"], "report.docx")),
		).rejects.toThrow("permission");
		expect(
			fixture.backend.storageState.uploadStorageItems,
		).not.toHaveBeenCalled();
	});

	it("propagates failed saves and leaves the dirty document open", async () => {
		fixture.items = [item("report.docx", false)];
		await renderStorage("shared", "", { enableDocx: true });
		await act(async () => {
			await fixture.loadFile?.("report.docx");
		});
		fixture.docxProps?.onDirtyChange?.(true);
		fixture.backend.storageState.uploadStorageItems.mockRejectedValueOnce(
			new Error("Upload failed"),
		);
		await expect(
			docxSave()(new File(["content"], "report.docx")),
		).rejects.toThrow("Upload failed");
		expect(container.querySelector("[data-docx-document]")).not.toBeNull();
		const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
		await act(async () => {
			fixture.closeDialog?.();
		});
		expect(confirm).toHaveBeenCalledOnce();
		expect(
			fixture.backend.storageState.listStorageItems,
		).toHaveBeenLastCalledWith("app-a", "", { refresh: true });
	});

	it("preserves the upload error when refreshing storage also fails", async () => {
		fixture.items = [item("report.docx", false)];
		await renderStorage("shared", "", { enableDocx: true });
		await act(async () => {
			await fixture.loadFile?.("report.docx");
		});
		fixture.backend.storageState.uploadStorageItems.mockRejectedValueOnce(
			new Error("Upload failed"),
		);
		fixture.backend.storageState.listStorageItems.mockRejectedValueOnce(
			new Error("Refresh failed"),
		);
		const logError = vi.spyOn(console, "error").mockImplementation(() => {});
		await expect(
			docxSave()(new File(["content"], "report.docx")),
		).rejects.toThrow("Upload failed");
		expect(logError).toHaveBeenCalledWith(
			"Failed to refresh storage:",
			expect.objectContaining({ message: "Refresh failed" }),
		);
	});

	it("retains the dirty editor when a refreshed listing becomes empty", async () => {
		fixture.items = [item("report.docx", false)];
		await renderStorage("shared", "", { enableDocx: true });
		await act(async () => {
			await fixture.loadFile?.("report.docx");
		});
		fixture.docxProps?.onDirtyChange?.(true);
		const documentNode = container.querySelector("[data-docx-document]");
		fixture.items = [];
		await renderStorage("shared", "", { enableDocx: true });
		expect(container.querySelector("[data-docx-document]")).toBe(documentNode);
		const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
		await act(async () => {
			fixture.closeDialog?.();
		});
		expect(confirm).toHaveBeenCalledOnce();
	});

	it("retains a dirty editor when close, folder navigation or another file is declined", async () => {
		fixture.items = [item("report.docx", false), item("second.docx", false)];
		await renderStorage("shared", "", { enableDocx: true });
		await act(async () => {
			await fixture.loadFile?.("report.docx");
		});
		fixture.docxProps?.onDirtyChange?.(true);
		const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
		await act(async () => {
			fixture.closeDialog?.();
			fixture.changePrefix?.("elsewhere");
			await fixture.loadFile?.("second.docx");
		});
		expect(confirm).toHaveBeenCalledTimes(3);
		expect(fixture.updatePrefix).not.toHaveBeenCalled();
		expect(
			container
				.querySelector("[data-docx-document]")
				?.getAttribute("data-docx-document"),
		).toBe("report.docx");
		expect(
			fixture.backend.storageState.downloadStorageItems,
		).toHaveBeenCalledOnce();
		confirm.mockReturnValue(true);
		await act(async () => {
			fixture.closeDialog?.();
		});
		expect(container.querySelector("[data-docx-document]")).toBeNull();
	});

	it("blocks unload and same-tab links while dirty, and prevents closing during save", async () => {
		fixture.items = [item("report.docx", false)];
		await renderStorage("shared", "", { enableDocx: true });
		await act(async () => {
			await fixture.loadFile?.("report.docx");
		});
		fixture.docxProps?.onDirtyChange?.(true);
		const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
		const unload = new Event("beforeunload", { cancelable: true });
		window.dispatchEvent(unload);
		expect(unload.defaultPrevented).toBe(true);
		const anchor = document.createElement("a");
		anchor.href = "/library/config/flows";
		container.append(anchor);
		const click = new MouseEvent("click", { bubbles: true, cancelable: true });
		anchor.dispatchEvent(click);
		expect(click.defaultPrevented).toBe(true);
		fixture.docxProps?.onDirtyChange?.(false);
		fixture.docxProps?.onBusyChange?.(true);
		confirm.mockClear();
		await act(async () => {
			fixture.closeDialog?.();
			window.history.back();
		});
		expect(confirm).not.toHaveBeenCalled();
		expect(container.querySelector("[data-docx-document]")).not.toBeNull();
	});

	it("keeps browser Back on the storage page until dirty changes are discarded", async () => {
		fixture.items = [item("report.docx", false)];
		const originalUrl = window.location.href;
		const originalState = window.history.state;
		await renderStorage("shared", "", { enableDocx: true });
		await act(async () => {
			await fixture.loadFile?.("report.docx");
		});
		fixture.docxProps?.onDirtyChange?.(true);
		const documentNode = container.querySelector("[data-docx-document]");
		const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
		await act(async () => {
			window.history.back();
		});
		expect(confirm).toHaveBeenCalledOnce();
		expect(container.querySelector("[data-docx-document]")).toBe(documentNode);
		expect(window.location.href).toBe(originalUrl);
		confirm.mockReturnValue(true);
		await act(async () => {
			window.history.back();
		});
		expect(container.querySelector("[data-docx-document]")).toBeNull();
		expect(window.history.state).toEqual(originalState);
		expect(window.location.href).toBe(originalUrl);
	});
});

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

describe("Storage Craft editors", () => {
	it.each([
		["user", "app-a"],
		["shared", "app-b"],
	])(
		"removes the agent target when scope changes to %s/%s",
		async (scope, appId) => {
			const location = "docs/part.solidcraft";
			fixture.items = [item(location, false)];
			await renderStorage("shared", "docs", { enableCraftEditors: true });
			await act(async () => {
				await fixture.loadFile?.(location);
			});
			expect(fixture.craftProps?.toolTarget).toEqual({
				appId: "app-a",
				surfaceId: "storage:shared",
				componentId: location,
			});
			await renderStorage(scope, "docs", { enableCraftEditors: true, appId });
			expect(fixture.craftProps?.toolTarget).toBeUndefined();
		},
	);

	it.each([
		["pdf", "pdfcraft"],
		["pptx", "deckcraft"],
		["xlsx", "gridcraft"],
		["svg", "vectorcraft"],
		["png", "photocraft"],
		["wav", "soundcraft"],
		["mp4", "filmcraft"],
		["dxf", "cadcraft"],
		["solidcraft", "solidcraft"],
	])(
		"lazily opens .%s using %s and saves the exact selected object",
		async (extension, id) => {
			const location = `docs/name%2Fpart.${extension}`;
			fixture.items = [item(location, false)];
			await renderStorage("shared", "docs", { enableCraftEditors: true });
			expect(fixture.craftRender).not.toHaveBeenCalled();
			expect(fixture.previewable.get(location)).toBe(true);
			await act(async () => {
				await fixture.loadFile?.(location);
			});
			if (["pdf", "svg", "png", "wav", "mp4"].includes(extension)) {
				expect(fixture.craftRender).not.toHaveBeenCalled();
				await act(async () => {
					container
						.querySelector<HTMLButtonElement>('[aria-label^="Edit "]')
						?.click();
				});
				expect(fixture.craftProps?.initialEditing).toBe(true);
			}
			expect(fixture.craftProps?.viewer.id).toBe(id);
			expect(fixture.craftProps?.toolTarget).toEqual({
				appId: "app-a",
				surfaceId: "storage:shared",
				componentId: location,
			});
			const onProgress = vi.fn();
			await act(async () => {
				await craftSave()(new File(["bytes"], "ignored.bin"), onProgress);
			});
			const [appId, directory, files, reportProgress] =
				fixture.backend.storageState.uploadStorageItems.mock.calls[0];
			expect(appId).toBe("app-a");
			expect(directory).toBe("docs");
			expect(files[0].name).toBe(`name%2Fpart.${extension}`);
			expect(reportProgress).toBe(onProgress);
		},
	);
	it.each(
		[
			"pdf",
			"png",
			"svg",
			"apng",
			"jpe",
			"jfif",
			"wav",
			"wave",
			"aiff",
			"mp4",
			"m4v",
			"mkv",
			"avi",
		].flatMap((extension) =>
			[false, true].map((canWrite) => ({ extension, canWrite })),
		),
	)(
		"keeps .$extension reading inline without editor history (canWrite=$canWrite)",
		async ({ extension, canWrite }) => {
			const filename = `report.${extension}`;
			fixture.items = [item(`docs/${filename}`, false)];
			await renderStorage("shared", "docs", {
				enableCraftEditors: true,
				canWrite,
			});
			const push = vi.spyOn(window.history, "pushState");
			await act(async () => {
				await fixture.loadFile?.(`docs/${filename}`);
			});
			expect(
				container
					.querySelector("[data-inline-preview]")
					?.getAttribute("data-inline-preview"),
			).toBe(filename);
			expect(fixture.closeDialog).toBeUndefined();
			expect(fixture.craftRender).not.toHaveBeenCalled();
			expect(push).not.toHaveBeenCalled();
			expect(!!container.querySelector('[aria-label^="Edit "]')).toBe(canWrite);
		},
	);
	it.each([
		["xls", "gridcraft", "xlsx"],
		["step", "solidcraft", "solidcraft"],
		["glb", "solidcraft", "solidcraft"],
	])(
		"uses one distinct conversion target without replacing the .%s source",
		async (extension, id, outputExtension) => {
			const location = `docs/source.${extension}`;
			fixture.items = [item(location, false)];
			await renderStorage("shared", "docs", { enableCraftEditors: true });
			expect(fixture.previewable.get(location)).toBe(true);
			await act(async () => {
				await fixture.loadFile?.(location);
			});
			expect(fixture.craftProps?.viewer.id).toBe(id);
			const outputName = fixture.craftProps?.outputFilename;
			await act(async () => {
				await craftSave()(new File(["bytes"], "untrusted.bin"));
				await craftSave()(new File(["bytes"], "untrusted.bin"));
			});
			const uploads =
				fixture.backend.storageState.uploadStorageItems.mock.calls;
			expect(uploads[0][1]).toBe("docs");
			expect(uploads[0][2][0].name).toMatch(
				new RegExp(`^source-converted-.+\\.${outputExtension}$`),
			);
			expect(uploads[1][2][0].name).toBe(uploads[0][2][0].name);
			expect(uploads[0][2][0].name).toBe(outputName);
		},
	);
	it("enforces write permissions and dirty navigation for all editors", async () => {
		fixture.items = [item("drawing.vectorcraft", false)];
		await renderStorage("shared", "", {
			enableCraftEditors: true,
			canWrite: false,
		});
		await act(async () => {
			await fixture.loadFile?.("drawing.vectorcraft");
		});
		expect(fixture.craftProps?.editable).toBe(false);
		await expect(
			craftSave()(new File(["bytes"], "drawing.vectorcraft")),
		).rejects.toThrow("permission");
		fixture.craftProps?.onDirtyChange?.(true);
		vi.spyOn(window, "confirm").mockReturnValue(false);
		await act(async () => {
			fixture.closeDialog?.();
		});
		expect(container.querySelector("[data-craft-document]")).not.toBeNull();
	});
});

const LOCKED_STATUS: ILockedStorageStatus = {
	enabled: true,
	provider: "s3",
	enforcement: "object",
	retention_mode: "compliance",
	default_retention_days: 3650,
	min_retention_days: 1,
	max_retention_days: 36500,
	max_file_bytes: 5_368_709_120,
	supports_legal_hold: true,
	supports_retention_extension: true,
};

const lockedItem = (name: string, folder: string): ILockedItem => ({
	id: `item-${name}`,
	app_id: "app-a",
	scope: "project",
	owner_id: null,
	name,
	folder,
	content_type: "application/pdf",
	latest_version: 2,
	latest_size: 10,
	live_versions: 2,
	live_size: 20,
	metadata: null,
	created_by: "user-a",
	created_at: "2026-10-01T00:00:00Z",
	updated_at: "2026-10-02T00:00:00Z",
	latest: null,
});

describe("Storage locked files", () => {
	it("stays a plain storage browser while locked storage is disabled", async () => {
		fixture.items = [item("apps/app-a/upload/docs/a.pdf", false)];
		await renderStorage("shared", "docs");
		expect(fixture.invokeArgs.get("lockedStorageStatus")).toEqual(["app-a"]);
		expect(container.querySelector("[data-locked-row]")).toBeNull();
		expect(
			[...container.querySelectorAll("[data-storage-location]")].map((node) =>
				node.getAttribute("data-storage-location"),
			),
		).toEqual(["apps/app-a/upload/docs/a.pdf"]);
	});

	it("lists the decoded folder and merges locked rows and folders by display name", async () => {
		const prefix = "Akten/%C3%9Cbersicht";
		fixture.lockedStatus = LOCKED_STATUS;
		fixture.items = [
			item(`apps/app-a/upload/${prefix}/%C3%9Cbersicht%20alt`, true),
			item(`apps/app-a/upload/${prefix}/notes.txt`, false),
		];
		fixture.lockedListing = {
			items: [lockedItem("Übersicht (2)#1.pdf", "Akten/Übersicht")],
			folders: ["Übersicht alt", "Verträge"],
			next_cursor: null,
		};
		await renderStorage("user", prefix);

		expect(fixture.invokeArgs.get("listLockedFolder")).toEqual([
			"app-a",
			"user",
			"Akten/Übersicht",
		]);
		const folders = [...container.querySelectorAll('[data-folder="true"]')].map(
			(node) => node.getAttribute("data-storage-location"),
		);
		expect(folders).toEqual([
			`apps/app-a/upload/${prefix}/%C3%9Cbersicht%20alt`,
			`${prefix}/Vertr%C3%A4ge`,
		]);
		expect(
			[...container.querySelectorAll("[data-locked-row]")].map((node) =>
				node.getAttribute("data-locked-row"),
			),
		).toEqual(["Übersicht (2)#1.pdf"]);

		await act(async () =>
			container
				.querySelector<HTMLButtonElement>('button[aria-label="Open Verträge"]')
				?.click(),
		);
		expect(fixture.updatePrefix).toHaveBeenCalledWith(
			`${prefix}/Vertr%C3%A4ge`,
		);
	});

	it("filters locked rows with the search box", async () => {
		fixture.lockedStatus = LOCKED_STATUS;
		fixture.items = [item("apps/app-a/upload/a.txt", false)];
		fixture.lockedListing = {
			items: [lockedItem("contract.pdf", ""), lockedItem("invoice.pdf", "")],
			folders: [],
			next_cursor: null,
		};
		await renderStorage("shared", "");
		expect(fixture.invokeArgs.get("listLockedFolder")).toEqual([
			"app-a",
			"project",
			"",
		]);
		const search = container.querySelector<HTMLInputElement>(
			'input[placeholder="Search files and folders..."]',
		);
		await act(async () => {
			Object.getOwnPropertyDescriptor(
				HTMLInputElement.prototype,
				"value",
			)?.set?.call(search, "INVOICE");
			search?.dispatchEvent(new Event("input", { bubbles: true }));
		});
		expect(
			[...container.querySelectorAll("[data-locked-row]")].map((node) =>
				node.getAttribute("data-locked-row"),
			),
		).toEqual(["invoice.pdf"]);
		expect(
			container.querySelector(
				'[data-storage-location="apps/app-a/upload/a.txt"]',
			),
		).toBeNull();
	});
});
