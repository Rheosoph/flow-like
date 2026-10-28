import { type JsonSchema, validateSchema } from "@flow-like/widget-sdk";
import { type ComponentProps, act, useEffect, useState } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { IHelperState } from "../../../state/backend-state/helper-state";
import { normalizeComponent } from "../../builder/componentDefaults";
import type { CraftDocument } from "../../storage/craft-document";
import type {
	useComponentEventTrigger,
	useOnAction,
	useSetElementValue,
} from "../ActionHandler";
import { applyElementUpdate } from "../apply-a2ui-message";
import { getFilePreviewEventContextSchema } from "../file-preview-events";
import type {
	BoundValue,
	EventHandlers,
	FileEditorComponent,
	FilePreviewComponent,
} from "../types";
import { A2UIFileEditor } from "./FileEditor";
import { A2UIFilePreview } from "./FilePreview";

type EditorProps = Omit<ComponentProps<typeof CraftDocument>, "viewer"> & {
	viewer?: ComponentProps<typeof CraftDocument>["viewer"];
};

const mocks = vi.hoisted(() => ({
	context: {
		appId: "app-1",
		isPreviewMode: true,
		resolveTemporaryUploadTarget: vi.fn(),
	},
	triggerEvent: vi.fn<ReturnType<typeof useComponentEventTrigger>>(),
	onAction: vi.fn<NonNullable<ReturnType<typeof useOnAction>>>(),
	setElementValue: vi.fn<NonNullable<ReturnType<typeof useSetElementValue>>>(),
	setByPath: vi.fn<(path: string, value: unknown) => void>(),
	upload: vi.fn<IHelperState["fileToUrl"]>(),
	boundValues: new Map<string, unknown>(),
	signature: "first",
	editor: undefined as EditorProps | undefined,
	mounts: 0,
	unmounts: 0,
	order: [] as string[],
}));

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));

vi.mock("../../../state/backend-state", () => ({
	useBackend: () => ({ helperState: { fileToUrl: mocks.upload } }),
}));

vi.mock("../ActionHandler", () => ({
	useActionContext: () => mocks.context,
	useComponentEventTrigger: () => mocks.triggerEvent,
	useOnAction: () => mocks.onAction,
	useSetElementValue: () => mocks.setElementValue,
}));

vi.mock("../DataContext", () => ({
	useData: () => ({
		resolve: (value: BoundValue) => {
			if ("path" in value)
				return mocks.boundValues.get(value.path) ?? value.defaultValue;
			if ("literalJson" in value) return JSON.parse(value.literalJson);
			return Object.values(value)[0];
		},
		setByPath: mocks.setByPath,
	}),
}));

vi.mock("../hooks/use-asset-url", () => ({
	useAssetUrl: (source: string | undefined) => ({
		url: source
			? `https://files.example/${source.replace(/^\//, "")}?signature=${mocks.signature}`
			: undefined,
		isLoading: false,
	}),
}));

vi.mock("../../ui/audio-preview", () => ({
	AudioPreview: ({ src }: { src: string }) => <div data-native-audio={src} />,
}));

vi.mock("../../ui/file-previewer", () => ({
	PdfFrame: ({ url }: { url: string }) => <div data-native-pdf={url} />,
}));

vi.mock("../../voice", () => ({
	AudioPlayback: ({ src }: { src: string }) => (
		<div data-audio-playback={src} />
	),
}));

vi.mock("../../storage/craft-document", () => ({
	CraftDocument: (props: ComponentProps<typeof CraftDocument>) => (
		<DocumentStub {...props} kind={props.viewer.id} />
	),
}));

vi.mock("../../storage/wordcraft-document", () => ({
	WordcraftDocument: (props: EditorProps) => (
		<DocumentStub {...props} kind="wordcraft" />
	),
}));

