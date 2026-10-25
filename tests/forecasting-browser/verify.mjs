import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { chromium } from "playwright-core";

const output =
	process.env.FORECAST_SCREENSHOT_DIR || "/tmp/flow-like-forecast-screenshots";
await mkdir(output, { recursive: true });
const browser = await chromium.launch({
	executablePath:
		process.env.CHROME_EXECUTABLE_PATH ||
		(process.platform === "darwin"
			? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
			: undefined),
	headless: true,
	args: ["--no-sandbox"],
});
const page = await browser.newPage({
	viewport: { width: 1440, height: 1100 },
	deviceScaleFactor: 1,
});
page.setDefaultTimeout(15_000);
const errors = [];
page.on("pageerror", (error) => errors.push(error.message));
page.on("console", (message) => {
	if (message.type() === "error") errors.push(message.text());
});
await page.route("**/*", (route) => {
	const url = new URL(route.request().url());
	return url.hostname === "127.0.0.1" ||
		["data:", "blob:"].includes(url.protocol)
		? route.continue()
		: route.abort();
});
const capture = async (name, fullPage = true) => {
	await page.mouse.move(0, 0);
	await page.screenshot({
		path: `${output}/${name}.png`,
		fullPage,
		animations: "disabled",
	});
	if (/^0[1-4]-/.test(name))
		await page
			.getByRole("region", { name: "Forecast setup", exact: true })
			.screenshot({
				path: `${output}/wizard-${name}.png`,
				animations: "disabled",
			});
};
const fits = async () => {
	const sizes = await page.evaluate(() => ({
		viewport: innerWidth,
		page: document.documentElement.scrollWidth,
	}));
	assert.ok(sizes.page <= sizes.viewport + 1, JSON.stringify(sizes));
};
const select = async (name, option) => {
	await page.getByRole("combobox", { name, exact: true }).click();
	await page.getByRole("option", { name: option, exact: true }).click();
};
const step = async (title, index) => {
	await page
		.getByRole("button", { name: `Step ${index}: ${title}`, exact: true })
		.click();
	await page
		.getByRole("heading", { name: `Step ${index} of 4: ${title}` })
		.waitFor();
};
const forecastCallCount = () =>
	page.evaluate(
		() =>
			window.forecastingQa.calls.filter((call) => call.method === "forecast")
				.length,
	);
const chart = () =>
	page.getByRole("region", { name: "Forecast visualization", exact: true });
const visibleRange = async () =>
	Promise.all(
		["Visible range start", "Visible range end"].map(async (name) =>
			Number(
				await page
					.getByRole("slider", { name, exact: true })
					.getAttribute("aria-valuenow"),
			),
		),
	);
const rangeSpan = ([start, end]) => end - start;
const waitForReady = () =>
	page.getByText(/^Forecast ready with 72 predictions$/).waitFor();
const chartColors = () =>
	chart().evaluate((element) => {
		const style = getComputedStyle(element);
		return { background: style.backgroundColor, foreground: style.color };
	});
const verifyForecastTransition = async (scope = chart()) => {
	const transitions = await scope
		.locator(".forecast-transition")
		.evaluateAll((groups) =>
			groups.map((group) => {
				const line = group.querySelector("line");
				const paths = group
					.closest("svg")
					.querySelectorAll(".recharts-line-curve");
				const history = paths[0].getPointAtLength(paths[0].getTotalLength());
				const forecast = paths[paths.length - 1].getPointAtLength(0);
				return {
					dash: line.getAttribute("stroke-dasharray"),
					startGap: Math.hypot(
						Number(line.getAttribute("x1")) - history.x,
						Number(line.getAttribute("y1")) - history.y,
					),
					endGap: Math.hypot(
						Number(line.getAttribute("x2")) - forecast.x,
						Number(line.getAttribute("y2")) - forecast.y,
					),
				};
			}),
		);
	assert.equal(
		transitions.length,
		2,
		"The main chart and overview include a forecast transition",
	);
	for (const transition of transitions) {
		assert.equal(transition.dash, "3 3");
		assert.ok(
			transition.startGap < 0.02 && transition.endGap < 0.02,
			`Transition joins the observed and forecast path endpoints: ${JSON.stringify(transition)}`,
		);
	}
};

