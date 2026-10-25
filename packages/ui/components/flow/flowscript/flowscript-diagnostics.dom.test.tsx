// @vitest-environment happy-dom

import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type {
	IApplyFlowScriptResponse,
	ICheckFlowScriptReconcileResponse,
} from "../../../state/backend-state/board-state";
import { FLOWSCRIPT_RECONCILE_DIAGNOSTIC_OWNER } from "./flowscript-diagnostics";
import { FLOWSCRIPT_DIAGNOSTIC_OWNER } from "./flowscript-language";
import { FlowScriptPanel } from "./flowscript-panel";

const harness = vi.hoisted(() => {
	const disposable = () => ({ dispose: vi.fn() });
	const model = {
		text: "",
		getValue: () => model.text,
		setValue: (value: string) => {
			model.text = value;
		},
		getLineCount: () => model.text.split("\n").length,
		getLineMaxColumn: (line: number) =>
			(model.text.split("\n")[line - 1]?.length ?? 0) + 1,
		isDisposed: () => false,
	};
	const editor = {
		getModel: vi.fn(() => model),
		getPosition: () => ({ lineNumber: 1, column: 1 }),
		getSelection: () => null,
		getVisibleRanges: () => [],
		hasTextFocus: () => false,
		createDecorationsCollection: () => ({ set: vi.fn(), clear: vi.fn() }),
		updateOptions: vi.fn(),
		setSelection: vi.fn(),
		revealRangeInCenter: vi.fn(),
		focus: vi.fn(),
		addCommand: vi.fn(),
		addAction: disposable,
		onKeyDown: disposable,
		onMouseDown: disposable,
		onDidChangeModelContent: disposable,
		onDidCompositionStart: disposable,
		onDidCompositionEnd: disposable,
		onDidBlurEditorText: disposable,
		onDidFocusEditorText: disposable,
		onDidChangeCursorPosition: disposable,
		onDidChangeCursorSelection: disposable,
		onDidScrollChange: disposable,
	};
	const monaco = {
		MarkerSeverity: { Error: 8, Warning: 4 },
		KeyMod: { CtrlCmd: 2048, Alt: 512 },
		KeyCode: { KeyS: 49, KeyG: 37 },
		editor: {
			setTheme: vi.fn(),
			setModelMarkers: vi.fn(),
			TrackedRangeStickiness: { NeverGrowsWhenTypingAtEdges: 1 },
			InjectedTextCursorStops: { None: 0 },
			MouseTargetType: { GUTTER_LINE_DECORATIONS: 4 },
		},
	};
	return {
		model,
		editor,
		monaco,
		backend: {
			boardState: {
				getFlowScript: vi.fn(),
				lintFlowScript: vi.fn(),
				checkFlowScriptReconcile: vi.fn(),
			},
			userState: { lookupUser: vi.fn() },
		},
	};
});

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: translate }),
}));
vi.mock("../../../state/backend-state", () => ({
	useBackend: () => harness.backend,
}));
vi.mock("../../../hooks/use-developer-mode", () => ({
	useDeveloperMode: () => ({ developerMode: false }),
}));
vi.mock("@monaco-editor/react", async () => {
	const { useEffect } = await import("react");
	return {
		default: ({
			value,
			onChange,
			onMount,
		}: {
			value: string;
			onChange: (value: string) => void;
			onMount: (editor: unknown, monaco: unknown) => void;
		}) => {
			harness.model.text = value;
			useEffect(() => {
				onMount(harness.editor, harness.monaco);
			}, [onMount]);
			return (
				<textarea
					aria-label="FlowScript editor"
					value={value}
					onChange={(event) => {
						harness.model.text = event.currentTarget.value;
						onChange(event.currentTarget.value);
					}}
				/>
			);
		},
	};
});
vi.mock("../../ui", async () => ({
	...(await import("../../ui/alert-dialog")),
	...(await import("../../ui/badge")),
	...(await import("../../ui/button")),
	...(await import("../../ui/tooltip")),
}));
vi.mock("./flowscript-comment-widgets", () => ({
	FlowScriptCommentOverlay: () => null,
}));
vi.mock("./flowscript-language", async (original) => ({
	...(await original<Record<string, unknown>>()),
	registerFlowScriptLanguage: vi.fn(),
	defineFlowScriptThemes: vi.fn(),
}));
vi.mock("./flowscript-language-providers", () => ({
	registerFlowScriptProviders: () => ({ dispose: vi.fn() }),
}));
vi.mock("./flowscript-merge", async (original) => ({
	...(await original<Record<string, unknown>>()),
	registerFlowScriptConflictLens: () => ({
		dispose: vi.fn(),
		refresh: vi.fn(),
	}),
}));
vi.mock("./flowscript-worker-client", () => ({
	computeFlowScriptMarkersPreferWorker: () => [],
}));

