import assert from "node:assert/strict";
import { chromium } from "playwright";
import puppeteer from "puppeteer";

const browser = await chromium.launch({
	headless: true,
	executablePath: puppeteer.executablePath(),
	args: ["--enable-webgl", "--enable-unsafe-swiftshader"],
});
const page = await browser.newPage({
	viewport: { width: 1600, height: 1000 },
});
page.setDefaultTimeout(15_000);
const errors = [];
page.on("pageerror", (error) => errors.push(error.message));
await page.route("**/*", (route) => {
	const url = new URL(route.request().url());
	return ["127.0.0.1", "localhost"].includes(url.hostname) ||
		["data:", "blob:"].includes(url.protocol)
		? route.continue()
		: route.abort();
});

const waitForBoard = () =>
	page.waitForFunction(() => window.__boardHost?.snapshot().nodes.length > 0);
const load = (query) =>
	page.goto(`http://127.0.0.1:4339/host?${query}`, {
		timeout: 180_000,
		waitUntil: "domcontentloaded",
	});

try {
	for (const renderer of ["egui", "react-flow"]) {
		await load(`boardRenderer=${renderer}&board-read-delay=2500`);
		const progress = page.getByText("Loading flow…", { exact: true });
		await progress.waitFor({ state: "visible" });
		assert.equal(
			await page.locator("[data-board-renderer]").getAttribute("aria-busy"),
			"true",
		);
		assert.equal(
			await page
				.getByRole("status")
				.filter({ hasText: "Loading flow…" })
				.count(),
			1,
		);
		await page.screenshot({ path: `/tmp/flow-board-loading-${renderer}.png` });
		await waitForBoard();
		await progress.waitFor({ state: "hidden" });
		console.log(
			`PASS ${renderer}: loading text is visible until the board loads`,
		);
	}

	const quality = page.getByRole("button", {
		name: /^Quality: (1 error or warning|\d+ errors and warnings)$/,
	});
	await quality.waitFor();
	const label = await quality.getAttribute("aria-label");
	assert.match(
		label,
		/^Quality: (1 error or warning|[1-9]\d* errors and warnings)$/,
	);
	const summary = label.replace("Quality: ", "");
	await quality.hover();
	await page.getByRole("tooltip").filter({ hasText: summary }).waitFor();
	await page.mouse.move(800, 50);
	await page.keyboard.press("Escape");
	await page
		.getByRole("tooltip")
		.filter({ hasText: summary })
		.waitFor({ state: "hidden" });
	await quality.focus();
	await page.getByRole("tooltip").filter({ hasText: summary }).waitFor();
	await page.screenshot({ path: "/tmp/flow-board-quality-tooltip.png" });
	console.log(
		`PASS Quality tooltip explains its badge on hover and focus: ${label}`,
	);

	for (const failure of ["board-read-failures", "board-read-invalid"]) {
		await load(`boardRenderer=egui&board-read-delay=1000&${failure}=1`);
		await page.getByText("Loading flow…", { exact: true }).waitFor();
		const failed = page
			.getByRole("alert")
			.filter({ hasText: "The flow could not be loaded." });
		await failed.waitFor();
		assert.equal(await page.locator("[data-board-renderer]").count(), 0);
		assert.equal(
			await page.getByRole("button", { name: /^Quality/ }).count(),
			0,
		);
		await page.screenshot({
			path: `/tmp/flow-board-load-failed-${failure}.png`,
		});
		await failed.getByRole("button", { name: "Retry", exact: true }).click();
		await failed
			.getByRole("button", { name: "Retry", exact: true, disabled: true })
			.waitFor();
		assert.equal(await page.getByText("Loading flow…").count(), 0);
		await waitForBoard();
		await failed.waitFor({ state: "hidden" });
		assert.equal(
			await page.evaluate(() => window.__boardHost.snapshot().reads.getBoard),
			2,
		);
		console.log(`PASS ${failure}: initial board read recovers with Retry`);
	}
	assert.deepEqual(errors, []);
} finally {
	await browser.close();
}