try {
	await page.goto("http://127.0.0.1:4344/", { waitUntil: "networkidle" });
	await page.getByText("4 columns available", { exact: true }).waitFor();
	await fits();
	await capture("01-source");
	await page
		.getByRole("button", { name: "Next: Columns", exact: true })
		.click();
	await select("Series ID", "machine_id");
	const load = page.getByRole("button", { name: "load_pct", exact: true });
	if ((await load.getAttribute("aria-pressed")) === "true") await load.click();
	await fits();
	await capture("02-columns-per-machine");
	await page.getByRole("button", { name: "Next: Model", exact: true }).click();
	await page
		.getByRole("switch", { name: "Evaluate on held-out data", exact: true })
		.click();
	await page
		.getByRole("spinbutton", { name: "Holdout steps", exact: true })
		.fill("12");
	await fits();
	await capture("03-model-and-evaluation");
	await page.getByRole("button", { name: "Next: Review", exact: true }).click();
	await fits();
	await capture("04-review");
	await page.getByRole("button", { name: "Run forecast", exact: true }).click();
	await waitForReady();
	assert.equal(
		await page
			.getByRole("button", { name: "Outlook", exact: true })
			.getAttribute("aria-pressed"),
		"true",
	);
	assert.equal(
		await page
			.getByRole("slider", { name: "Holdout forecast as of", exact: true })
			.count(),
		0,
	);
	await select("Chart series", "Machine B");
	await verifyForecastTransition();
	await fits();
	await capture("05-grouped-forecast");
	await page
		.getByRole("region", { name: "Forecast visualization", exact: true })
		.screenshot({
			path: `${output}/05-grouped-chart-detail.png`,
			animations: "disabled",
		});
	const fullRange = await visibleRange();
	const fullTicks = await chart()
		.locator(".recharts-xAxis")
		.first()
		.textContent();
	await page.getByRole("button", { name: "Zoom in", exact: true }).click();
	const zoomRange = await visibleRange();
	assert.ok(
		rangeSpan(zoomRange) < rangeSpan(fullRange),
		"Zoom in narrows the visible range",
	);
	assert.notEqual(
		await chart().locator(".recharts-xAxis").first().textContent(),
		fullTicks,
		"Zoom updates plotted timestamps",
	);
	await page.getByRole("button", { name: "Zoom out", exact: true }).click();
	assert.ok(
		rangeSpan(await visibleRange()) > rangeSpan(zoomRange),
		"Zoom out expands the visible range",
	);
	await page.getByRole("button", { name: "Zoom in", exact: true }).click();
	await page.getByRole("button", { name: "Reset zoom", exact: true }).click();
	assert.deepEqual(await visibleRange(), fullRange);
	const plot = chart().locator(".recharts-cartesian-grid").first();
	await plot.scrollIntoViewIfNeeded();
	const plotBox = await plot.boundingBox();
	assert.ok(plotBox);
	await page.mouse.move(
		plotBox.x + plotBox.width * 0.2,
		plotBox.y + plotBox.height / 2,
	);
	await page.mouse.down();
	await page.mouse.move(
		plotBox.x + plotBox.width * 0.65,
		plotBox.y + plotBox.height / 2,
		{ steps: 12 },
	);
	await page.mouse.up();
	assert.ok(
		rangeSpan(await visibleRange()) < rangeSpan(fullRange),
		"Dragging the main plot zooms to the selected timestamps",
	);
	await page.mouse.dblclick(
		plotBox.x + plotBox.width / 2,
		plotBox.y + plotBox.height / 2,
	);
	assert.deepEqual(
		await visibleRange(),
		fullRange,
		"Double-click resets plot zoom",
	);
	const startSlider = page.getByRole("slider", {
		name: "Visible range start",
		exact: true,
	});
	await startSlider.focus();
	await startSlider.press("ArrowRight");
	assert.ok(
		(await visibleRange())[0] > fullRange[0],
		"The range is keyboard operable",
	);
	const rangeBeforeDrag = await visibleRange();
	const startBox = await startSlider.boundingBox();
	assert.ok(startBox);
	await page.mouse.move(
		startBox.x + startBox.width / 2,
		startBox.y + startBox.height / 2,
	);
	await page.mouse.down();
	await page.mouse.move(
		startBox.x + startBox.width / 2 + 140,
		startBox.y + startBox.height / 2,
		{ steps: 8 },
	);
	await page.mouse.up();
	assert.ok(
		(await visibleRange())[0] > rangeBeforeDrag[0],
		"The range responds to a pointer drag",
	);
	await page
		.getByRole("button", { name: "Focus forecast", exact: true })
		.click();
	assert.ok(
		rangeSpan(await visibleRange()) < rangeSpan(fullRange),
		"Forecast focus narrows the range",
	);
	await select("Chart series", "Machine C");
	assert.deepEqual(
		await visibleRange(),
		fullRange,
		"Changing group resets the visible range",
	);
	await select("Chart series", "Machine B");
	await page
		.getByRole("button", { name: "Focus forecast", exact: true })
		.click();
	const focusedRange = await visibleRange();
	await verifyForecastTransition();
	await page.getByRole("button", { name: "Edit setup", exact: true }).click();
	assert.deepEqual(
		await visibleRange(),
		focusedRange,
		"Opening setup preserves chart zoom",
	);
	await page
		.getByRole("button", { name: "Back to prediction", exact: true })
		.click();
	assert.deepEqual(
		await visibleRange(),
		focusedRange,
		"Closing setup preserves chart zoom",
	);
	const fullscreenButton = page.getByRole("button", {
		name: "Full screen",
		exact: true,
	});
	await fullscreenButton.click();
	await page.getByRole("dialog").waitFor();
	assert.deepEqual(
		await visibleRange(),
		focusedRange,
		"Full screen preserves the visible range",
	);
	await page.getByRole("dialog").evaluate(async (element) => {
		await Promise.all(
			element.getAnimations().map((animation) => animation.finished),
		);
	});
	const dialogBox = await page.getByRole("dialog").boundingBox();
	assert.ok(
		dialogBox &&
			dialogBox.width >= 1300 &&
			dialogBox.height >= 900 &&
			Math.abs(dialogBox.x) < 1 &&
			Math.abs(dialogBox.y) < 1,
		`Full screen uses the viewport: ${JSON.stringify(dialogBox)}`,
	);
	await capture("05a-fullscreen-forecast", false);
	await verifyForecastTransition(page.getByRole("dialog"));
	const transitionBox = await page
		.getByRole("dialog")
		.locator(".forecast-transition line")
		.first()
		.boundingBox();
	assert.ok(transitionBox);
	await page.screenshot({
		path: `${output}/05c-forecast-boundary-detail.png`,
		clip: {
			x: Math.max(0, transitionBox.x - 90),
			y: Math.max(0, transitionBox.y - 80),
			width: transitionBox.width + 180,
			height: transitionBox.height + 160,
		},
		animations: "disabled",
	});
	await page.keyboard.press("Escape");
	await page.getByRole("dialog").waitFor({ state: "hidden" });
	assert.deepEqual(
		await visibleRange(),
		focusedRange,
		"Closing full screen preserves the visible range",
	);
	assert.ok(
		await fullscreenButton.evaluate(
			(element) => element === document.activeElement,
		),
		"Escape restores focus to the full screen button",
	);
	await page.getByRole("button", { name: "Reset zoom", exact: true }).click();
	const darkColors = await chartColors();
	await page
		.getByRole("button", { name: "Toggle color theme", exact: true })
		.click();
	await page.waitForFunction(() =>
		document.documentElement.classList.contains("light"),
	);
	const lightColors = await chartColors();
	assert.notEqual(
		lightColors.background,
		darkColors.background,
		"The chart background follows the theme",
	);
	assert.notEqual(
		lightColors.foreground,
		darkColors.foreground,
		"The chart text follows the theme",
	);
	await capture("05b-light-forecast");
	await page.evaluate(() =>
		document.documentElement.style.setProperty(
			"--chart-1",
			"rgb(180, 20, 140)",
		),
	);
	assert.ok(
		await chart()
			.locator(".recharts-line-curve")
			.evaluateAll((paths) =>
				paths.some(
					(path) => getComputedStyle(path).stroke === "rgb(180, 20, 140)",
				),
			),
		"Forecast lines use the chart theme token",
	);
	await page.evaluate(() =>
		document.documentElement.style.removeProperty("--chart-1"),
	);
	await page
		.getByRole("button", { name: "Toggle color theme", exact: true })
		.click();
	assert.equal(
		await forecastCallCount(),
		1,
		"Chart exploration does not rerun inference",
	);
	await page
		.getByRole("button", { name: "Focus forecast", exact: true })
		.click();
	await page.getByRole("button", { name: "Holdout", exact: true }).click();
	assert.equal(
		(await visibleRange())[0],
		0,
		"Changing to holdout resets the visible range",
	);
	const slider = page.getByRole("slider", {
		name: "Holdout forecast as of",
		exact: true,
	});
	await slider.focus();
	await slider.press("ArrowLeft");
	await page.waitForFunction(
		() =>
			window.forecastingQa.calls.filter((call) => call.method === "forecast")
				.length === 2,
	);
	await waitForReady();
	assert.equal(
		await page
			.getByRole("button", { name: "Holdout", exact: true })
			.getAttribute("aria-pressed"),
		"true",
	);
	const countBeforeDrag = await forecastCallCount();
	const holdoutBeforeDrag = Number(await slider.getAttribute("aria-valuenow"));
	const holdoutBox = await slider.boundingBox();
	assert.ok(holdoutBox);
	await page.mouse.move(
		holdoutBox.x + holdoutBox.width / 2,
		holdoutBox.y + holdoutBox.height / 2,
	);
	await page.mouse.down();
	await page.mouse.move(
		holdoutBox.x + holdoutBox.width / 2 - 160,
		holdoutBox.y + holdoutBox.height / 2,
		{ steps: 8 },
	);
	assert.notEqual(
		Number(await slider.getAttribute("aria-valuenow")),
		holdoutBeforeDrag,
		"Dragging previews a different holdout cutoff",
	);
	assert.equal(
		await forecastCallCount(),
		countBeforeDrag,
		"Dragging does not run a forecast before release",
	);
	await page.mouse.up();
	await page.waitForFunction(
		(count) =>
			window.forecastingQa.calls.filter((call) => call.method === "forecast")
				.length ===
			count + 1,
		countBeforeDrag,
	);
	await waitForReady();
	assert.equal(
		await page
			.getByRole("button", { name: "Holdout", exact: true })
			.getAttribute("aria-pressed"),
		"true",
	);
	await fits();
	await capture("06-historical-evaluation");
	await page
		.getByRole("region", { name: "Forecast visualization", exact: true })
		.screenshot({
			path: `${output}/06-historical-chart-detail.png`,
			animations: "disabled",
		});
	const forecastCalls = await page.evaluate(() =>
		window.forecastingQa.calls.filter((call) => call.method === "forecast"),
	);
	assert.equal(forecastCalls[0].payload.mapping.series_id, "machine_id");
	assert.deepEqual(forecastCalls[0].payload.mapping.targets, ["temperature_c"]);
	assert.equal(forecastCalls[1].payload.backtest_offset, 1);
	await page.setViewportSize({ width: 390, height: 844 });
	await fits();
	await capture("07-mobile-evaluation");
	await page.getByRole("button", { name: "Edit setup", exact: true }).click();
	await step("Columns", 2);
	await fits();
	await capture("08-mobile-columns");
	await page.setViewportSize({ width: 1440, height: 1100 });
	await step("Model", 3);
	await select("Forecasting model", "IBM Granite TinyTimeMixer");
	await page.getByText("Point forecast only", { exact: true }).waitFor();
	await capture("09-point-only-model");
	await page
		.getByRole("button", { name: "New prediction", exact: true })
		.click();
	await page.getByText("4 columns available", { exact: true }).waitFor();
	await page.getByRole("button", { name: "SQL query", exact: true }).click();
	await page.waitForFunction(
		() => window.monaco?.editor.getModels().length > 0,
	);
	// Monaco creates its model before React attaches the change subscription.
	await page.waitForTimeout(300);
	await page.evaluate(() =>
		window.monaco.editor
			.getModels()[0]
			.setValue(
				'SELECT timestamp, AVG(temperature_c) AS temperature_c\nFROM "sensor_readings"\nGROUP BY timestamp\nORDER BY timestamp',
			),
	);
	await page.getByText("Source needs a preview", { exact: true }).waitFor();
	await page
		.getByRole("button", { name: "Preview columns", exact: true })
		.click();
	await page.getByText("2 columns available", { exact: true }).waitFor();
	await fits();
	await capture("10-overall-source-sql");
	await page
		.getByRole("region", { name: "Forecast setup", exact: true })
		.screenshot({
			path: `${output}/wizard-10-overall-source-sql.png`,
			animations: "disabled",
		});
	await step("Columns", 2);
	assert.equal(
		await page
			.getByRole("combobox", { name: "Series ID", exact: true })
			.innerText(),
		"Single series",
	);
	await capture("11-overall-columns");
	assert.deepEqual(errors, []);
	console.log(
		`Forecast browser checks passed: setup, prediction-first results, zoom, keyboard and pointer range selection, full screen and focus restoration, theme tokens, holdout release-only reruns, point-only model, overall SQL preview, and desktop/mobile layouts. Screenshots: ${output}`,
	);
} catch (error) {
	console.error("Browser errors:", errors);
	await capture("failure");
	throw error;
} finally {
	await browser.close();
}