function translate(key: string, fallback: string | Record<string, unknown>) {
	if (typeof fallback === "string") return fallback;
	const template =
		(fallback.count === 1
			? (fallback.defaultValue_one ?? fallback.defaultValue)
			: (fallback.defaultValue_other ?? fallback.defaultValue)) ?? key;
	return String(template).replace(/\{\{(\w+)\}\}/g, (_, name: string) =>
		String(fallback[name] ?? ""),
	);
}

let root: Root;
let container: HTMLDivElement;
const apply = vi.fn<() => Promise<IApplyFlowScriptResponse | undefined>>();
const source = "function send_emails() {\n  push({ email: address });\n}";
const message = "node `push` has no input pin named `email`";
const rejected: IApplyFlowScriptResponse = {
	commands: [],
	board_commands: [],
	diagnostics: [message],
	structured_diagnostics: [
		{
			id: "pin-error",
			message,
			scope: "function:send_emails",
			source_span_is_exact: true,
			source_span: {
				start: { line: 2, column: 10 },
				end: { line: 2, column: 15 },
			},
		},
	],
};

beforeEach(async () => {
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	vi.useFakeTimers();
	vi.clearAllMocks();
	harness.editor.getModel.mockReturnValue(harness.model);
	harness.backend.boardState.getFlowScript
		.mockReset()
		.mockResolvedValue("original");
	harness.backend.boardState.lintFlowScript.mockReset().mockResolvedValue([]);
	harness.backend.boardState.checkFlowScriptReconcile
		.mockReset()
		.mockResolvedValue({
			parse_valid: true,
			reconcile_valid: true,
			idempotent: true,
			command_count: 0,
			corrections: [],
			diagnostics: [],
		});
	apply.mockReset().mockResolvedValue(rejected);
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
	await act(async () => {
		root.render(
			<FlowScriptPanel
				appId="app"
				boardId="board"
				onApplyFlowScript={apply}
				onClose={() => {}}
			/>,
		);
	});
	await edit(source);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	vi.useRealTimers();
	vi.unstubAllGlobals();
});

async function edit(text: string) {
	await act(async () => {
		const element = container.querySelector("textarea");
		if (!element) throw new Error("FlowScript editor not found");
		Object.getOwnPropertyDescriptor(
			HTMLTextAreaElement.prototype,
			"value",
		)?.set?.call(element, text);
		element.dispatchEvent(new Event("input", { bubbles: true }));
	});
}

function button(label: string) {
	const element = [...container.querySelectorAll("button")].find(
		(element) => element.textContent?.trim() === label,
	);
	if (!element) throw new Error(`Button not found: ${label}`);
	return element;
}

async function click(label: string) {
	await act(async () => button(label).click());
}

function markers() {
	return harness.monaco.editor.setModelMarkers.mock.calls
		.filter(([, owner]) => owner === FLOWSCRIPT_RECONCILE_DIAGNOSTIC_OWNER)
		.at(-1)?.[2];
}

test("rejected Apply shows function and location, highlights the token, and navigates on click", async () => {
	await click("Apply to board");
	expect(container.querySelector('[role="alert"]')?.textContent).toContain(
		"function send_emails:",
	);
	expect(markers()).toEqual([
		{
			message: `function:send_emails: ${message}`,
			severity: 8,
			startLineNumber: 2,
			startColumn: 10,
			endLineNumber: 2,
			endColumn: 15,
		},
	]);
	await click("Line 2, column 10");
	const range = {
		startLineNumber: 2,
		startColumn: 10,
		endLineNumber: 2,
		endColumn: 15,
	};
	expect(harness.editor.setSelection).toHaveBeenLastCalledWith(range);
	expect(harness.editor.revealRangeInCenter).toHaveBeenLastCalledWith(range);
	expect(harness.editor.focus).toHaveBeenCalledOnce();

	await act(async () => vi.advanceTimersByTimeAsync(300));
	expect(markers()).toHaveLength(1);
	expect(
		harness.monaco.editor.setModelMarkers.mock.calls.some(
			([, owner]) => owner !== FLOWSCRIPT_RECONCILE_DIAGNOSTIC_OWNER,
		),
	).toBe(true);
});

test("editing a rejected draft clears its markers and removes its location links", async () => {
	await click("Apply to board");
	await edit(`// changed\n${source}`);
	expect(markers()).toEqual([]);
	expect(
		[...container.querySelectorAll("button")].some((element) =>
			element.textContent?.includes("Line 2, column 10"),
		),
	).toBe(false);
	expect(container.textContent).toContain(message);
});

test("an Apply response for an edited buffer cannot install stale markers or links", async () => {
	let resolve!: (response: IApplyFlowScriptResponse) => void;
	apply.mockImplementationOnce(
		() =>
			new Promise((done) => {
				resolve = done;
			}),
	);
	await click("Apply to board");
	await edit(`// changed\n${source}`);
	await act(async () => resolve(rejected));
	expect(markers()).toEqual([]);
	expect(
		[...container.querySelectorAll("button")].some((element) =>
			element.textContent?.includes("Line 2, column 10"),
		),
	).toBe(false);
});

