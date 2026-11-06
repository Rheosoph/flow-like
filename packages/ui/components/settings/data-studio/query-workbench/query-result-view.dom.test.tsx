// @vitest-environment happy-dom
import React, { act, type ComponentProps } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ExecuteSqlResult } from "../../../../state/backend-state/query-state";
import { QueryResultView } from "./query-result-view";

const notices = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn() }));
vi.mock("sonner", () => ({ toast: notices }));
vi.mock("@flow-like/locales", () => {
	const t = (
		_key: string,
		fallback: string,
		values: Record<string, unknown> = {},
	) =>
		fallback.replace(/\{\{(\w+)\}\}/g, (_, name: string) =>
			String(values[name] ?? ""),
		);
	return { useTranslation: () => ({ t }), i18n: { t } };
});
vi.mock("./query-result-chart", () => ({
	inferChartConfig: () => ({}),
	QueryResultChart: () => null,
}));
vi.mock("./query-result-graph", () => ({
	inferGraphConfig: () => ({}),
	QueryResultGraph: () => null,
}));
vi.mock("./result-value", () => ({
	ResultDetailValue: ({ value }: { value: unknown }) => (
		<div data-testid="rich-value">
			{typeof value === "string" ? value : JSON.stringify(value)}
		</div>
	),
}));
vi.mock("./query-result-table", () => ({
	QueryResultTable: ({ rows }: { rows: Record<string, unknown>[] }) => (
		<div data-testid="result-table">
			{rows.map((row) => row.name).join(", ")}
		</div>
	),
}));

const result: ExecuteSqlResult = {
	columns: [{ name: "name", type_name: "Utf8", position: 0 }],
	rows: [{ name: "Alpha" }, { name: "Beta" }],
	row_count: 2,
	truncated: false,
};
const defaults: ComponentProps<typeof QueryResultView> = {
	result,
	loading: false,
	error: null,
	vizConfig: { view: "table" },
	onVizConfigChange: vi.fn(),
};
let root: Root;
let container: HTMLDivElement;
function required<T>(value: T | null | undefined): T {
	if (value == null) throw new Error("Expected element");
	return value;
}
function button(label: string) {
	return required(
		[...document.querySelectorAll<HTMLButtonElement>("button")].find(
			(node) =>
				node.getAttribute("aria-label") === label ||
				node.textContent?.trim() === label,
		),
	);
}
async function render(
	props: Partial<ComponentProps<typeof QueryResultView>> = {},
) {
	await act(async () =>
		root.render(<QueryResultView {...defaults} {...props} />),
	);
}
async function filter(value: string) {
	const input = required(
		container.querySelector<HTMLInputElement>(
			'input[aria-label="Filter result rows"]',
		),
	);
	await act(async () => {
		Object.getOwnPropertyDescriptor(
			HTMLInputElement.prototype,
			"value",
		)?.set?.call(input, value);
		input.dispatchEvent(new Event("input", { bubbles: true }));
	});
}
async function tab(label: string) {
	await act(async () =>
		button(label).dispatchEvent(
			new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
		),
	);
}
async function openExport() {
	await act(async () =>
		button("Export all").dispatchEvent(
			new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
		),
	);
}

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true, React });
	notices.success.mockClear();
	notices.error.mockClear();
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});
afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	vi.restoreAllMocks();
});

