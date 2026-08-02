import { describe, expect, test } from "bun:test";
import { join } from "node:path";
import type { ConfigEnv, UserConfig } from "vite";
import {
	contractTagForHtml,
	discoverWidgetEntries,
	flowLikeWidgets,
	widgetIdFromHtmlPath,
} from "../src/vite";
import { makeProjectFixture } from "./helpers";

const ENV: ConfigEnv = { command: "build", mode: "production" };

describe("discoverWidgetEntries", () => {
	test("finds src/widgets/*/index.html entrypoints", () => {
		const fixture = makeProjectFixture();
		const entries = discoverWidgetEntries(fixture.groupDir);
		expect(entries).toEqual([
			{
				id: "hello-widget",
				htmlPath: join(
					fixture.groupDir,
					"src",
					"widgets",
					"hello-widget",
					"index.html",
				),
			},
		]);
	});

	test("returns nothing outside a widget group", () => {
		expect(discoverWidgetEntries("/nonexistent")).toEqual([]);
	});
});

describe("widgetIdFromHtmlPath", () => {
	test("parses widget entry paths", () => {
		expect(
			widgetIdFromHtmlPath("/g", "/g/src/widgets/kpi-card/index.html"),
		).toBe("kpi-card");
		expect(widgetIdFromHtmlPath("/g", "/g/index.html")).toBeNull();
		expect(
			widgetIdFromHtmlPath("/g", "/g/src/widgets/kpi-card/other.html"),
		).toBeNull();
	});
});

describe("flowLikeWidgets", () => {
	test("config hook wires widget inputs and shared/ output names", () => {
		const fixture = makeProjectFixture();
		const plugin = flowLikeWidgets();
		const configHook = plugin.config as (
			config: UserConfig,
			env: ConfigEnv,
		) => UserConfig;
		const patch = configHook({ root: fixture.groupDir }, ENV);

		const rollup = patch.build?.rollupOptions;
		expect(rollup?.input).toEqual({
			"hello-widget": join(
				fixture.groupDir,
				"src",
				"widgets",
				"hello-widget",
				"index.html",
			),
		});
		const output = rollup?.output as Record<string, string>;
		expect(output.entryFileNames).toStartWith("shared/");
		expect(output.chunkFileNames).toStartWith("shared/");
		expect(output.assetFileNames).toStartWith("shared/");
	});

	test("contractTagForHtml injects the extracted contract for widget pages", () => {
		const fixture = makeProjectFixture();
		const htmlPath = join(
			fixture.groupDir,
			"src",
			"widgets",
			"hello-widget",
			"index.html",
		);
		const tag = contractTagForHtml(fixture.groupDir, htmlPath);
		expect(tag).not.toBeNull();
		expect(tag?.tag).toBe("script");
		expect(tag?.injectTo).toBe("head-prepend");
		expect(tag?.children).toContain("globalThis.__FLW_CONTRACT__");
		expect(tag?.children).toContain('"id":"hello-widget"');

		expect(
			contractTagForHtml(
				fixture.groupDir,
				join(fixture.groupDir, "index.html"),
			),
		).toBeNull();
	}, 60000);
});