function DocumentStub({ kind, ...props }: EditorProps & { kind: string }) {
	const [mount] = useState(() => ++mocks.mounts);
	mocks.editor = props;
	useEffect(
		() => () => {
			mocks.unmounts++;
		},
		[],
	);
	return (
		<div
			data-document={kind}
			data-mount={mount}
			data-editable={props.editable}
			data-url={props.url}
		>
			{props.filename}
		</div>
	);
}

function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (reason: unknown) => void;
	const promise = new Promise<T>((resolvePromise, rejectPromise) => {
		resolve = resolvePromise;
		reject = rejectPromise;
	});
	return { promise, resolve, reject };
}

const source = "/files/presentation.pptx";
const temporary = {
	url: "https://temporary.example/export.pptx",
};
const saveAction = {
	name: "workflow_event",
	context: { nodeId: "save-event", appId: "app-1" },
};
const editableProps: Partial<FilePreviewComponent> = {
	editable: { literalBool: true },
	value: { path: "$.data.exportedFile" },
	eventHandlers: { save: [saveAction] },
};
let root: Root | null;
let container: HTMLDivElement;
let originalConfirm: PropertyDescriptor | undefined;

beforeEach(() => {
	vi.resetAllMocks();
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	mocks.context.appId = "app-1";
	mocks.context.isPreviewMode = true;
	mocks.context.resolveTemporaryUploadTarget.mockResolvedValue("local");
	mocks.signature = "first";
	mocks.editor = undefined;
	mocks.mounts = 0;
	mocks.unmounts = 0;
	mocks.order = [];
	mocks.boundValues.clear();
	mocks.upload.mockImplementation(async () => {
		mocks.order.push("upload");
		return temporary.url;
	});
	mocks.setElementValue.mockImplementation(() => {
		mocks.order.push("element");
	});
	mocks.setByPath.mockImplementation((path, value) => {
		mocks.order.push("binding");
		mocks.boundValues.set(path, value);
	});
	mocks.onAction.mockImplementation((message) => {
		if (message.type === "userAction")
			mocks.order.push(`action:${message.name}`);
	});
	mocks.triggerEvent.mockImplementation(async (name) => {
		mocks.order.push(`event:${name}`);
	});
	originalConfirm = Object.getOwnPropertyDescriptor(window, "confirm");
	Object.defineProperty(window, "confirm", {
		configurable: true,
		value: vi.fn().mockReturnValue(false),
	});
	vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("Plain text")));
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	if (root) await act(async () => root?.unmount());
	root = null;
	container?.remove();
	if (originalConfirm)
		Object.defineProperty(window, "confirm", originalConfirm);
	else Reflect.deleteProperty(window, "confirm");
	vi.restoreAllMocks();
	vi.unstubAllGlobals();
});

async function render(overrides: Partial<FilePreviewComponent> = {}) {
	await act(async () => {
		root?.render(
			<A2UIFilePreview
				component={{
					id: "preview",
					type: "filePreview",
					src: { literalString: source },
					...overrides,
				}}
				componentId="preview"
				surfaceId="page-1"
				renderChild={() => null}
			/>,
		);
	});
	await act(async () => vi.dynamicImportSettled());
}

async function renderEditor(overrides: Partial<FileEditorComponent> = {}) {
	await act(async () => {
		root?.render(
			<A2UIFileEditor
				component={{
					id: "preview",
					type: "fileEditor",
					editor: { literalString: "solidcraft" },
					...overrides,
				}}
				componentId="preview"
				surfaceId="page-1"
				renderChild={() => null}
			/>,
		);
	});
	await act(async () => vi.dynamicImportSettled());
}

function editor() {
	if (!mocks.editor || !container.querySelector("[data-document]"))
		throw new Error("No document editor is mounted");
	return mocks.editor;
}

function saveFile(
	file: File,
	onProgress?: Parameters<NonNullable<EditorProps["onSave"]>>[1],
) {
	const onSave = editor().onSave;
	if (!onSave) throw new Error("No Save workflow is configured");
	return onSave(file, onProgress);
}

