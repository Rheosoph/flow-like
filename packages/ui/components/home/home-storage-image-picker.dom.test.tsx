import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { IStorageItem } from "../../lib/schema/storage/storage-item";
import { HomeStorageImagePicker } from "./home-storage-image-picker";

const fixture = vi.hoisted(() => ({
	profile: { id: "profile-a", hub: "https://storage-fixture.invalid" },
	viewer: "viewer-a",
	ready: true,
	listStorageItems: vi.fn(),
}));

vi.mock("react-oidc-context", () => ({
	useAuth: () => ({
		isAuthenticated: true,
		user: { profile: { sub: fixture.viewer } },
	}),
}));
vi.mock("../../state/backend-state", () => ({
	useBackendReady: () => fixture.ready,
	useBackend: () => ({
		profile: fixture.profile,
		storageState: { listStorageItems: fixture.listStorageItems },
	}),
}));

const item = (location: string, is_dir = false): IStorageItem => ({
	location,
	is_dir,
	size: 100,
	last_modified: "2026-09-09T12:00:00Z",
});

let client: QueryClient;
let root: Root;
let container: HTMLDivElement;
let onChange: ReturnType<typeof vi.fn>;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	fixture.profile = { id: "profile-a", hub: "https://storage-fixture.invalid" };
	fixture.viewer = "viewer-a";
	fixture.ready = true;
	fixture.listStorageItems.mockReset();
	fixture.listStorageItems.mockResolvedValue([]);
	onChange = vi.fn();
	client = new QueryClient({
		defaultOptions: {
			queries: {
				retry: false,
				refetchOnWindowFocus: false,
				gcTime: Number.POSITIVE_INFINITY,
			},
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

async function renderPicker(appId = "app-a", value = "") {
	await act(async () => {
		root.render(
			<QueryClientProvider client={client}>
				<HomeStorageImagePicker
					appId={appId}
					value={value}
					onChange={onChange}
				/>
			</QueryClientProvider>,
		);
	});
}

async function settled(assertion: () => void) {
	let lastError: unknown;
	for (let attempt = 0; attempt < 100; attempt++) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 10));
		});
		try {
			assertion();
			return;
		} catch (error) {
			lastError = error;
		}
	}
	throw lastError;
}

function button(label: string) {
	const result = Array.from(container.querySelectorAll("button")).find(
		(candidate) =>
			candidate.getAttribute("aria-label") === label ||
			candidate.textContent === label,
	);
	if (!result) throw new Error(`Missing button: ${label}`);
	return result;
}

async function search(value: string) {
	const input = container.querySelector("input");
	if (!input) throw new Error("Missing search input");
	await act(async () => {
		Object.getOwnPropertyDescriptor(
			HTMLInputElement.prototype,
			"value",
		)?.set?.call(input, value);
		input.dispatchEvent(new Event("input", { bubbles: true }));
	});
}

