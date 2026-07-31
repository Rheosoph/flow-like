import { describe, expect, it, vi } from "vitest";
import {
	boundFlowScriptSource,
	readFlowScriptSource,
} from "./read-flowscript-source";

describe("readFlowScriptSource", () => {
	it("preserves an authorized explicit cross-app target", async () => {
		const getFlowScript = vi
			.fn()
			.mockResolvedValue(
				'eventsSimple() {\n    log({ text: "prior art" })\n}\n',
			);

		const result = await readFlowScriptSource(
			{
				appId: "referenced-app",
				boardId: "referenced-board",
				scopedAppId: "current-app",
				locator: "prior art",
			},
			{
				getProfileAppIds: async () => new Set(["referenced-app"]),
				getFlowScript,
			},
		);

		expect(getFlowScript).toHaveBeenCalledWith(
			"referenced-app",
			"referenced-board",
		);
		expect(result).toMatchObject({
			status: "ok",
			app_id: "referenced-app",
			board_id: "referenced-board",
			locator_matched: true,
			truncated: false,
		});
		expect(result.source).toContain("prior art");
	});

	it("allows the scoped app before profile propagation but rejects arbitrary app ids", async () => {
		const getFlowScript = vi.fn().mockResolvedValue("eventsSimple() {}\n");
		const dependencies = {
			getProfileAppIds: async () => new Set<string>(),
			getFlowScript,
		};

		await expect(
			readFlowScriptSource(
				{
					appId: "current-app",
					boardId: "current-board",
					scopedAppId: "current-app",
				},
				dependencies,
			),
		).resolves.toMatchObject({ status: "ok", app_id: "current-app" });
		await expect(
			readFlowScriptSource(
				{
					appId: "unrelated-app",
					boardId: "some-board",
					scopedAppId: "current-app",
				},
				dependencies,
			),
		).resolves.toMatchObject({
			status: "forbidden",
			code: "FLOWSCRIPT_SOURCE_APP_NOT_ACCESSIBLE",
		});
		expect(getFlowScript).toHaveBeenCalledTimes(1);
	});

	it("centers a bounded large-source response on a stable locator id", () => {
		const locator = "node-id-1234567890";
		const source = `${"a".repeat(70_000)}\n// @node ${locator}\n${"b".repeat(
			70_000,
		)}`;
		const bounded = boundFlowScriptSource(
			source,
			`workflow rooted at ${locator}`,
		);

		expect(bounded.truncated).toBe(true);
		expect(bounded.locatorMatched).toBe(true);
		expect(bounded.source.length).toBeLessThanOrEqual(60_000);
		expect(bounded.source).toContain(locator);
		expect(bounded.startOffset).toBeGreaterThan(0);
		expect(bounded.endOffset).toBeLessThan(source.length);
	});
});
