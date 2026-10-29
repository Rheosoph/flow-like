import { createHash, webcrypto } from "node:crypto";
import type { Window as HappyWindow } from "happy-dom";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ViewerAbout } from "./viewer-about";

const native = vi.hoisted(() => ({
	isTauri: vi.fn(),
	write: vi.fn(),
}));
vi.mock("../../lib/platform", () => ({ isTauri: native.isTauri }));
vi.mock("../../state/backend-state", () => ({
	useBackend: () => ({ storageState: { writeStorageItems: native.write } }),
}));

const bytes = new Uint8Array([31, 139, 8, 0, 1, 2, 3]);
const index = "/craft/soundcraft/licenses/index.html";
const archive = "/craft/soundcraft/licenses/sources/symphonia-0.5.5.crate";
const manifest = {
	packages: [
		{
			selectedLicense: "MPL-2.0",
			sourceArchive: "sources/symphonia-0.5.5.crate",
			checksum: createHash("sha256").update(bytes).digest("hex"),
		},
	],
};
let root: Root;
let container: HTMLDivElement;
let licenseDocument: Document;
let fetcher: ReturnType<typeof vi.fn>;
let writtenBlob: Blob | undefined;
let navigationDisabled: boolean;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	const settings = (window as unknown as HappyWindow).happyDOM.settings;
	navigationDisabled = settings.navigation.disableChildFrameNavigation;
	settings.navigation.disableChildFrameNavigation = true;
	native.isTauri.mockReturnValue(true);
	native.write.mockResolvedValue(undefined);
	writtenBlob = undefined;
	vi.stubGlobal("crypto", webcrypto);
	vi.spyOn(URL, "createObjectURL").mockImplementation((blob) => {
		writtenBlob = blob as Blob;
		return "blob:http://localhost/verified-source";
	});
	vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});
	fetcher = vi.fn(async (url: URL) =>
		url.pathname.endsWith("manifest.json")
			? Response.json(manifest)
			: new Response(bytes),
	);
	vi.stubGlobal("fetch", fetcher);
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	(
		window as unknown as HappyWindow
	).happyDOM.settings.navigation.disableChildFrameNavigation =
		navigationDisabled;
	vi.restoreAllMocks();
	vi.unstubAllGlobals();
	vi.clearAllMocks();
});

async function openAbout() {
	await act(async () =>
		root.render(<ViewerAbout label="Flows Audio" licensesUrl={index} />),
	);
	await act(async () =>
		container.querySelector<HTMLButtonElement>("button")?.click(),
	);
	const frame = document.querySelector<HTMLIFrameElement>("iframe");
	if (!frame) throw new Error("Missing license frame");
	licenseDocument = document.implementation.createHTMLDocument("Licenses");
	Object.defineProperty(frame, "contentDocument", { value: licenseDocument });
	await act(async () => frame.dispatchEvent(new Event("load")));
}

async function clickSource(url = new URL(archive, window.location.href).href) {
	const link = licenseDocument.createElement("a");
	link.href = url;
	link.download = "";
	licenseDocument.body.append(link);
	const event = new MouseEvent("click", { bubbles: true, cancelable: true });
	await act(async () => link.dispatchEvent(event));
	return event;
}

async function settle(check: () => void) {
	await vi.waitFor(async () => {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
		check();
	});
}

describe("editor source downloads", () => {
	it("uses the native save service for verified same-origin source bytes", async () => {
		await openAbout();
		const click = await clickSource();
		expect(click.defaultPrevented).toBe(true);
		await settle(() => expect(native.write).toHaveBeenCalledOnce());
		expect(native.write).toHaveBeenCalledWith([
			{
				prefix: "symphonia-0.5.5.crate",
				url: "blob:http://localhost/verified-source",
			},
		]);
		if (!writtenBlob) throw new Error("No source file was written");
		expect(new Uint8Array(await writtenBlob.arrayBuffer())).toEqual(bytes);
		expect(URL.revokeObjectURL).toHaveBeenCalledWith(
			"blob:http://localhost/verified-source",
		);
		expect(document.querySelector('[role="alert"]')).toBeNull();
	});

	it("rejects source bytes that differ from the manifest checksum", async () => {
		fetcher.mockImplementation(async (url: URL) =>
			url.pathname.endsWith("manifest.json")
				? Response.json(manifest)
				: new Response("changed"),
		);
		await openAbout();
		await clickSource();
		await settle(() =>
			expect(document.querySelector('[role="alert"]')?.textContent).toContain(
				"integrity check",
			),
		);
		expect(native.write).not.toHaveBeenCalled();
	});

	it("blocks external source links without making a request", async () => {
		await openAbout();
		await clickSource(`https://external.example${archive}`);
		await settle(() =>
			expect(document.querySelector('[role="alert"]')?.textContent).toContain(
				"does not belong",
			),
		);
		expect(fetcher).not.toHaveBeenCalled();
		expect(native.write).not.toHaveBeenCalled();
	});

	it("rejects local source paths absent from the runtime manifest", async () => {
		await openAbout();
		await clickSource(
			new URL(archive.replace("symphonia", "unknown"), window.location.href)
				.href,
		);
		await settle(() =>
			expect(document.querySelector('[role="alert"]')?.textContent).toContain(
				"missing from the editor's inventory",
			),
		);
		expect(fetcher).toHaveBeenCalledOnce();
		expect(native.write).not.toHaveBeenCalled();
	});

	it("reports native write failures and releases the verified source blob", async () => {
		native.write.mockRejectedValue(
			new Error("Could not write the selected file."),
		);
		await openAbout();
		await clickSource();
		await settle(() =>
			expect(document.querySelector('[role="alert"]')?.textContent).toContain(
				"Close and reopen About, then try again.",
			),
		);
		expect(URL.revokeObjectURL).toHaveBeenCalledOnce();
	});

	it("keeps browser downloads on the ordinary link path", async () => {
		native.isTauri.mockReturnValue(false);
		await openAbout();
		const click = await clickSource();
		expect(click.defaultPrevented).toBe(false);
		expect(fetcher).not.toHaveBeenCalled();
		expect(native.write).not.toHaveBeenCalled();
	});

	it("cancels a pending source request when About closes", async () => {
		let resolve!: (value: Response) => void;
		fetcher.mockImplementation(
			() =>
				new Promise<Response>((done) => {
					resolve = done;
				}),
		);
		await openAbout();
		await clickSource();
		const signal = fetcher.mock.calls[0][1].signal as AbortSignal;
		await act(async () =>
			document
				.querySelector<HTMLButtonElement>('[data-slot="dialog-close"]')
				?.click(),
		);
		expect(signal.aborted).toBe(true);
		await act(async () => resolve(Response.json(manifest)));
		expect(native.write).not.toHaveBeenCalled();
	});
});
