import type { Window as HappyWindow } from "happy-dom";
import { type ComponentProps, type ReactNode, act } from "react";
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
import type { BulkUploadProgressCallback } from "../../lib/bulk-upload";
import { downloadCraftFile } from "./craft-download";
vi.mock("./craft-download", () => ({ downloadCraftFile: vi.fn() }));
import { CraftDocument } from "./craft-document";
import { executeCraftCommand } from "./craft-mcp-session";
import { craftViewer } from "./craft-viewers";
const CRAFT_CHANNEL = "flow-like-craft";
const XLSX_MIME = requireViewer("report.xlsx").mime;

vi.mock("../ui/button", () => ({
	Button: ({
		asChild,
		children,
		variant: _variant,
		size: _size,
		...props
	}: ComponentProps<"button"> & {
		asChild?: boolean;
		children?: ReactNode;
		variant?: string;
		size?: string;
	}) =>
		asChild ? (
			<>{children}</>
		) : (
			<button type="button" {...props}>
				{children}
			</button>
		),
}));

function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (reason: unknown) => void;
	const promise = new Promise<T>((resolvePromise, rejectPromise) => {
		resolve = resolvePromise;
		reject = rejectPromise;
	});
	return { promise, resolve, reject };
}

const originalBytes = new Uint8Array([80, 75, 3, 4, 1, 2]);
const editedBytes = new Uint8Array([80, 75, 3, 4, 9, 8]);
let root: Root | null;
let container: HTMLDivElement;
let frameWindow: Window;
let send: ReturnType<typeof vi.fn>;
let fetchDocument: ReturnType<typeof vi.fn>;
let saveDocument: Mock<
	(file: File, onProgress?: BulkUploadProgressCallback) => Promise<void>
>;
let dirtyChange: Mock<(dirty: boolean) => void>;
let busyChange: Mock<(busy: boolean) => void>;
let reportError: Mock<(message: string) => void>;
let iframeLoadingDisabled: boolean;

beforeEach(() => {
	vi.mocked(downloadCraftFile).mockClear();
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	const settings = (window as unknown as HappyWindow).happyDOM.settings;
	iframeLoadingDisabled = settings.navigation.disableChildFrameNavigation;
	settings.navigation.disableChildFrameNavigation = true;
	send = vi.fn();
	frameWindow = { postMessage: send } as unknown as Window;
	vi.spyOn(HTMLIFrameElement.prototype, "contentWindow", "get").mockReturnValue(
		frameWindow,
	);
	fetchDocument = vi.fn().mockResolvedValue(new Response(originalBytes));
	vi.stubGlobal("fetch", fetchDocument);
	saveDocument = vi.fn().mockResolvedValue(undefined);
	dirtyChange = vi.fn();
	busyChange = vi.fn();
	reportError = vi.fn();
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	if (root) await act(async () => root?.unmount());
	root = null;
	container.remove();
	const settings = (window as unknown as HappyWindow).happyDOM.settings;
	settings.navigation.disableChildFrameNavigation = iframeLoadingDisabled;
	vi.restoreAllMocks();
	vi.unstubAllGlobals();
	vi.unstubAllEnvs();
	vi.useRealTimers();
});

async function render(
	editable = true,
	url = "/files/report.xlsx",
	props: Partial<ComponentProps<typeof CraftDocument>> = {},
) {
	await act(async () => {
		root?.render(
			<CraftDocument
				viewer={requireViewer("report.xlsx")}
				url={url}
				filename="report.xlsx"
				editable={editable}
				onSave={saveDocument}
				onDirtyChange={dirtyChange}
				onBusyChange={busyChange}
				onError={reportError}
				{...props}
			/>,
		);
	});
}

function session() {
	const frame = container.querySelector("iframe");
	if (!frame) throw new Error("The viewer frame is not mounted");
	return new URL(frame.src, window.location.href).searchParams.get("session");
}

async function receive(
	message: Record<string, unknown>,
	overrides: MessageEventInit = {},
) {
	await act(async () => {
		window.dispatchEvent(
			new MessageEvent("message", {
				data: { channel: CRAFT_CHANNEL, session: session(), ...message },
				origin: window.location.origin,
				source: frameWindow,
				...overrides,
			}),
		);
	});
}

function messages(type: string) {
	return send.mock.calls
		.map(([message]) => message as Record<string, unknown>)
		.filter((message) => message.type === type);
}

