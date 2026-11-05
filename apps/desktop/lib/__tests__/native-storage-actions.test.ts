// @vitest-environment happy-dom
import { act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

interface StorageActions {
	revealInExplorer?: (location: string) => Promise<void>;
	openWithApp?: (location: string, appPath?: string) => Promise<void>;
	listAppsForFile?: (location: string) => Promise<unknown>;
}

const mocks = vi.hoisted(() => ({
	storage: vi.fn(),
	invoke: vi.fn(),
	openPath: vi.fn(),
	revealItemInDir: vi.fn(),
}));
vi.mock("@flow-like/flow-like-ui", () => ({
	RolePermissions: { WriteFiles: 1 },
	useAppPermissions: () => ({ isLoading: false, can: () => true }),
	useBackend: () => ({ storageState: {} }),
	StorageSystem: (props: StorageActions) => {
		mocks.storage(props);
		return null;
	},
}));
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
vi.mock("@tauri-apps/api/core", () => ({
	invoke: mocks.invoke,
	convertFileSrc: (path: string) => path,
}));
vi.mock("@tauri-apps/plugin-opener", () => ({
	openPath: mocks.openPath,
	revealItemInDir: mocks.revealItemInDir,
}));
vi.mock("next/navigation", () => ({
	useRouter: () => ({ push: vi.fn() }),
	useSearchParams: () => new URLSearchParams({ id: "project" }),
}));

import StoragePage from "../../app/library/config/storage/page";
import UserStoragePage from "../../app/library/config/user-storage/page";

let root: Root;
beforeEach(() => {
	vi.clearAllMocks();
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	Object.defineProperty(window, "__TAURI_INTERNALS__", {
		configurable: true,
		value: {},
	});
	mocks.invoke.mockResolvedValue("/selected/storage/report.pdf");
	root = createRoot(document.createElement("div"));
});
afterEach(async () => {
	await act(async () => root.unmount());
	Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
	vi.unstubAllGlobals();
});

for (const [name, Page, resolveCommand] of [
	["app storage", StoragePage, "storage_to_fullpath"],
	["user storage", UserStoragePage, "storage_user_to_fullpath"],
] as const) {
	describe(name, () => {
		async function render(
			userAgent: string,
			platform = "",
			maxTouchPoints = 0,
		) {
			vi.stubGlobal("navigator", { userAgent, platform, maxTouchPoints });
			await act(async () => root.render(createElement(Page)));
			return mocks.storage.mock.lastCall?.[0] as StorageActions;
		}

		test.each([
			["iPhone", "iPhone", 1],
			["Android", "Linux armv8l", 1],
			["Macintosh", "MacIntel", 5],
		])("%s hides external file actions", async (ua, platform, touch) => {
			const actions = await render(ua, platform, touch);
			expect(actions.revealInExplorer).toBeUndefined();
			expect(actions.listAppsForFile).toBeUndefined();
			expect(actions.openWithApp).toBeUndefined();
			expect(mocks.invoke).not.toHaveBeenCalled();
			expect(mocks.openPath).not.toHaveBeenCalled();
		});

		test("desktop retains default opening, reveal and application selection", async () => {
			const actions = await render("Windows NT 10.0", "Win32");
			await actions.openWithApp?.("report.pdf");
			expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith(resolveCommand, {
				appId: "project",
				prefix: "report.pdf",
			});
			expect(mocks.openPath).toHaveBeenCalledExactlyOnceWith(
				"/selected/storage/report.pdf",
			);
			await actions.revealInExplorer?.("report.pdf");
			expect(mocks.revealItemInDir).toHaveBeenCalledWith(
				"/selected/storage/report.pdf",
			);
			await actions.listAppsForFile?.("report.pdf");
			expect(mocks.invoke).toHaveBeenCalledWith("list_apps_for_file", {
				filePath: "/selected/storage/report.pdf",
			});
			await actions.openWithApp?.("report.pdf", "/desktop/editor");
			expect(mocks.invoke).toHaveBeenCalledWith("open_file_with_app", {
				filePath: "/selected/storage/report.pdf",
				appPath: "/desktop/editor",
			});
			expect(mocks.openPath).toHaveBeenCalledTimes(1);
		});
	});
}
