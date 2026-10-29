import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import {
	type Mock,
	afterEach,
	beforeEach,
	describe,
	expect,
	it,
	vi,
} from "vitest";
import type { HomeImageReference } from "./home-image-config";
import { HomeInformationImage } from "./home-information-image";
import type { HomeStorageImageState } from "./use-home-storage-image";

const fixture = vi.hoisted(() => ({ useStorageImage: vi.fn() }));

vi.mock("./use-home-storage-image", () => ({
	useHomeStorageImage: fixture.useStorageImage,
}));

let root: Root;
let container: HTMLDivElement;
let retry: Mock<() => void>;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	fixture.useStorageImage.mockReset();
	retry = vi.fn<() => void>();
	storageState();
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
});

function storageState(overrides: Partial<HomeStorageImageState> = {}) {
	fixture.useStorageImage.mockReturnValue({
		src: "https://storage.example/media/team.webp?signature=first",
		isLoading: false,
		error: false,
		retry,
		...overrides,
	} satisfies HomeStorageImageState);
}

async function renderImage(
	image: HomeImageReference,
	alt = "Team at the office",
) {
	await act(async () => {
		root.render(<HomeInformationImage image={image} alt={alt} />);
	});
}

function image() {
	const element = container.querySelector("img");
	if (!element) throw new Error("Missing image");
	return element;
}

async function failImage() {
	await act(async () => image().dispatchEvent(new Event("error")));
}

async function retryImage() {
	const button = container.querySelector("button");
	if (!button) throw new Error("Missing retry button");
	expect(button.textContent).toBe("Try again");
	await act(async () => button.click());
}

const storedImage = (
	appId = "app-a",
	path = "media/team.webp",
): HomeImageReference => ({
	source: "storage",
	appId,
	path,
});

describe("Home information image", () => {
	it("renders existing URL images with description, lazy loading, and no referrer", async () => {
		await renderImage({
			source: "url",
			url: "https://images.example/team.webp",
		});
		expect(image().getAttribute("src")).toBe(
			"https://images.example/team.webp",
		);
		expect(image().alt).toBe("Team at the office");
		expect(image().getAttribute("loading")).toBe("lazy");
		expect(image().getAttribute("referrerpolicy")).toBe("no-referrer");
		expect(fixture.useStorageImage).not.toHaveBeenCalled();
	});

	it("retries a previously failed URL after changing to another image and back", async () => {
		const first: HomeImageReference = {
			source: "url",
			url: "https://images.example/first.png",
		};
		await renderImage(first);
		await failImage();
		expect(container.querySelector("img")).toBeNull();
		expect(container.textContent).toContain(
			"Check its URL in widget settings.",
		);
		expect(container.querySelector("button")).toBeNull();
		await renderImage({
			source: "url",
			url: "https://images.example/second.png",
		});
		expect(image().getAttribute("src")).toBe(
			"https://images.example/second.png",
		);
		await renderImage(first);
		expect(image().getAttribute("src")).toBe(first.url);
		expect(container.textContent).not.toContain("could not be loaded");
		expect(fixture.useStorageImage).not.toHaveBeenCalled();
	});

	it("shows storage loading and access errors, then allows a retry", async () => {
		storageState({ src: undefined, isLoading: true });
		await renderImage(storedImage());
		expect(container.querySelector("output")?.textContent).toContain(
			"Loading image",
		);
		expect(container.querySelector("img")).toBeNull();
		storageState({ src: undefined, isLoading: false, error: true });
		await renderImage(storedImage());
		expect(container.textContent).toContain(
			"Check your connection and access to the app.",
		);
		await retryImage();
		expect(retry).toHaveBeenCalledOnce();
		storageState();
		await renderImage(storedImage());
		expect(image().getAttribute("src")).toBe(
			"https://storage.example/media/team.webp?signature=first",
		);
		expect(image().alt).toBe("Team at the office");
	});

	it("retries a storage image decode failure and renders a refreshed URL", async () => {
		await renderImage(storedImage());
		await failImage();
		expect(container.querySelector("img")).toBeNull();
		expect(container.textContent).toContain(
			"Check your connection and access to the app.",
		);
		await retryImage();
		expect(retry).toHaveBeenCalledOnce();
		expect(image().getAttribute("src")).toBe(
			"https://storage.example/media/team.webp?signature=first",
		);
		await failImage();
		storageState({
			src: "https://storage.example/media/team.webp?signature=second",
		});
		await renderImage(storedImage());
		expect(image().getAttribute("src")).toBe(
			"https://storage.example/media/team.webp?signature=second",
		);
		expect(container.textContent).not.toContain("could not load");
	});

	it("loads a changed app and storage path with a fresh image frame", async () => {
		await renderImage(storedImage());
		expect(fixture.useStorageImage).toHaveBeenLastCalledWith(
			"app-a",
			"media/team.webp",
		);
		await failImage();
		await renderImage(storedImage("app-b"));
		expect(fixture.useStorageImage).toHaveBeenLastCalledWith(
			"app-b",
			"media/team.webp",
		);
		expect(image().getAttribute("src")).toBe(
			"https://storage.example/media/team.webp?signature=first",
		);
		await failImage();
		await renderImage(storedImage("app-b", "brand/logo.svg"), "Company logo");
		expect(fixture.useStorageImage).toHaveBeenLastCalledWith(
			"app-b",
			"brand/logo.svg",
		);
		expect(image().getAttribute("src")).toBe(
			"https://storage.example/media/team.webp?signature=first",
		);
		expect(image().alt).toBe("Company logo");
	});
});