it("keeps the editor's loading controls reachable while progress extends its timeout", async () => {
	vi.useFakeTimers();
	await render(true, "/files/design.solidcraft", {
		viewer: requireViewer("design.solidcraft"),
		filename: "design.solidcraft",
		initialEditing: true,
	});
	await receive({
		type: "loading",
		interactive: true,
		completed: 0,
		total: 10,
	});
	expect(container.querySelector("output")?.classList.contains("sr-only")).toBe(
		false,
	);
	await receive({ type: "ready" });
	await act(async () => vi.advanceTimersByTime(80_000));
	await receive({
		type: "loading",
		interactive: true,
		completed: 2,
		total: 10,
	});
	expect(container.querySelector("output")?.classList.contains("sr-only")).toBe(
		true,
	);
	expect(button("Save").disabled).toBe(true);
	await act(async () => vi.advanceTimersByTime(80_000));
	expect(reportError).not.toHaveBeenCalled();
	await receive({ type: "opened" });
	expect(container.querySelector("output")).toBeNull();
	expect(button("Save").disabled).toBe(false);
	await receive({
		type: "loading",
		interactive: true,
		completed: 3,
		total: 10,
	});
	await act(async () => vi.advanceTimersByTime(100_000));
	expect(reportError).not.toHaveBeenCalled();
});

it("fails a staged load when progress stops and allows retry after cancellation", async () => {
	vi.useFakeTimers();
	fetchDocument.mockImplementation(async () => new Response(originalBytes));
	await render();
	await receive({ type: "ready" });
	await receive({
		type: "loading",
		interactive: true,
		completed: 0,
		total: 10,
	});
	await act(async () => vi.advanceTimersByTime(90_000));
	expect(reportError).toHaveBeenCalledOnce();
	expect(container.querySelector("iframe")).toBeNull();
	await act(async () => button("Try again").click());
	await receive({ type: "ready" });
	await receive({
		type: "loading",
		interactive: true,
		completed: 1,
		total: 10,
	});
	await receive({
		type: "error",
		error: "Opening the document was cancelled.",
	});
	expect(container.textContent).toContain(
		"Opening the document was cancelled.",
	);
	expect(container.querySelector("iframe")).toBeNull();
	expect(button("Try again").disabled).toBe(false);
});

it("registers the opened document for scoped MCP and invalidates it on replacement", async () => {
	fetchDocument.mockImplementation(async () => new Response(originalBytes));
	const toolTarget = {
		appId: "app",
		surfaceId: "page",
		componentId: "preview",
	};
	const context = { appId: "app" };
	const elementRef = "page/preview";
	await render(true, "/files/report.xlsx", { toolTarget });
	await expect(
		executeCraftCommand("craft.session", { elementRef }, context),
	).rejects.toMatchObject({ code: "session_unavailable" });
	await receive({ type: "ready" });
	await receive({ type: "opened" });
	const pin = (await executeCraftCommand(
		"craft.session",
		{ elementRef },
		context,
	)) as { sessionId: string; editor: string };
	expect(pin).toEqual({ sessionId: session(), editor: "gridcraft" });
	const copy = vi
		.spyOn(navigator.clipboard, "writeText")
		.mockResolvedValue(undefined);
	await act(async () => {
		container
			.querySelector<HTMLButtonElement>('[aria-label="Copy agent target"]')
			?.click();
	});
	expect(copy).toHaveBeenCalledExactlyOnceWith(elementRef);
	const message = { jsonrpc: "2.0", id: "list", method: "tools/list" };
	const result = executeCraftCommand(
		"craft.mcp",
		{ elementRef, ...pin, message },
		context,
	);
	const request = messages("mcp-request").at(-1);
	expect(request?.message).toEqual(message);
	await receive({
		type: "mcp-response",
		requestId: request?.requestId,
		response: { jsonrpc: "2.0", id: "list", result: { tools: [] } },
	});
	await expect(result).resolves.toMatchObject({ id: "list" });
	const pending = executeCraftCommand(
		"craft.mcp",
		{ elementRef, ...pin, message },
		context,
	);
	const rejected = expect(pending).rejects.toMatchObject({
		code: "session_closed",
	});
	await render(true, "/files/replacement.xlsx", { toolTarget });
	await rejected;
	await receive({ type: "ready" });
	await receive({ type: "opened" });
	await expect(
		executeCraftCommand("craft.mcp", { elementRef, ...pin, message }, context),
	).rejects.toMatchObject({ code: "session_changed" });
});

function button(label: string) {
	const match = [...container.querySelectorAll("button")].find((item) =>
		item.textContent?.includes(label),
	);
	if (!match) throw new Error(`Missing button: ${label}`);
	return match;
}

async function click(label: string) {
	await act(async () => button(label).click());
}

async function openForEditing() {
	await render();
	await receive({ type: "ready" });
	await receive({ type: "opened" });
	await click("Edit");
	await receive({ type: "dirty", dirty: true });
}

