import type { Window as HappyWindow } from "happy-dom";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { FilePreviewer, PdfFrame } from "./file-previewer";

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
vi.mock("./audio-preview", () => ({ AudioPreview: () => null }));
vi.mock("./monaco-file-editor", () => ({ MonacoFileEditor: () => null }));
vi.mock("./text-editor", () => ({ TextEditor: () => null }));
const svg =
	'<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80"><rect width="120" height="80" fill="red"/></svg>';
const pdf = "%PDF-1.7\npreview bytes";
let container: HTMLDivElement;
let root: Root | null;
let originalIframeLoading: boolean;
let originalChildNavigation: boolean;
const fetchFile = vi.fn();

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	const settings = (window as unknown as HappyWindow).happyDOM.settings;
	originalIframeLoading = settings.enableIframePageLoading;
	originalChildNavigation = settings.navigation.disableChildFrameNavigation;
	settings.enableIframePageLoading = true;
	settings.navigation.disableChildFrameNavigation = true;
	fetchFile.mockReset().mockImplementation(
		async () =>
			new Response(svg, {
				headers: {
					"Content-Type": "application/octet-stream",
					"Content-Disposition": "attachment",
				},
			}),
	);
	vi.stubGlobal("fetch", fetchFile);
	vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:svg-preview");
	vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	if (root) await act(async () => root?.unmount());
	container.remove();
	const settings = (window as unknown as HappyWindow).happyDOM.settings;
	settings.enableIframePageLoading = originalIframeLoading;
	settings.navigation.disableChildFrameNavigation = originalChildNavigation;
	vi.restoreAllMocks();
	vi.unstubAllGlobals();
});

async function render(url = "/storage/image.svg") {
	await act(async () =>
		root?.render(<FilePreviewer url={url} filename="image.svg" />),
	);
}

it("previews SVG storage attachments as SVG images without embedding their markup", async () => {
	await render();
	expect(fetchFile).toHaveBeenCalledWith("/storage/image.svg", {
		cache: "no-store",
		signal: expect.any(AbortSignal),
	});
	const blob = vi.mocked(URL.createObjectURL).mock.calls[0][0] as Blob;
	expect(blob.type).toBe("image/svg+xml");
	expect(await blob.text()).toBe(svg);
	expect(container.querySelector("img")?.getAttribute("src")).toBe(
		"blob:svg-preview",
	);
	expect(container.querySelector("svg, iframe, object")).toBeNull();
});

it("releases SVG URLs when switching files and closing the preview", async () => {
	vi.mocked(URL.createObjectURL)
		.mockReturnValueOnce("blob:first")
		.mockReturnValueOnce("blob:second");
	await render("/first.svg");
	await render("/second.svg");
	expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:first");
	expect(fetchFile).toHaveBeenLastCalledWith("/second.svg", {
		cache: "no-store",
		signal: expect.any(AbortSignal),
	});
	await act(async () => root?.unmount());
	root = null;
	expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:second");
});

it("keeps a direct SVG source available when fetching is unavailable", async () => {
	fetchFile.mockRejectedValueOnce(new Error("custom asset protocol"));
	vi.spyOn(console, "warn").mockImplementation(() => {});
	await render("asset://local/image.svg");
	expect(container.querySelector("img")?.getAttribute("src")).toBe(
		"asset://local/image.svg",
	);
	expect(URL.createObjectURL).not.toHaveBeenCalled();
});

function pdfResponse() {
	return new Response(pdf, {
		headers: {
			"Content-Type": "application/octet-stream",
			"Content-Disposition": "attachment; filename=report.pdf",
		},
	});
}

async function renderPdf(url = "/storage/report.pdf", page?: number) {
	await act(async () =>
		root?.render(<PdfFrame url={url} filename="report.pdf" page={page} />),
	);
}