function editedFile() {
	return new File(["edited presentation"], "updated.pptx", {
		type: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
	});
}

function eventNames() {
	return mocks.triggerEvent.mock.calls.map(([name]) => name);
}

describe("A2UI file creation", () => {
	it.each([
		["wordcraft", "docx"],
		["pdfcraft", "pdf"],
		["deckcraft", "pptx"],
		["gridcraft", "xlsx"],
		["vectorcraft", "vectorcraft"],
		["photocraft", "pcraft"],
		["soundcraft", "scraftz"],
		["filmcraft", "fcproj"],
		["designcraft", "designcraft"],
		["cadcraft", "dxf"],
		["solidcraft", "solidcraft"],
	])(
		"creates a blank %s document when its Save workflow is configured",
		async (id, extension) => {
			await renderEditor({
				editor: { literalString: id },
				eventHandlers: { save: [saveAction] },
			});
			expect(
				container
					.querySelector("[data-document]")
					?.getAttribute("data-document"),
			).toBe(id);
			expect(editor()).toMatchObject({
				create: true,
				filename: `Untitled.${extension}`,
				editable: true,
				initialEditing: true,
				showDownload: true,
				toolTarget: {
					appId: "app-1",
					surfaceId: "page-1",
					componentId: "preview",
				},
			});
			expect(editor().url).toBeUndefined();
			expect(fetch).not.toHaveBeenCalled();
			expect(mocks.upload).not.toHaveBeenCalled();
		},
	);

	it.each<EventHandlers | undefined>([
		undefined,
		{ "*": [saveAction] },
		{ save: [] },
		{ save: [{ name: "navigate_page", context: { route: "/" } }] },
	])(
		"shows setup without opening a blank editor until an exact Save workflow exists: %j",
		async (eventHandlers) => {
			await renderEditor({ eventHandlers });
			expect(container.querySelector("[data-document]")).toBeNull();
			expect(container.textContent).toContain("Connect a Save workflow");
			expect(fetch).not.toHaveBeenCalled();
		},
	);

	it.each([false, true])(
		"loads an optional initial model after normalization=%s",
		async (normalized) => {
			const component: FileEditorComponent = {
				id: "preview",
				type: "fileEditor",
				editor: { literalString: "solidcraft" },
				url: { literalString: "/files/part.step" },
				eventHandlers: { save: [saveAction] },
			};
			await renderEditor(
				normalized
					? (normalizeComponent(component) as FileEditorComponent)
					: component,
			);
			expect(editor()).toMatchObject({
				create: false,
				filename: "part.step",
				editable: true,
				initialEditing: true,
				viewer: { id: "solidcraft", extension: "step" },
				url: expect.stringContaining("/files/part.step"),
			});
		},
	);

	it("keeps a source image on its native preview until saving is configured", async () => {
		await renderEditor({
			editor: { literalString: "photocraft" },
			src: { literalString: "/files/picture.png" },
		});
		expect(container.querySelector("img")).not.toBeNull();
		expect(container.querySelector("[data-document]")).toBeNull();
	});

	it("keeps File Preview source-based even when editable is enabled", async () => {
		await render({ src: undefined, ...editableProps });
		expect(container.querySelector("[data-document]")).toBeNull();
	});

	it("retains a blank draft read-only when the Save workflow is removed", async () => {
		await renderEditor({ eventHandlers: { save: [saveAction] } });
		const mount = mocks.mounts;
		await act(async () => editor().onDirtyChange?.(true));
		await renderEditor();
		expect(mocks.mounts).toBe(mount);
		expect(editor()).toMatchObject({ create: true, editable: false });
		expect(editor().onSave).toBeUndefined();
	});

	it("saves a new document through upload and the existing bound event contract", async () => {
		await renderEditor({
			value: { path: "$.data.createdFile" },
			eventHandlers: { save: [saveAction] },
		});
		const file = new File(["native model"], "Untitled.solidcraft", {
			type: "application/octet-stream",
		});
		await act(async () => saveFile(file));
		expect(mocks.upload).toHaveBeenCalledWith(file, false);
		expect(mocks.triggerEvent).toHaveBeenCalledWith(
			"save",
			expect.anything(),
			expect.objectContaining({
				src: "",
				filename: file.name,
				file: expect.objectContaining({ name: file.name, url: temporary.url }),
			}),
			expect.objectContaining({ requireSuccess: true }),
		);
		expect(mocks.setByPath).toHaveBeenCalledWith(
			"$.data.createdFile",
			expect.objectContaining({ name: file.name }),
		);
		expect(eventNames()).toEqual(["save", "saved"]);
	});

	it("keeps Set Media Source updates compatible with the selected editor", async () => {
		const updated = applyElementUpdate(
			{
				id: "preview",
				component: {
					id: "preview",
					type: "fileEditor",
					editor: { literalString: "solidcraft" },
				},
			},
			{
				type: "setMediaSource",
				src: "/files/part.step",
				filename: "part.step",
				mimeType: "model/step",
				mediaKind: "file",
			},
		);
		expect(updated.component).not.toHaveProperty("fileType");
		await renderEditor({
			...(updated.component as FileEditorComponent),
			eventHandlers: { save: [saveAction] },
		});
		expect(editor()).toMatchObject({
			create: false,
			viewer: { id: "solidcraft", extension: "step" },
		});
	});
});