async function exportDocument(revision = 4) {
	await click("Save");
	const request = messages("export").at(-1);
	if (!request) throw new Error("No export requested");
	await receive({
		type: "exported",
		requestId: request.requestId,
		bytes: editedBytes.slice().buffer,
		revision,
	});
	return request.requestId;
}

describe("Flows Sheets document host", () => {
	it("reports readiness once to the latest callback after the document opens", async () => {
		const ready = vi.fn();
		const latestReady = vi.fn();
		await render(true, undefined, { onReady: ready });
		await receive({ type: "ready" });
		expect(ready).not.toHaveBeenCalled();
		await render(true, undefined, { onReady: latestReady });
		await receive({ type: "opened" });
		expect(ready).not.toHaveBeenCalled();
		expect(latestReady).toHaveBeenCalledOnce();
		await receive({ type: "opened" });
		await render(true, undefined, { onReady: ready });
		expect(latestReady).toHaveBeenCalledOnce();
		expect(ready).not.toHaveBeenCalled();
		expect(fetchDocument).toHaveBeenCalledTimes(1);
	});

	it.each([true, false])(
		"opens in initial edit mode only with permission: %s",
		async (editable) => {
			await render(editable, undefined, { initialEditing: true });
			await receive({ type: "ready" });
			expect(messages("open").at(-1)).toMatchObject({ readOnly: !editable });
			await receive({ type: "opened" });
			expect(messages("set-read-only").at(-1)).toMatchObject({
				readOnly: !editable,
			});
			expect(!!container.querySelector('[aria-label="Save"]')).toBe(editable);
		},
	);

	it("revokes initial editing while the file is still loading", async () => {
		const download = deferred<Response>();
		fetchDocument.mockReturnValue(download.promise);
		await render(true, undefined, { initialEditing: true });
		await receive({ type: "ready" });
		await render(false, undefined, { initialEditing: true });
		await act(async () => download.resolve(new Response(originalBytes)));
		expect(messages("open").at(-1)).toMatchObject({ readOnly: true });
		await receive({ type: "opened" });
		await render(true, undefined, { initialEditing: true });
		expect(messages("set-read-only").at(-1)).toMatchObject({ readOnly: true });
		expect(container.querySelector('[aria-label="Save"]')).toBeNull();
	});

	it("can hide the download control without hiding editing controls", async () => {
		await render(true, undefined, { showDownload: false });
		await receive({ type: "ready" });
		await receive({ type: "opened" });
		expect(container.querySelector("a[download]")).toBeNull();
		expect(button("Edit").disabled).toBe(false);
		await render();
		expect(container.querySelector("a[download]")).not.toBeNull();
	});

	it("reports a load error to the latest callback without repeating it on render", async () => {
		const download = deferred<Response>();
		fetchDocument.mockReturnValue(download.promise);
		await render();
		const latestError = vi.fn();
		await render(true, undefined, { onError: latestError });
		expect(fetchDocument).toHaveBeenCalledTimes(1);
		await act(async () =>
			download.resolve(new Response(null, { status: 503 })),
		);
		expect(reportError).not.toHaveBeenCalled();
		expect(latestError).toHaveBeenCalledExactlyOnceWith(
			"The document could not be downloaded.",
		);
		await render();
		expect(reportError).not.toHaveBeenCalled();
		expect(latestError).toHaveBeenCalledTimes(1);
	});

	it("reports export and persistence errors once while retaining edits", async () => {
		await openForEditing();
		await click("Save");
		await receive({ type: "error", error: "Export failed." });
		await receive({ type: "error", error: "Export failed." });
		expect(reportError).toHaveBeenCalledExactlyOnceWith("Export failed.");
		expect(messages("saved")).toHaveLength(0);
		saveDocument.mockRejectedValueOnce(new Error("Upload failed."));
		await exportDocument();
		expect(reportError).toHaveBeenCalledTimes(2);
		expect(reportError).toHaveBeenLastCalledWith("Upload failed.");
		expect(messages("saved")).toHaveLength(0);
		expect(container.textContent).toContain("Unsaved changes");
	});

	it("waits for both viewer readiness and downloaded bytes before opening", async () => {
		const download = deferred<Response>();
		fetchDocument.mockReturnValue(download.promise);
		await render();
		await receive({ type: "ready" });
		expect(messages("open")).toHaveLength(0);
		await act(async () => download.resolve(new Response(originalBytes)));
		expect(messages("open")).toHaveLength(1);
		const open = messages("open")[0];
		expect(open).toMatchObject({ name: "report.xlsx", readOnly: true });
		expect(new Uint8Array(open.bytes as ArrayBuffer)).toEqual(originalBytes);
		expect(send).toHaveBeenCalledWith(open, window.location.origin, [
			open.bytes,
		]);
		await receive({ type: "ready" });
		expect(messages("open")).toHaveLength(1);
	});

	it("does not open downloaded bytes until the active iframe sends ready", async () => {
		await render();
		expect(fetchDocument).toHaveBeenCalledWith(
			"/files/report.xlsx",
			expect.objectContaining({ cache: "no-store" }),
		);
		expect(messages("open")).toHaveLength(0);
		await receive({ type: "ready" }, { source: window });
		await receive({ type: "ready" }, { origin: "https://forged.example.test" });
		await receive({ type: "ready", session: "previous-session" });
		expect(messages("open")).toHaveLength(0);
		await receive({ type: "ready" });
		expect(messages("open")).toHaveLength(1);
	});

	it("starts read-only and enables editing only after the user chooses Edit", async () => {
		await render();
		expect(button("Edit").disabled).toBe(true);
		await receive({ type: "ready" });
		await receive({ type: "opened" });
		expect(messages("set-read-only").at(-1)).toMatchObject({ readOnly: true });
		expect(button("Edit").disabled).toBe(false);
		await click("Edit");
		expect(messages("set-read-only").at(-1)).toMatchObject({ readOnly: false });
		expect(
			container.querySelector('[aria-label="About Flows Sheets"]'),
		).not.toBeNull();
	});

	it("never exposes editing when storage write access is unavailable", async () => {
		await render(false);
		await receive({ type: "ready" });
		await receive({ type: "opened" });
		expect(container.textContent).not.toContain("Edit file");
		expect(container.textContent).not.toContain("Save");
		expect(messages("set-read-only").at(-1)).toMatchObject({ readOnly: true });
	});

	it("opens licenses in the app for read-only users without replacing the document", async () => {
		await render(false);
		const documentFrame = container.querySelector("iframe");
		await act(async () => {
			container
				.querySelector<HTMLButtonElement>('[aria-label="About Flows Sheets"]')
				?.click();
		});
		const dialog = document.querySelector('[role="dialog"]');
		expect(dialog).not.toBeNull();
		expect(dialog?.querySelector("iframe")?.getAttribute("src")).toBe(
			"/craft/gridcraft/licenses/index.html",
		);
		expect(dialog?.querySelector("iframe")?.title).toBe(
			"Flows Sheets third-party licenses and source",
		);
		await act(async () => {
			dialog
				?.querySelector<HTMLButtonElement>('[data-slot="dialog-close"]')
				?.click();
		});
		expect(document.querySelector('[role="dialog"]')).toBeNull();
		expect(container.querySelector("iframe")).toBe(documentFrame);
		expect(saveDocument).not.toHaveBeenCalled();
	});

	it("uploads binary spreadsheet bytes and acknowledges only after storage succeeds", async () => {
		const upload = deferred<void>();
		saveDocument.mockReturnValue(upload.promise);
		await openForEditing();
		const downloadBeforeSave = container.querySelector("a")?.href;
		await exportDocument(7);
		expect(saveDocument).toHaveBeenCalledOnce();
		const file = saveDocument.mock.calls[0][0] as File;
		expect(file.name).toBe("report.xlsx");
		expect(file.type).toBe(XLSX_MIME);
		expect(new Uint8Array(await file.arrayBuffer())).toEqual(editedBytes);
		expect(messages("saved")).toHaveLength(0);
		expect(button("Save").disabled).toBe(true);
		expect(busyChange).toHaveBeenLastCalledWith(true);
		await act(async () => upload.resolve());
		expect(container.querySelector("a")?.href).not.toBe(downloadBeforeSave);
		expect(messages("saved")).toEqual([
			expect.objectContaining({ type: "saved", revision: 7 }),
		]);
		expect(busyChange).toHaveBeenLastCalledWith(false);
		expect(dirtyChange).toHaveBeenLastCalledWith(true);
		await receive({ type: "dirty", dirty: false });
		expect(container.textContent).toContain("Saved");
	});

	it("retains dirty edits after a failed upload and permits another save", async () => {
		saveDocument.mockRejectedValueOnce(new Error("Storage is unavailable"));
		await openForEditing();
		const failedRequest = await exportDocument();
		expect(container.querySelector('[role="alert"]')?.textContent).toContain(
			"Storage is unavailable",
		);
		expect(container.textContent).toContain("Unsaved changes");
		expect(messages("saved")).toHaveLength(0);
		expect(dirtyChange).toHaveBeenLastCalledWith(true);
		expect(button("Save").disabled).toBe(false);
		const nextRequest = await exportDocument();
		expect(nextRequest).not.toBe(failedRequest);
		expect(saveDocument).toHaveBeenCalledTimes(2);
		expect(messages("saved")).toHaveLength(1);
		expect(container.querySelector('[role="alert"]')).toBeNull();
	});

	it("shows export stages and storage progress until the save is acknowledged", async () => {
		const upload = deferred<void>();
		saveDocument.mockReturnValue(upload.promise);
		await openForEditing();
		await click("Save");
		const requestId = messages("export").at(-1)?.requestId;
		expect(container.textContent).toContain("Preparing file...");
		expect(container.querySelector('[role="progressbar"]')).toBeNull();
		await receive({
			type: "export-progress",
			requestId,
			stage: "rendering",
			progress: 42.5,
		});
		expect(container.textContent).toContain("Rendering video...");
		expect(
			container
				.querySelector('[role="progressbar"]')
				?.getAttribute("aria-valuenow"),
		).toBe("42");
		await receive({
			type: "export-progress",
			requestId,
			stage: "finalizing",
			progress: null,
		});
		expect(container.textContent).toContain("Finalizing video...");
		expect(container.querySelector('[role="progressbar"]')).toBeNull();
		await receive({
			type: "exported",
			requestId,
			bytes: editedBytes.slice().buffer,
			revision: 4,
		});
		expect(container.textContent).toContain("Saving file...");
		const onProgress = saveDocument.mock.calls[0][1];
		await act(async () => onProgress?.(73));
		expect(
			container
				.querySelector('[role="progressbar"]')
				?.getAttribute("aria-valuenow"),
		).toBe("73");
		await act(async () => onProgress?.(100));
		expect(container.textContent).toContain("Finishing save...");
		expect(messages("saved")).toHaveLength(0);
		expect(button("Save").disabled).toBe(true);
		await act(async () => upload.resolve());
		expect(messages("saved")).toHaveLength(1);
		expect(container.textContent).not.toContain("Finishing save...");
		await act(async () => onProgress?.(20));
		expect(container.querySelector('[role="progressbar"]')).toBeNull();
	});

	it("ignores stale and forged progress, including progress after export", async () => {
		const upload = deferred<void>();
		saveDocument.mockReturnValue(upload.promise);
		await openForEditing();
		await click("Save");
		const requestId = messages("export").at(-1)?.requestId;
		const progress = {
			type: "export-progress",
			requestId,
			stage: "rendering",
			progress: 50,
		};
		await receive({ ...progress, requestId: "old-save" });
		await receive(progress, { source: window });
		await receive(progress, { origin: "https://forged.example.test" });
		expect(container.textContent).toContain("Preparing file...");
		await receive({
			type: "exported",
			requestId,
			bytes: editedBytes.slice().buffer,
			revision: 4,
		});
		await receive(progress);
		expect(container.textContent).toContain("Saving file...");
		expect(container.textContent).not.toContain("Rendering video...");
		await act(async () => upload.resolve());
	});

	it("does not report a save failure for a viewer error while storage is still writing", async () => {
		const upload = deferred<void>();
		saveDocument.mockReturnValue(upload.promise);
		await openForEditing();
		await exportDocument();
		await receive({ type: "error", error: "The video preview stopped." });
		expect(container.querySelector('[role="alert"]')?.textContent).toContain(
			"The video preview stopped.",
		);
		expect(container.textContent).not.toContain("Could not save file");
		expect(container.textContent).toContain("Saving file...");
		expect(button("Save").disabled).toBe(true);
		await act(async () => upload.resolve());
		expect(messages("saved")).toHaveLength(1);
	});

	it("keeps measured long exports running and reports a stalled export", async () => {
		vi.useFakeTimers();
		await openForEditing();
		await click("Save");
		const requestId = messages("export").at(-1)?.requestId;
		const progress = {
			type: "export-progress",
			requestId,
			stage: "rendering",
			progress: 10,
		};
		await receive(progress);
		await act(async () => vi.advanceTimersByTimeAsync(9 * 60_000));
		await receive({ ...progress, progress: 20 });
		await act(async () => vi.advanceTimersByTimeAsync(9 * 60_000));
		expect(container.querySelector('[role="alert"]')).toBeNull();
		expect(button("Save").disabled).toBe(true);
		await act(async () => vi.advanceTimersByTimeAsync(60_000));
		expect(container.querySelector('[role="alert"]')?.textContent).toContain(
			"Preparing the file took too long",
		);
		expect(container.textContent).toContain("Your changes are still open");
		expect(button("Save").disabled).toBe(false);
		expect(container.querySelector('[role="progressbar"]')).toBeNull();
		expect(busyChange).toHaveBeenLastCalledWith(false);
		await receive(progress);
		expect(container.querySelector('[role="progressbar"]')).toBeNull();
	});

	it("shows export errors immediately and retries without discarding edits", async () => {
		await openForEditing();
		await click("Save");
		const requestId = messages("export").at(-1)?.requestId;
		await receive({
			type: "error",
			requestId,
			error: "The video encoder is unavailable.",
		});
		expect(container.querySelector('[role="alert"]')?.textContent).toContain(
			"The video encoder is unavailable.",
		);
		expect(container.textContent).toContain("Could not save file");
		expect(dirtyChange).toHaveBeenLastCalledWith(true);
		expect(messages("saved")).toHaveLength(0);
		expect(busyChange).toHaveBeenLastCalledWith(false);
		await click("Try saving again");
		expect(messages("export")).toHaveLength(2);
		expect(messages("export").at(-1)?.requestId).not.toBe(requestId);
		expect(container.querySelector('[role="alert"]')).toBeNull();
		expect(container.textContent).toContain("Preparing file...");
	});

	it("ignores stale exports and duplicate responses during and after upload", async () => {
		const upload = deferred<void>();
		saveDocument.mockReturnValue(upload.promise);
		await openForEditing();
		await click("Save");
		const requestId = messages("export").at(-1)?.requestId;
		await receive({
			type: "exported",
			requestId: "stale-save",
			bytes: editedBytes.slice().buffer,
			revision: 4,
		});
		expect(saveDocument).not.toHaveBeenCalled();
		const exported = {
			type: "exported",
			requestId,
			bytes: editedBytes.slice().buffer,
			revision: 4,
		};
		await receive(exported);
		await receive(exported);
		expect(saveDocument).toHaveBeenCalledOnce();
		await act(async () => upload.resolve());
		await receive(exported);
		expect(saveDocument).toHaveBeenCalledOnce();
		expect(messages("saved")).toHaveLength(1);
	});

	it("ignores forged dirty and export messages after opening", async () => {
		await openForEditing();
		await receive({ type: "dirty", dirty: false }, { source: window });
		expect(dirtyChange).toHaveBeenLastCalledWith(true);
		await click("Save");
		const requestId = messages("export").at(-1)?.requestId;
		await receive(
			{
				type: "exported",
				requestId,
				bytes: editedBytes.slice().buffer,
				revision: 1,
			},
			{ origin: "https://forged.example.test" },
		);
		expect(saveDocument).not.toHaveBeenCalled();
	});

	it("refuses an export when write permission is revoked while preparing it", async () => {
		await openForEditing();
		await click("Save");
		const requestId = messages("export").at(-1)?.requestId;
		await render(false);
		expect(messages("set-read-only").at(-1)).toMatchObject({ readOnly: true });
		await receive({
			type: "exported",
			requestId,
			bytes: editedBytes.slice().buffer,
			revision: 1,
		});
		expect(saveDocument).not.toHaveBeenCalled();
		expect(container.querySelector('[role="alert"]')?.textContent).toContain(
			"permission",
		);
		expect(busyChange).toHaveBeenLastCalledWith(false);
	});

	it("aborts download and resets host callbacks when unmounted", async () => {
		const download = deferred<Response>();
		fetchDocument.mockReturnValue(download.promise);
		await render();
		const signal = fetchDocument.mock.calls[0][1].signal as AbortSignal;
		expect(signal.aborted).toBe(false);
		await act(async () => root?.unmount());
		root = null;
		expect(signal.aborted).toBe(true);
		expect(dirtyChange).toHaveBeenLastCalledWith(false);
		expect(busyChange).toHaveBeenLastCalledWith(false);
		await act(async () => download.resolve(new Response(originalBytes)));
		expect(send).not.toHaveBeenCalled();
	});

	it("does not acknowledge a late upload after the editor has unmounted", async () => {
		const upload = deferred<void>();
		saveDocument.mockReturnValue(upload.promise);
		await openForEditing();
		await exportDocument();
		await act(async () => root?.unmount());
		root = null;
		await act(async () => upload.resolve());
		expect(messages("saved")).toHaveLength(0);
		expect(dirtyChange).toHaveBeenLastCalledWith(false);
		expect(busyChange).toHaveBeenLastCalledWith(false);
	});

	it("retries a failed download with a new iframe session", async () => {
		const download = deferred<Response>();
		fetchDocument.mockReturnValueOnce(download.promise);
		await render();
		const previousSession = session();
		await act(async () =>
			download.resolve(new Response(null, { status: 503 })),
		);
		expect(container.querySelector('[role="alert"]')?.textContent).toContain(
			"could not be downloaded",
		);
		expect(container.querySelector("iframe")).toBeNull();
		fetchDocument.mockResolvedValueOnce(new Response(originalBytes));
		await click("Try again");
		expect(session()).not.toBe(previousSession);
		await receive({ type: "ready", session: previousSession });
		expect(messages("open")).toHaveLength(0);
		await receive({ type: "ready" });
		expect(fetchDocument).toHaveBeenCalledTimes(2);
		expect(messages("open")).toHaveLength(1);
		expect(container.querySelector('[role="alert"]')).toBeNull();
	});
});

