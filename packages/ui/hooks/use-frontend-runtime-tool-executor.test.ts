import { describe, expect, test } from "bun:test";

import { normalizeDatabaseTableIdentifier } from "../lib/database-table-name";
import { resolveUiInspectWidgetEntries } from "./use-frontend-runtime-tool-executor";

describe("normalizeDatabaseTableIdentifier", () => {
	test("keeps valid physical identifiers unchanged", () => {
		expect(normalizeDatabaseTableIdentifier("Existing.Table-v2")).toBe(
			"Existing.Table-v2",
		);
	});

	test("maps human-facing labels to stable semantic identifiers", () => {
		expect(normalizeDatabaseTableIdentifier("Library Files")).toBe(
			"library_files",
		);
		expect(normalizeDatabaseTableIdentifier("R&D / Reports")).toBe(
			"r_and_d_reports",
		);
	});
});

describe("resolveUiInspectWidgetEntries", () => {
	const list = [
		[
			"app-expenses",
			"widget-expense-row",
			{ name: "Expense Row", description: "Reusable expense item." },
		],
		[
			"app-expenses",
			"widget-legacy",
			{ title: "Legacy Row", description: "Legacy metadata title." },
		],
		["app-expenses", "widget-no-metadata", undefined],
	] as const;

	test("normalizes the [appId, widgetId, metadata] contract", () => {
		expect(resolveUiInspectWidgetEntries(list).entries).toEqual([
			{
				widgetId: "widget-expense-row",
				selector: "Expense Row",
				description: "Reusable expense item.",
			},
			{
				widgetId: "widget-legacy",
				selector: "Legacy Row",
				description: "Legacy metadata title.",
			},
			{
				widgetId: "widget-no-metadata",
				selector: "widget-no-metadata",
				description: undefined,
			},
		]);
	});

	test("resolves selectors by widget id or real metadata name", () => {
		expect(
			resolveUiInspectWidgetEntries(list, "widget-expense-row").match,
		).toMatchObject({
			widgetId: "widget-expense-row",
			selector: "Expense Row",
		});
		expect(
			resolveUiInspectWidgetEntries(list, "Expense Row").match,
		).toMatchObject({
			widgetId: "widget-expense-row",
			selector: "Expense Row",
		});
		expect(
			resolveUiInspectWidgetEntries(list, "Legacy Row").match,
		).toMatchObject({
			widgetId: "widget-legacy",
			selector: "Legacy Row",
		});
	});

	test("never treats the tuple app id as a widget selector", () => {
		expect(
			resolveUiInspectWidgetEntries(list, "app-expenses").match,
		).toBeUndefined();
	});
});