describe("A2UI file preview routing", () => {
	it.each([
		["presentation.pptx", "application/octet-stream", "file", "deckcraft"],
		["letter.docx", "application/octet-stream", "file", "wordcraft"],
		["report.xlsx", "application/octet-stream", "file", "gridcraft"],
		["report.csv", "text/csv", "text", "gridcraft"],
		["part.step", "model/step", "file", "solidcraft"],
		["part.solidcraft", "application/octet-stream", "file", "solidcraft"],
	])(
		"previews %s read-only after the Set Media Source node update",
		async (filename, mimeType, mediaKind, viewer) => {
			const updated = applyElementUpdate(
				{
					id: "preview",
					component: { id: "preview", type: "filePreview" },
				},
				{
					type: "setMediaSource",
					src: "https://storage.example/object?signature=node",
					url: "https://storage.example/object?signature=node",
					filename,
					mimeType,
					mediaKind,
				},
			);
			await render(updated.component as FilePreviewComponent);
			expect(editor()).toMatchObject({ filename, editable: false });
			expect(
				container
					.querySelector("[data-document]")
					?.getAttribute("data-document"),
			).toBe(viewer);
			expect(fetch).not.toHaveBeenCalled();
			expect(mocks.upload).not.toHaveBeenCalled();
		},
	);

	it.each([
		["presentation.pptx", "deckcraft"],
		["letter.docx", "wordcraft"],
		["report.xlsx", "gridcraft"],
		["report.csv", "gridcraft"],
		["part.step", "solidcraft"],
		["part.glb", "solidcraft"],
		["part.solidcraft", "solidcraft"],
	])("opens %s in its WASM viewer by default", async (filename, viewer) => {
		await render({ src: { literalString: `/files/${filename}` } });
		expect(
			container.querySelector("[data-document]")?.getAttribute("data-document"),
		).toBe(viewer);
		expect(editor()).toMatchObject({
			filename,
			editable: false,
			initialEditing: false,
			showDownload: false,
		});
		expect(fetch).not.toHaveBeenCalled();
		expect(mocks.upload).not.toHaveBeenCalled();
	});

	it.each([
		["model/step", "step"],
		["model/gltf+json", "gltf"],
	])(
		"opens an opaque CAD source with MIME type %s",
		async (mime, extension) => {
			await render({
				src: { literalString: "/files/download" },
				mimeType: { literalString: mime },
			});
			expect(editor()).toMatchObject({
				filename: `download.${extension}`,
				viewer: { id: "solidcraft", outputExtension: "solidcraft" },
				editable: false,
			});
			expect(container.querySelector("code")).toBeNull();
			expect(fetch).not.toHaveBeenCalled();
		},
	);

	it("leaves legacy presentations on the unsupported fallback", async () => {
		await render({
			src: { literalString: "/files/legacy.ppt" },
			...editableProps,
		});
		expect(container.querySelector("[data-document]")).toBeNull();
		expect(container.textContent).toContain("Cannot preview this file");
	});

	it.each([
		["report.pdf", "[data-native-pdf]"],
		["picture.png", "img"],
		["vector.svg", "img"],
		["recording.wav", "[data-native-audio]"],
		["clip.mp4", "video"],
	])("preserves the native %s preview", async (filename, selector) => {
		await render({ src: { literalString: `/files/${filename}` } });
		expect(container.querySelector(selector)).not.toBeNull();
		expect(container.querySelector("[data-document]")).toBeNull();
	});

	it.each([
		["layers.psd", "image/vnd.adobe.photoshop", "photocraft"],
		["drawing.emf", "image/emf", "vectorcraft"],
		["drawing.dxf", "image/vnd.dxf", "cadcraft"],
	])(
		"routes %s to WASM despite its image MIME type",
		async (filename, mimeType, viewer) => {
			await render({
				src: { literalString: `/files/${filename}` },
				mimeType: { literalString: mimeType },
			});
			expect(editor()).toMatchObject({ filename, viewer: { id: viewer } });
			expect(container.querySelector("img")).toBeNull();
		},
	);

	it("keeps CSV as text when explicitly requested without editing", async () => {
		await render({
			src: { literalString: "/files/report.csv" },
			fileType: { literalString: "text" },
		});
		expect(container.querySelector("[data-document]")).toBeNull();
		expect(container.querySelector("code")?.textContent).toBe("Plain text");
		expect(fetch).toHaveBeenCalledWith(
			"https://files.example/files/report.csv?signature=first",
			{ signal: expect.any(AbortSignal) },
		);
	});

	it("accepts a source supplied through the url alias", async () => {
		await render({ src: undefined, url: { literalString: source } });
		expect(editor().filename).toBe("presentation.pptx");
	});

	it("binds agent control to the mounted app, surface and component", async () => {
		await render({ src: { literalString: "/files/part.solidcraft" } });
		expect(editor().toolTarget).toEqual({
			appId: "app-1",
			surfaceId: "page-1",
			componentId: "preview",
		});
	});

	it("uses a document override to select a viewer for an opaque URL", async () => {
		await render({
			src: { literalString: "/files/download" },
			mimeType: {
				literalString:
					"application/vnd.openxmlformats-officedocument.presentationml.presentation",
			},
			fileType: { literalString: "document" },
		});
		expect(editor()).toMatchObject({
			filename: "download.pptx",
			viewer: { id: "deckcraft" },
		});
	});

	it.each<Partial<FilePreviewComponent>>([
		{},
		{ actions: [saveAction] },
		{ eventHandlers: { "*": [saveAction] } },
		{ eventHandlers: { save: [] } },
		{
			eventHandlers: {
				save: [saveAction, { name: "navigate_page", context: { route: "/" } }],
			},
		},
	])(
		"requires an exact save workflow before enabling editing: %j",
		async (handlers) => {
			await render({
				editable: { literalBool: true },
				initialEditing: { literalBool: true },
				...handlers,
			});
			expect(editor().editable).toBe(false);
		},
	);

	it.each(["workflow_event", "widget_event"])(
		"enables editing with an exact %s save handler",
		async (name) => {
			await render({
				...editableProps,
				initialEditing: { literalBool: true },
				eventHandlers: { save: [{ name, context: {} }] },
			});
			expect(editor()).toMatchObject({ editable: true, initialEditing: true });
		},
	);

	it("keeps the editor read-only in the builder", async () => {
		mocks.context.isPreviewMode = false;
		await render(editableProps);
		expect(editor().editable).toBe(false);
		await expect(saveFile(editedFile())).rejects.toThrow(
			"Editing is no longer enabled",
		);
		expect(mocks.upload).not.toHaveBeenCalled();
	});
});