describe("native previews and conversion", () => {
	async function renderFile(
		name: string,
		initialEditing = false,
		editable = true,
	) {
		await act(async () =>
			root?.render(
				<CraftDocument
					viewer={requireViewer(name)}
					url={`/files/${name}`}
					filename={name}
					editable={editable}
					initialEditing={initialEditing}
					onSave={saveDocument}
					onDirtyChange={dirtyChange}
					onBusyChange={busyChange}
				/>,
			),
		);
	}
	it("reports readiness when a native preview has downloaded", async () => {
		const ready = vi.fn();
		const download = deferred<Response>();
		fetchDocument.mockReturnValue(download.promise);
		await render(true, "/files/report.pdf", {
			viewer: requireViewer("report.pdf"),
			filename: "report.pdf",
			onReady: ready,
		});
		expect(ready).not.toHaveBeenCalled();
		await act(async () => download.resolve(new Response(originalBytes)));
		expect(ready).toHaveBeenCalledOnce();
		expect(container.querySelector("iframe")?.title).toBe(
			"PDF Preview: report.pdf",
		);
	});

	it("reads PDF in the browser and mounts PDFCraft only after Edit", async () => {
		await renderFile("report.pdf");
		expect(container.querySelector("iframe")?.title).toBe(
			"PDF Preview: report.pdf",
		);
		expect(container.querySelector("iframe")?.src).toContain(
			"#toolbar=1&view=FitH",
		);
		expect(container.querySelector('iframe[src*="/craft/"]')).toBeNull();
		expect(messages("open")).toHaveLength(0);
		await click("Edit");
		expect(container.querySelector("iframe")?.src).toContain(
			"/craft/pdfcraft/index.html",
		);
		await receive({ type: "ready" });
		expect(messages("open").at(-1)).toMatchObject({
			name: "report.pdf",
			readOnly: false,
		});
	});
	it.each(["report.pdf", "drawing.svg", "sample.mp3", "sample.mp4"])(
		"opens %s editing directly from the inline preview action",
		async (name) => {
			await renderFile(name, true);
			expect(container.querySelector("iframe")?.src).toContain(
				`/craft/${requireViewer(name).id}/index.html`,
			);
			expect(container.querySelector('[aria-label="Edit file"]')).toBeNull();
			await receive({ type: "ready" });
			expect(messages("open").at(-1)).toMatchObject({
				name,
			});
			await receive({ type: "opened" });
			expect(messages("set-read-only").at(-1)).toMatchObject({
				readOnly: false,
			});
			await exportDocument();
			expect(saveDocument).toHaveBeenCalledOnce();
		},
	);
	it("keeps PDFs in read mode when initial editing lacks write permission", async () => {
		await renderFile("report.pdf", true, false);
		expect(container.querySelector("iframe")?.title).toBe(
			"PDF Preview: report.pdf",
		);
		expect(container.querySelector('iframe[src*="/craft/"]')).toBeNull();
		expect(container.querySelector('[aria-label="Save"]')).toBeNull();
	});
	it("uses a converted extension and keeps the conversion target stable across saves", async () => {
		await renderFile("legacy.xls");
		await receive({ type: "ready" });
		await receive({ type: "opened", warnings: ["A chart is unsupported."] });
		expect(container.textContent).toContain("A chart is unsupported.");
		await click("Edit");
		expect(button("Save as .xlsx")).toBeTruthy();
		await exportDocument();
		expect(messages("export").at(-1)?.format).toBe("xlsx");
		const first = saveDocument.mock.calls[0][0];
		expect(first.name).toMatch(/^legacy-converted-.+\.xlsx$/);
		expect(first.type).toBe(requireViewer("legacy.xlsx").mime);
		await exportDocument();
		expect(saveDocument.mock.calls[1][0].name).toBe(first.name);
	});
	it("shows embedded-preview import limits", async () => {
		await renderFile("photo.kra");
		await receive({ type: "ready" });
		await receive({ type: "opened", previewOnly: true });
		expect(container.textContent).toContain(
			"Original layers and raw image data are unavailable",
		);
	});
});