test("an Apply response for a replaced model cannot annotate an identical buffer", async () => {
	let resolve!: (response: IApplyFlowScriptResponse) => void;
	apply.mockImplementationOnce(
		() =>
			new Promise((done) => {
				resolve = done;
			}),
	);
	await click("Apply to board");
	harness.editor.getModel.mockReturnValue({ ...harness.model });
	await act(async () => resolve(rejected));
	expect(markers()).toEqual([]);
	expect(container.querySelector('[role="alert"]')).toBeNull();
});

test("native lint positions after an emoji use Monaco UTF16 columns", async () => {
	await edit('const label = "🙂"; bad_name');
	harness.backend.boardState.lintFlowScript.mockResolvedValueOnce([
		{ message: "unknown name", line: 1, col: 20, severity: "error" },
	]);
	await act(async () => vi.advanceTimersByTimeAsync(300));
	const nativeMarkers = harness.monaco.editor.setModelMarkers.mock.calls
		.filter(([, owner]) => owner === FLOWSCRIPT_DIAGNOSTIC_OWNER)
		.at(-1)?.[2];
	expect(nativeMarkers).toEqual([
		{
			message: "unknown name",
			severity: 8,
			startLineNumber: 1,
			startColumn: 21,
			endLineNumber: 1,
			endColumn: 29,
		},
	]);
});

test("reconcile preview installs validation markers and clickable locations before Apply", async () => {
	harness.backend.boardState.checkFlowScriptReconcile.mockResolvedValueOnce({
		parse_valid: true,
		reconcile_valid: false,
		idempotent: false,
		command_count: 0,
		corrections: [],
		diagnostics: rejected.diagnostics,
		structured_diagnostics: rejected.structured_diagnostics,
	} satisfies ICheckFlowScriptReconcileResponse);
	await act(async () => vi.advanceTimersByTimeAsync(300));
	expect(apply).not.toHaveBeenCalled();
	expect(markers()).toEqual([
		expect.objectContaining({
			startLineNumber: 2,
			startColumn: 10,
			severity: 8,
		}),
	]);
	await click("1 error");
	await click("Line 2, column 10");
	expect(harness.editor.setSelection).toHaveBeenLastCalledWith(
		expect.objectContaining({ startLineNumber: 2, startColumn: 10 }),
	);
});

test("identical errors retain distinct links and markers throughout a long script", async () => {
	const lines = source.split("\n");
	while (lines.length < 1199) lines.push("");
	lines.push("push({ email: other });");
	await edit(lines.join("\n"));
	apply.mockResolvedValueOnce({
		...rejected,
		diagnostics: [message, message],
		structured_diagnostics: [
			...(rejected.structured_diagnostics ?? []),
			{
				id: "second-pin-error",
				message,
				source_span_is_exact: true,
				source_span: {
					start: { line: 1200, column: 8 },
					end: { line: 1200, column: 13 },
				},
			},
		],
	});
	await click("Apply to board");
	expect(
		container.querySelector('[role="alert"]')?.querySelectorAll("li"),
	).toHaveLength(2);
	expect(markers()).toHaveLength(2);
	await click("Line 1200, column 8");
	expect(harness.editor.setSelection).toHaveBeenLastCalledWith({
		startLineNumber: 1200,
		startColumn: 8,
		endLineNumber: 1200,
		endColumn: 13,
	});
});

test("candidate locations remain navigable without marking them as confirmed errors", async () => {
	apply.mockResolvedValueOnce({
		...rejected,
		structured_diagnostics: rejected.structured_diagnostics?.map((entry) => ({
			...entry,
			source_span_is_exact: false,
		})),
	});
	await click("Apply to board");
	expect(container.querySelector('[role="alert"]')?.textContent).toContain(
		"Possible location:",
	);
	expect(markers()).toEqual([]);
	await click("Line 2, column 10");
	expect(harness.editor.focus).toHaveBeenCalledOnce();
});

test("a reconcile result for a replaced Monaco model cannot install markers", async () => {
	let resolve!: (response: ICheckFlowScriptReconcileResponse) => void;
	harness.backend.boardState.checkFlowScriptReconcile.mockImplementationOnce(
		() =>
			new Promise((done) => {
				resolve = done;
			}),
	);
	await act(async () => vi.advanceTimersByTimeAsync(300));
	harness.editor.getModel.mockReturnValue({ ...harness.model });
	await act(async () =>
		resolve({
			parse_valid: true,
			reconcile_valid: false,
			idempotent: false,
			command_count: 0,
			corrections: [],
			diagnostics: rejected.diagnostics,
			structured_diagnostics: rejected.structured_diagnostics,
		}),
	);
	expect(markers()).toEqual([]);
});