describe("A2UI document save actions", () => {
	it("uploads, publishes bound metadata, and waits for save success before saved", async () => {
		const completion = deferred<void>();
		mocks.triggerEvent.mockImplementation(async (name) => {
			mocks.order.push(`event:${name}`);
			if (name === "save") await completion.promise;
		});
		const secondAction = {
			name: "widget_event",
			context: { actionId: "save" },
		};
		await render({
			...editableProps,
			eventHandlers: { save: [saveAction, secondAction] },
		});
		const file = editedFile();
		const onProgress = vi.fn();
		let saving!: Promise<void>;
		await act(async () => {
			saving = saveFile(file, onProgress);
		});
		const value = {
			name: file.name,
			size: file.size,
			type: file.type,
			url: temporary.url,
			backendUrl: temporary.url,
		};
		expect(mocks.upload).toHaveBeenCalledExactlyOnceWith(file, false);
		expect(mocks.context.resolveTemporaryUploadTarget).not.toHaveBeenCalled();
		expect(mocks.setElementValue).toHaveBeenCalledWith("page-1/preview", value);
		expect(mocks.setByPath).toHaveBeenCalledWith("$.data.exportedFile", value);
		expect(mocks.boundValues.get("$.data.exportedFile")).toEqual(value);
		expect(mocks.triggerEvent).toHaveBeenCalledWith(
			"save",
			{ eventHandlers: { save: [saveAction, secondAction] } },
			{
				actionId: "save",
				url: temporary.url,
				signedUrls: temporary.url,
				file: value,
				value,
				src: source,
				filename: file.name,
			},
			{ legacyFallback: false, wildcardFallback: false, requireSuccess: true },
		);
		expect(mocks.order).toEqual([
			"upload",
			"element",
			"binding",
			"action:save",
			"event:save",
		]);
		expect(eventNames()).not.toContain("saved");
		await act(async () => {
			completion.resolve();
			await saving;
		});
		expect(eventNames()).toEqual(["save", "saved"]);
		expect(mocks.triggerEvent).toHaveBeenLastCalledWith(
			"saved",
			expect.anything(),
			{
				actionId: "saved",
				url: temporary.url,
				signedUrls: temporary.url,
				file: value,
				value,
				src: source,
				filename: file.name,
			},
			{ legacyFallback: false, wildcardFallback: false },
		);
		expect(editor().url).toContain("/files/presentation.pptx");
	});

	it("does not emit saved when the save workflow rejects", async () => {
		mocks.triggerEvent.mockRejectedValueOnce(new Error("Destination rejected"));
		await render(editableProps);
		await expect(saveFile(editedFile())).rejects.toThrow(
			"Destination rejected",
		);
		expect(eventNames()).toEqual(["save"]);
		expect(mocks.setElementValue).toHaveBeenCalledTimes(1);
		expect(
			mocks.onAction.mock.calls.map(
				([message]) => message.type === "userAction" && message.name,
			),
		).not.toContain("saved");
	});

	it("does not publish metadata or run save when uploading fails", async () => {
		mocks.upload.mockRejectedValueOnce(new Error("Upload failed"));
		await render(editableProps);
		await expect(saveFile(editedFile())).rejects.toThrow("Upload failed");
		expect(mocks.setElementValue).not.toHaveBeenCalled();
		expect(mocks.setByPath).not.toHaveBeenCalled();
		expect(mocks.triggerEvent).not.toHaveBeenCalled();
	});

	it.each([
		"",
		"blob:https://app.example/local-document",
		"data:application/octet-stream;base64,AA==",
		"file:///tmp/document.pptx",
		"/tmp/document.pptx",
	])(
		"rejects a temporary reference that is not a backend URL: %j",
		async (url) => {
			mocks.upload.mockResolvedValueOnce(url);
			await render(editableProps);
			await expect(saveFile(editedFile())).rejects.toThrow();
			expect(mocks.setElementValue).not.toHaveBeenCalled();
			expect(mocks.setByPath).not.toHaveBeenCalled();
			expect(mocks.triggerEvent).not.toHaveBeenCalled();
		},
	);

	it("uploads online even when the save workflow executes locally", async () => {
		mocks.context.resolveTemporaryUploadTarget.mockResolvedValue("local");
		await render(editableProps);
		const file = editedFile();
		await act(async () => saveFile(file));
		expect(mocks.upload).toHaveBeenCalledExactlyOnceWith(file, false);
		expect(mocks.context.resolveTemporaryUploadTarget).not.toHaveBeenCalled();
	});

	it("ignores an upload completing after unmount without dispatching the workflow", async () => {
		const upload = deferred<string>();
		mocks.upload.mockReturnValueOnce(upload.promise);
		await render(editableProps);
		const file = editedFile();
		const saving = saveFile(file).catch((error: Error) => error);
		await act(async () => {
			await Promise.resolve();
		});
		expect(mocks.upload).toHaveBeenCalledExactlyOnceWith(file, false);
		await act(async () => root?.unmount());
		root = null;
		upload.resolve(temporary.url);
		expect(await saving).toMatchObject({
			message: "The document closed before saving finished.",
		});
		expect(mocks.triggerEvent).not.toHaveBeenCalled();
		expect(mocks.setElementValue).not.toHaveBeenCalled();
	});

	it("emits exact dirty and error notifications from the editor", async () => {
		await render(editableProps);
		await act(async () => {
			editor().onDirtyChange?.(true);
			editor().onDirtyChange?.(true);
			editor().onError?.("Export failed");
		});
		expect(eventNames()).toEqual(["dirtyChange", "error"]);
		expect(mocks.triggerEvent).toHaveBeenCalledWith(
			"dirtyChange",
			expect.anything(),
			{
				actionId: "dirtyChange",
				dirty: true,
				src: source,
				filename: "presentation.pptx",
			},
			{ legacyFallback: false, wildcardFallback: false },
		);
		expect(mocks.triggerEvent).toHaveBeenCalledWith(
			"error",
			expect.anything(),
			{
				actionId: "error",
				message: "Export failed",
				src: source,
				filename: "presentation.pptx",
			},
			{ legacyFallback: false, wildcardFallback: false },
		);
	});

	it("emits contexts that match the shared workflow event schemas", async () => {
		await render(editableProps);
		await act(async () => {
			editor().onDirtyChange?.(true);
			editor().onError?.("Export failed");
			await saveFile(editedFile());
		});
		expect(eventNames()).toEqual(["dirtyChange", "error", "save", "saved"]);
		for (const [name, , context] of mocks.triggerEvent.mock.calls) {
			const schema = getFilePreviewEventContextSchema(name);
			expect(schema).toBeDefined();
			expect(validateSchema(schema, context)).toEqual({
				valid: true,
				errors: [],
			});
			expect(context?.actionId).toBe(name);
			// The widget validator handles direct fields; check the shared file definition too.
			if (name === "save" || name === "saved") {
				const definitions = schema?.$defs as Record<string, JsonSchema>;
				for (const key of ["file", "value"]) {
					expect(
						validateSchema(definitions.FilePreviewFile, context?.[key]).valid,
					).toBe(true);
				}
			}
		}
	});
});