function requireViewer(name: string) {
	const viewer = craftViewer(name);
	if (!viewer) throw new Error(`Missing viewer for ${name}`);
	return viewer;
}

it("waits for a development build without loading a missing iframe or timing out", async () => {
	vi.stubEnv("NODE_ENV", "development");
	vi.useFakeTimers();
	let built = false;
	fetchDocument.mockImplementation(async (url: string) =>
		url.endsWith("runtime.json")
			? new Response("{}", { status: built ? 200 : 404 })
			: new Response(originalBytes),
	);
	await render();
	expect(container.querySelector("iframe")).toBeNull();
	expect(container.textContent).toContain(
		"Waiting for Flows Sheets to finish building",
	);
	await act(async () => {
		await vi.advanceTimersByTimeAsync(95_000);
	});
	expect(container.querySelector('[role="alert"]')).toBeNull();
	built = true;
	await act(async () => {
		await vi.advanceTimersByTimeAsync(2000);
	});
	expect(container.querySelector("iframe")?.src).toContain(
		"/craft/gridcraft/index.html",
	);
	await receive({ type: "ready" });
	expect(messages("open")).toHaveLength(1);
});

describe("blank document creation", () => {
	it("creates without a source download and saves through the configured handler", async () => {
		await render(true, undefined, {
			url: undefined,
			create: true,
			initialEditing: true,
		});
		expect(fetchDocument).not.toHaveBeenCalled();
		await receive({ type: "ready" });
		expect(messages("open")).toHaveLength(0);
		expect(messages("create")).toEqual([
			expect.objectContaining({ name: "report.xlsx", readOnly: false }),
		]);
		expect(messages("create")[0]).not.toHaveProperty("bytes");
		await receive({ type: "opened", revision: 0 });
		await receive({ type: "dirty", dirty: true, revision: 0 });
		await click("Save");
		const request = messages("export").at(-1);
		await receive({
			type: "exported",
			requestId: request?.requestId,
			bytes: editedBytes.buffer.slice(0),
			revision: 0,
		});
		expect(saveDocument).toHaveBeenCalledTimes(1);
		expect(saveDocument.mock.calls[0][0].name).toBe("report.xlsx");
		expect(messages("saved")).toEqual([
			expect.objectContaining({ revision: 0 }),
		]);
	});
	it("downloads the live draft without invoking Save or clearing dirty state", async () => {
		await render(true, undefined, {
			url: undefined,
			create: true,
			initialEditing: true,
		});
		await receive({ type: "ready" });
		await receive({ type: "opened", revision: 0 });
		await receive({ type: "dirty", dirty: true, revision: 0 });
		await act(async () => {
			container.querySelector<HTMLAnchorElement>("a[download]")?.click();
		});
		const request = messages("export").at(-1);
		expect(request).toMatchObject({ purpose: "download" });
		await receive({
			type: "exported",
			requestId: request?.requestId,
			bytes: editedBytes.buffer.slice(0),
			revision: 0,
		});
		expect(downloadCraftFile).toHaveBeenCalledTimes(1);
		const file = vi.mocked(downloadCraftFile).mock.calls[0][0];
		expect(file.name).toBe("report.xlsx");
		expect(new Uint8Array(await file.arrayBuffer())).toEqual(editedBytes);
		expect(saveDocument).not.toHaveBeenCalled();
		expect(messages("saved")).toHaveLength(0);
		expect(dirtyChange).toHaveBeenLastCalledWith(true);
	});
	it.each([true, false])(
		"retains the draft and download after Save revocation (create=%s)",
		async (create) => {
			const props = {
				url: create ? undefined : "/files/report.xlsx",
				create,
				initialEditing: true,
			};
			await render(true, undefined, props);
			await receive({ type: "ready" });
			await receive({ type: "opened" });
			await receive({ type: "dirty", dirty: true, revision: 1 });
			const originalSession = session();
			await render(true, undefined, { ...props, onSave: undefined });
			expect(session()).toBe(originalSession);
			expect(messages(create ? "create" : "open")).toHaveLength(1);
			expect(messages("set-read-only").at(-1)).toMatchObject({
				readOnly: true,
			});
			expect(container.querySelector('[aria-label="Save"]')).toBeNull();
			expect(container.querySelector('[aria-label^="Edit"]')).toBeNull();
			await act(async () => {
				container.querySelector<HTMLAnchorElement>("a[download]")?.click();
			});
			expect(messages("export").at(-1)).toMatchObject({ purpose: "download" });
		},
	);
});