describe("Home storage image picker", () => {
	it("navigates nested folders and saves an app-relative image path", async () => {
		fixture.listStorageItems.mockImplementation(
			async (_appId: string, prefix: string) => {
				if (!prefix)
					return [
						item("apps/app-a/upload/media/", true),
						item("apps/app-a/upload/readme.txt"),
					];
				if (prefix === "media")
					return [item("apps/app-a/upload/media/brand/", true)];
				if (prefix === "media/brand")
					return [
						item("apps/app-a/upload/media/brand/logo.SVG"),
						item("apps/app-a/upload/media/brand/notes.pdf"),
					];
				return [];
			},
		);
		await renderPicker();
		await settled(() => expect(button("Open folder media")).toBeDefined());
		expect(container.textContent).not.toContain("readme.txt");
		await act(async () => button("Open folder media").click());
		await settled(() => expect(button("Open folder brand")).toBeDefined());
		await act(async () => button("Open folder brand").click());
		await settled(() => expect(button("Select image logo.SVG")).toBeDefined());
		expect(fixture.listStorageItems.mock.calls).toEqual([
			["app-a", ""],
			["app-a", "media"],
			["app-a", "media/brand"],
		]);
		expect(container.textContent).not.toContain("notes.pdf");
		expect(onChange).not.toHaveBeenCalled();
		await act(async () => button("Select image logo.SVG").click());
		expect(onChange).toHaveBeenCalledWith("media/brand/logo.SVG");
		await renderPicker("app-a", "media/brand/logo.SVG");
		expect(button("Select image logo.SVG").getAttribute("aria-pressed")).toBe(
			"true",
		);
		expect(
			container.querySelector('[aria-label="Selected image"]')?.textContent,
		).toBe("media/brand/logo.SVG");
		await act(async () => button("App storage").click());
		await settled(() => expect(button("Open folder media")).toBeDefined());
		await act(async () => button("Clear selected image").click());
		expect(onChange).toHaveBeenLastCalledWith("");
	});

	it("offers web image formats and filters the current folder by name", async () => {
		fixture.listStorageItems.mockResolvedValue([
			item("hero.avif"),
			item("photo.WEBP"),
			item("logo.svg"),
			item("animation.gif"),
			item("poster.png"),
			item("portrait.jpg"),
			item("image.jpeg"),
			item("icon.ico"),
			item("bitmap.bmp"),
			item("notes.txt"),
			item("video.mp4"),
			item("images", true),
		]);
		await renderPicker();
		await settled(() =>
			expect(container.querySelectorAll("button[aria-pressed]")).toHaveLength(
				9,
			),
		);
		expect(container.textContent).not.toContain("notes.txt");
		expect(container.textContent).not.toContain("video.mp4");
		await search("PHOTO");
		expect(container.querySelectorAll("button[aria-pressed]")).toHaveLength(1);
		expect(button("Select image photo.WEBP")).toBeDefined();
		await search("missing");
		expect(container.textContent).toContain("No matching images or folders.");
	});

	it("resets folder navigation and search when switching apps", async () => {
		fixture.listStorageItems.mockImplementation(
			async (appId: string, prefix: string) => {
				if (appId === "app-b") return [item("other.png")];
				return prefix ? [item("media/photo.png")] : [item("media", true)];
			},
		);
		await renderPicker();
		await settled(() => expect(button("Open folder media")).toBeDefined());
		await act(async () => button("Open folder media").click());
		await settled(() => expect(button("Select image photo.png")).toBeDefined());
		await search("photo");
		await renderPicker("app-b");
		await settled(() => expect(button("Select image other.png")).toBeDefined());
		expect(fixture.listStorageItems).toHaveBeenLastCalledWith("app-b", "");
		expect(fixture.listStorageItems).not.toHaveBeenCalledWith("app-b", "media");
		expect(container.querySelector("input")?.value).toBe("");
		expect(container.textContent).not.toContain("photo.png");
		await renderPicker("app-a");
		await settled(() => expect(button("Open folder media")).toBeDefined());
		expect(container.querySelector("input")?.value).toBe("");
	});

	it.each(["viewer", "profile", "origin"])(
		"does not reuse another %s's cached storage listing",
		async (changedScope) => {
			fixture.listStorageItems.mockResolvedValueOnce([item("private.png")]);
			await renderPicker();
			await settled(() =>
				expect(button("Select image private.png")).toBeDefined(),
			);
			fixture.listStorageItems.mockResolvedValueOnce([item("current.png")]);
			if (changedScope === "viewer") fixture.viewer = "viewer-b";
			if (changedScope === "profile") fixture.profile.id = "profile-b";
			if (changedScope === "origin")
				fixture.profile.hub = "https://another-storage.invalid";
			await renderPicker();
			expect(container.textContent).not.toContain("private.png");
			await settled(() =>
				expect(button("Select image current.png")).toBeDefined(),
			);
			expect(fixture.listStorageItems).toHaveBeenCalledTimes(2);
		},
	);

	it("shows loading, exposes a retry after failure, and reports an empty folder", async () => {
		let fail: (reason: Error) => void = () => {};
		fixture.listStorageItems.mockImplementationOnce(
			() =>
				new Promise<IStorageItem[]>((_resolve, reject) => {
					fail = reject;
				}),
		);
		await renderPicker();
		expect(container.querySelector("output")?.textContent).toContain(
			"Loading images",
		);
		await act(async () => fail(new Error("Storage access denied")));
		await settled(() =>
			expect(container.querySelector('[role="alert"]')?.textContent).toContain(
				"Images could not load",
			),
		);
		await act(async () => button("Try again").click());
		await settled(() =>
			expect(container.querySelector("output")?.textContent).toBe(
				"No images or folders here.",
			),
		);
		expect(fixture.listStorageItems).toHaveBeenCalledTimes(2);
	});

	it("waits for the backend and an app before listing storage", async () => {
		fixture.ready = false;
		await renderPicker();
		expect(fixture.listStorageItems).not.toHaveBeenCalled();
		expect(container.textContent).toContain("Loading images");
		fixture.ready = true;
		await renderPicker("");
		expect(fixture.listStorageItems).not.toHaveBeenCalled();
		expect(container.textContent).toContain("Choose an app");
		await renderPicker();
		await settled(() =>
			expect(fixture.listStorageItems).toHaveBeenCalledOnce(),
		);
	});
});