describe("A2UI document session preservation", () => {
	it("keeps the opened editor and its URL when a storage signature renews", async () => {
		await render(editableProps);
		const node = container.querySelector("[data-document]");
		const originalUrl = editor().url;
		await act(async () => {
			editor().onReady?.();
			editor().onDirtyChange?.(true);
		});
		mocks.signature = "renewed";
		await render(editableProps);
		expect(editor().url).toBe(originalUrl);
		expect(container.querySelector("[data-document]")).toBe(node);
		expect(mocks.mounts).toBe(1);
		expect(window.confirm).not.toHaveBeenCalled();
	});

	it("adopts a renewed URL before the viewer has opened the document", async () => {
		await render();
		expect(editor().url).toContain("signature=first");
		mocks.signature = "renewed";
		await render();
		expect(editor().url).toContain("signature=renewed");
	});

	it("retains an opened media editor as read-only when editing is revoked", async () => {
		const media = { src: { literalString: "/files/picture.png" } };
		await render({ ...editableProps, ...media });
		const previousSave = editor().onSave;
		if (!previousSave) throw new Error("Expected a configured Save workflow");
		const node = container.querySelector("[data-document]");
		await act(async () => {
			editor().onReady?.();
			editor().onDirtyChange?.(true);
		});
		await render({ ...media, editable: { literalBool: false } });
		expect(container.querySelector("[data-document]")).toBe(node);
		expect(editor().editable).toBe(false);
		expect(container.querySelector("img")).toBeNull();
		expect(editor().onSave).toBeUndefined();
		await expect(previousSave(editedFile())).rejects.toThrow(
			"Editing is no longer enabled",
		);
		expect(mocks.upload).not.toHaveBeenCalled();
	});

	it("retains a dirty document when a source change is declined", async () => {
		await render(editableProps);
		await act(async () => editor().onDirtyChange?.(true));
		const node = container.querySelector("[data-document]");
		await render({
			...editableProps,
			src: { literalString: "/files/replacement.pptx" },
		});
		expect(window.confirm).toHaveBeenCalledWith(
			"Discard unsaved document changes?",
		);
		expect(container.querySelector("[data-document]")).toBe(node);
		expect(editor().filename).toBe("presentation.pptx");
	});

	it("saves to the original destination and binding after declining a source change", async () => {
		const originalAction = {
			name: "workflow_event",
			context: { nodeId: "save-event", destination: "A" },
		};
		await render({
			...editableProps,
			value: { path: "$.data.A.exportedFile" },
			eventHandlers: { save: [originalAction] },
		});
		await act(async () => editor().onDirtyChange?.(true));
		await render({
			...editableProps,
			src: { literalString: "/files/replacement.pptx" },
			value: { path: "$.data.B.exportedFile" },
			eventHandlers: {
				save: [
					{
						name: "workflow_event",
						context: { nodeId: "save-event", destination: "B" },
					},
				],
			},
		});
		expect(window.confirm).toHaveBeenCalledTimes(1);
		expect(editor().filename).toBe("presentation.pptx");
		await act(async () => saveFile(editedFile()));
		expect(mocks.triggerEvent).toHaveBeenCalledWith(
			"save",
			{ eventHandlers: { save: [originalAction] } },
			expect.objectContaining({ src: source }),
			{ legacyFallback: false, wildcardFallback: false, requireSuccess: true },
		);
		expect(mocks.setByPath).toHaveBeenCalledOnce();
		expect(mocks.setByPath).toHaveBeenCalledWith(
			"$.data.A.exportedFile",
			expect.objectContaining({ url: temporary.url }),
		);
		expect(mocks.boundValues.has("$.data.B.exportedFile")).toBe(false);
	});

	it("opens the replacement after the user confirms discarding dirty changes", async () => {
		vi.mocked(window.confirm).mockReturnValue(true);
		await render(editableProps);
		await act(async () => editor().onDirtyChange?.(true));
		const node = container.querySelector("[data-document]");
		await render({
			...editableProps,
			src: { literalString: "/files/replacement.pptx" },
		});
		expect(window.confirm).toHaveBeenCalledTimes(1);
		expect(container.querySelector("[data-document]")).not.toBe(node);
		expect(editor().filename).toBe("replacement.pptx");
		expect(mocks.unmounts).toBe(1);
	});
});
