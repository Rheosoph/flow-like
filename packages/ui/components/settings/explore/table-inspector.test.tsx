import { afterAll, beforeEach, describe, expect, mock, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import type { LanceDBExplorerProps } from "../../ui/lance-viewer";

const calls: { name: string; args: unknown[] }[] = [];
let explorer: LanceDBExplorerProps | undefined;
let failures: Record<string, Error> = {};
const dbState = Object.fromEntries(
	[
		"getSchema",
		"countItems",
		"listItems",
		"databaseHistory",
		"optimize",
		"updateItem",
		"dropColumns",
		"addColumn",
		"alterColumn",
		"getIndices",
		"dropIndex",
		"buildIndex",
	].map((name) => [
		name,
		Object.defineProperty(
			async (...args: unknown[]) => {
				calls.push({ name, args });
				return [];
			},
			"name",
			{ value: name },
		),
	]),
);
mock.module("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_: string, fallback: string) => fallback }),
}));
mock.module("../../../hooks/use-app-permissions", () => ({
	useAppPermissions: () => ({ can: () => true, isLoading: false }),
}));
mock.module("../../../hooks/use-invoke", () => ({
	useInvoke: (fn: { name: string }, _: unknown, args: unknown[]) => {
		calls.push({ name: fn.name, args });
		return {
			error: failures[fn.name],
			data:
				fn.name === "databaseHistory"
					? {
							reference: {
								branch: "training",
								version: 7,
								read_only: true,
								pinned: true,
								table: "samples",
							},
						}
					: fn.name === "getSchema"
						? { fields: [] }
						: fn.name === "countItems"
							? 0
							: [],
			refetch: () => Promise.resolve(),
			isLoading: false,
		};
	},
	useInvalidateInvoke: () => async () => {},
}));
mock.module("../../../state/backend-state", () => ({
	useBackend: () => ({ dbState }),
}));
mock.module("../../../lib", () => ({
	cn: (...args: unknown[]) => args.filter(Boolean).join(" "),
}));
mock.module("../../ui/lance-viewer", () => ({
	default: (props: LanceDBExplorerProps) => {
		explorer = props;
		return <div>rows</div>;
	},
}));
mock.module("../data-studio/database-history-controls", () => ({
	DatabaseHistoryControls: () => <div>history</div>,
}));
mock.module("../permission/permission-gate", () => ({
	SectionLockedPanel: () => null,
}));
mock.module("../permission/permission-notice", () => ({
	PermissionNotice: () => null,
}));
afterAll(() => mock.restore());
beforeEach(() => {
	calls.length = 0;
	explorer = undefined;
	failures = {};
});
const { TableInspector } = await import("./table-inspector");

describe("reference-aware table inspector", () => {
	test("failed branch or tag refresh hides cached rows and mutation controls", () => {
		failures.databaseHistory = new Error("Reference no longer exists");
		for (const selector of [{ branch: "deleted" }, { tag: "removed" }]) {
			calls.length = 0;
			const markup = renderToStaticMarkup(
				<TableInspector appId="app" table="samples" selector={selector} />,
			);
			expect(markup).toContain("Reference no longer exists");
			expect(explorer).toBeUndefined();
			expect(calls.some((call) => call.name === "listItems")).toBe(false);
		}
	});
	test("failed table reads never leave cached data editable", () => {
		for (const name of ["getSchema", "countItems", "listItems"]) {
			failures = { [name]: new Error("Selected branch is unavailable") };
			const markup = renderToStaticMarkup(
				<TableInspector
					appId="app"
					table="samples"
					selector={{ branch: "experiment" }}
				/>,
			);
			expect(markup).toContain("Selected branch is unavailable");
			expect(explorer).toBeUndefined();
		}
	});
	test("resolves a tag once before reading rows, schema and count", () => {
		renderToStaticMarkup(
			<TableInspector
				appId="app"
				table="samples"
				selector={{ tag: "accepted" }}
			/>,
		);
		for (const name of ["getSchema", "countItems", "listItems"]) {
			expect(calls.find((call) => call.name === name)?.args.at(-1)).toEqual({
				branch: "training",
				version: 7,
				read_only: undefined,
			});
		}
	});
	test("uses the selected scope and branch for rows, count, schema and edits", async () => {
		const selector = { branch: "experiment" };
		renderToStaticMarkup(
			<TableInspector
				appId="app"
				table="samples"
				userScoped
				selector={selector}
			/>,
		);
		for (const name of ["getSchema", "countItems", "listItems"]) {
			const read = calls.find((call) => call.name === name);
			expect(read?.args.at(-1)).toEqual(selector);
			expect(read?.args.at(-2)).toBe(true);
		}
		await explorer?.onUpdateItem?.("id = 'row-1'", { label: "accepted" });
		expect(calls.find((call) => call.name === "updateItem")?.args).toEqual([
			"app",
			"samples",
			"id = 'row-1'",
			{ label: "accepted" },
			true,
			selector,
		]);
	});
	test("historical views hide row, schema and index mutations while preserving index reads", () => {
		for (const selector of [
			{ branch: "experiment", version: 7 },
			{ tag: "training" },
			{ read_only: true },
		]) {
			renderToStaticMarkup(
				<TableInspector appId="app" table="samples" selector={selector} />,
			);
			for (const name of [
				"onUpdateItem",
				"onOptimize",
				"onDropColumns",
				"onAddColumn",
				"onAlterColumn",
				"onDropIndex",
				"onBuildIndex",
			] as const)
				expect(explorer?.[name]).toBeUndefined();
			expect(explorer?.onGetIndices).toBeFunction();
		}
	});
	test("changing the selected reference changes every table read identity", () => {
		renderToStaticMarkup(
			<TableInspector
				appId="app"
				table="samples"
				selector={{ branch: "main" }}
			/>,
		);
		renderToStaticMarkup(
			<TableInspector
				appId="app"
				table="samples"
				selector={{ branch: "experiment", version: 2 }}
			/>,
		);
		expect(
			calls
				.filter((call) => call.name === "listItems")
				.map((call) => call.args.at(-1)),
		).toEqual([{ branch: "main" }, { branch: "experiment", version: 2 }]);
	});
});