it("previews a PDF download response as a typed blob at the requested page", async () => {
	fetchFile.mockResolvedValueOnce(pdfResponse());
	vi.mocked(URL.createObjectURL).mockReturnValue("blob:pdf-preview");
	await renderPdf("/storage/report.pdf?signature=example", 4);

	expect(fetchFile).toHaveBeenCalledWith(
		"/storage/report.pdf?signature=example",
		{ cache: "no-store", signal: expect.any(AbortSignal) },
	);
	const blob = vi.mocked(URL.createObjectURL).mock.calls[0][0] as Blob;
	expect(blob.type).toBe("application/pdf");
	expect(await blob.text()).toBe(pdf);
	expect(container.querySelector("iframe")?.getAttribute("src")).toBe(
		"blob:pdf-preview#page=4&toolbar=1&view=FitH",
	);
});

it.each(["network", "http"])(
	"shows download and retry actions without navigating after a PDF %s failure",
	async (failure) => {
		if (failure === "network") {
			fetchFile.mockRejectedValueOnce(new TypeError("Failed to fetch"));
		} else {
			fetchFile.mockResolvedValueOnce(new Response(null, { status: 403 }));
		}
		vi.spyOn(console, "warn").mockImplementation(() => {});
		await renderPdf();

		expect(container.querySelector("iframe, object, embed")).toBeNull();
		expect(URL.createObjectURL).not.toHaveBeenCalled();
		expect(container.querySelector('[role="alert"]')?.textContent).toBe(
			"Could not load the PDF preview.",
		);
		const download = container.querySelector("a");
		expect(download?.getAttribute("href")).toBe("/storage/report.pdf");
		expect(download?.getAttribute("download")).toBe("report.pdf");
		expect(download?.textContent).toBe("Download the PDF");

		fetchFile.mockResolvedValueOnce(pdfResponse());
		vi.mocked(URL.createObjectURL).mockReturnValue("blob:retried-pdf");
		const retry = container.querySelector("button");
		expect(retry?.textContent).toBe("Retry");
		await act(async () => retry?.click());

		expect(fetchFile).toHaveBeenCalledTimes(2);
		expect(container.querySelector('[role="alert"]')).toBeNull();
		expect(container.querySelector("iframe")?.getAttribute("src")).toBe(
			"blob:retried-pdf#toolbar=1&view=FitH",
		);
	},
);

it("releases PDF object URLs when switching files and closing the preview", async () => {
	fetchFile.mockImplementation(async () => pdfResponse());
	vi.mocked(URL.createObjectURL)
		.mockReturnValueOnce("blob:first-pdf")
		.mockReturnValueOnce("blob:second-pdf");
	await renderPdf("/first.pdf");
	await renderPdf("/second.pdf");
	expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:first-pdf");
	expect(container.querySelector("iframe")?.getAttribute("src")).toBe(
		"blob:second-pdf#toolbar=1&view=FitH",
	);
	await act(async () => root?.unmount());
	root = null;
	expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:second-pdf");
});

it("aborts a replaced PDF request and ignores its late response", async () => {
	let resolveFirst!: (response: Response) => void;
	fetchFile.mockReturnValueOnce(
		new Promise<Response>((resolve) => {
			resolveFirst = resolve;
		}),
	);
	await renderPdf("/first.pdf");
	const firstSignal = fetchFile.mock.calls[0][1].signal as AbortSignal;
	expect(container.querySelector("iframe")).toBeNull();

	fetchFile.mockResolvedValueOnce(pdfResponse());
	vi.mocked(URL.createObjectURL).mockReturnValue("blob:second-pdf");
	await renderPdf("/second.pdf");
	expect(firstSignal.aborted).toBe(true);
	await act(async () => resolveFirst(pdfResponse()));

	expect(URL.createObjectURL).toHaveBeenCalledTimes(1);
	expect(container.querySelector("iframe")?.getAttribute("src")).toBe(
		"blob:second-pdf#toolbar=1&view=FitH",
	);
});