describe("query results", () => {
	it("clears a filter for a new result without carrying a misleading empty state forward", async () => {
		await render();
		await filter("Alpha");
		expect(
			container.querySelector('[data-testid="result-table"]')?.textContent,
		).toBe("Alpha");
		expect(container.textContent).toContain("1 of 2 rows");
		await render({
			result: {
				...result,
				rows: [{ name: "Gamma" }, { name: "Delta" }],
				row_count: 2,
			},
		});
		expect(container.querySelector("input")?.value).toBe("");
		expect(
			container.querySelector('[data-testid="result-table"]')?.textContent,
		).toBe("Gamma, Delta");
		expect(container.textContent).toContain("2 rows");
	});

	it("offers a recovery action when the row filter has no matches", async () => {
		await render();
		await filter("Missing");
		expect(container.textContent).toContain("No rows match");
		await act(async () => button("Clear row filter").click());
		expect(
			container.querySelector('[data-testid="result-table"]')?.textContent,
		).toBe("Alpha, Beta");
	});

	it("exports all returned rows after filtering and reports clipboard failure", async () => {
		const copy = vi
			.spyOn(navigator.clipboard, "writeText")
			.mockRejectedValue(new Error("Clipboard unavailable"));
		await render();
		await filter("Alpha");
		await openExport();
		expect(document.body.textContent).toContain("All 2 returned rows");
		const item = required(
			[...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find(
				(node) => node.textContent?.includes("Copy as Markdown"),
			),
		);
		await act(async () => item.click());
		expect(copy).toHaveBeenCalledWith(expect.stringContaining("Beta"));
		expect(notices.success).not.toHaveBeenCalled();
		expect(notices.error).toHaveBeenCalledWith(
			"Could not copy to the clipboard.",
		);
	});

	it("retains the restore control through loading and failure, and disables an invalid initial run", async () => {
		const toggle = vi.fn();
		await render({
			loading: true,
			resultsExpanded: true,
			onToggleResultsExpanded: toggle,
		});
		await act(async () => button("Restore editor").click());
		expect(toggle).toHaveBeenCalledOnce();
		await render({
			error: "Unknown table",
			resultsExpanded: true,
			onToggleResultsExpanded: toggle,
		});
		expect(button("Restore editor").disabled).toBe(false);
		expect(container.querySelector('[role="alert"]')?.textContent).toContain(
			"Unknown table",
		);
		await render({ result: null, onRun: vi.fn(), runDisabled: true });
		expect(button("Run query").disabled).toBe(true);
	});

	it("shows stale and limited result boundaries alongside execution duration", async () => {
		await render({
			result: { ...result, truncated: true },
			stale: true,
			lastRun: { ok: true, durationMs: 92, rowCount: 2 },
		});
		expect(container.textContent).toContain("92 ms");
		expect(container.textContent).toContain("Query changed. Run again");
		expect(container.textContent).toContain(
			"Exports include only returned rows",
		);
	});

	it("opens a single returned record with full content and lets the user return to the table", async () => {
		const onVizConfigChange = vi.fn();
		const content = `### Full record\n\n${"Long readable content. ".repeat(30)}`;
		await render({
			result: {
				columns: [
					...result.columns,
					{ name: "content", type_name: "Utf8", position: 1 },
					{ name: "id", type_name: "Utf8", position: 2 },
				],
				rows: [{ name: "Alpha", content, id: "BS_0034_2010" }],
				row_count: 1,
				truncated: false,
			},
			onVizConfigChange,
		});
		expect(button("Record").getAttribute("aria-selected")).toBe("true");
		expect(
			container.querySelector('[data-testid="rich-value"]')?.textContent,
		).toBe(content);
		expect(
			container.querySelectorAll('[data-testid="rich-value"]'),
		).toHaveLength(1);
		expect(
			[...container.querySelectorAll("dd")].some(
				(field) => field.textContent === "BS_0034_2010",
			),
		).toBe(true);
		expect(
			container.querySelector('input[aria-label="Filter result rows"]'),
		).toBeNull();
		expect(button("Previous row").disabled).toBe(true);
		expect(button("Next row").disabled).toBe(true);
		await tab("Table");
		expect(
			container.querySelector('[data-testid="result-table"]')?.textContent,
		).toBe("Alpha");
		expect(onVizConfigChange).toHaveBeenLastCalledWith({ view: "table" });
		await tab("Record");
		expect(button("Record").getAttribute("aria-selected")).toBe("true");
		expect(onVizConfigChange).toHaveBeenLastCalledWith({ view: "table" });
	});

	it("navigates records, copies the selected row, and resets for a new result", async () => {
		const copy = vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue();
		await render();
		await filter("Alpha");
		await tab("Record");
		expect(container.textContent).toContain("Row 1 of 2");
		expect(container.querySelector("input")).toBeNull();
		await act(async () => button("Next row").click());
		expect(container.textContent).toContain("Row 2 of 2");
		expect(button("Next row").disabled).toBe(true);
		expect(container.querySelector("dd")?.textContent).toBe("Beta");
		await act(async () => button("Copy row").click());
		expect(copy).toHaveBeenCalledWith(
			JSON.stringify({ name: "Beta" }, null, 2),
		);
		expect(notices.success).toHaveBeenCalledWith("Copied row as JSON");
		await render({
			result: { ...result, rows: [{ name: "Gamma" }], row_count: 1 },
		});
		expect(container.textContent).toContain("Row 1 of 1");
		expect(container.querySelector("dd")?.textContent).toBe("Gamma");
	});

	it("offers table browsing before a query has run", async () => {
		const choose = vi.fn();
		await render({ result: null, onChooseTable: choose });
		await act(async () => button("Browse tables").click());
		expect(choose).toHaveBeenCalledOnce();
	});
});
