import assert from "node:assert/strict";
import { chromium } from "playwright-core";

const browser = await chromium.launch({
	executablePath:
		process.env.CHROME_EXECUTABLE_PATH ||
		"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
	headless: true,
	args: ["--no-sandbox"],
});
const page = await browser.newPage();
const errors = [];
page.on("pageerror", (error) => errors.push(error.message));
await page.route("**/*", (route) => {
	const url = new URL(route.request().url());
	return url.hostname === "127.0.0.1" ||
		["data:", "blob:"].includes(url.protocol)
		? route.continue()
		: route.abort();
});
const base = "http://127.0.0.1:4326";
const rect = (selector) => page.locator(selector).boundingBox();
const waitLoaded = () =>
	page.waitForFunction(() =>
		window.exploreQa?.calls.some(
			(call) =>
				["searchApps", "registry"].includes(call.method) &&
				call.status === "success",
		),
	);
const open = async (path) => {
	await page.goto(base + path, { waitUntil: "networkidle" });
	await waitLoaded();
};
const unchanged = (a, b, label) => {
	for (const key of ["x", "y", "width", "height"])
		assert.ok(
			Math.abs(a[key] - b[key]) <= 1,
			`${label}: ${key} moved from ${a[key]} to ${b[key]}`,
		);
};
const fits = async () => {
	const sizes = await page.evaluate(() => ({
		width: innerWidth,
		document: document.documentElement.scrollWidth,
		viewport: document.querySelector("[data-explore-scroll]").clientWidth,
		content: document.querySelector("[data-explore-scroll]").scrollWidth,
	}));
	assert.ok(
		sizes.document <= sizes.width + 1 && sizes.content <= sizes.viewport + 1,
		JSON.stringify(sizes),
	);
};
try {
	for (const width of [320, 390, 1440]) {
		await page.setViewportSize({ width, height: width < 768 ? 844 : 1000 });
		await open("/store/explore/apps?developer");
		await page.getByRole("link", { name: "Packages", exact: true }).waitFor();
		const appsHeader = await rect("[data-explore-header]");
		const appsSearch = await rect('input[type="search"]');
		const appsToolbar = await rect("[data-explore-toolbar]");
		const appsContent = await rect("[data-explore-content]");
		await page.getByRole("searchbox").fill("invoice");
		await page.waitForURL(/q=invoice/);
		await page.getByRole("heading", { name: "Search results" }).waitFor();
		unchanged(
			appsSearch,
			await rect('input[type="search"]'),
			`${width}px Apps search after typing`,
		);
		await page.getByRole("combobox", { name: "Sort results" }).click();
		await page
			.getByRole("option", { name: /Recently updated/i, exact: true })
			.click();
		unchanged(
			appsSearch,
			await rect('input[type="search"]'),
			`${width}px Apps search after sort`,
		);
		await open("/store/explore/apps?developer");
		await page.locator("[data-explore-scroll]").evaluate((el) => {
			el.scrollTop = 600;
		});
		const scrolledHeader = await rect("[data-explore-header]");
		assert.ok(
			scrolledHeader.y + scrolledHeader.height <= 0,
			"Entire Apps header scrolls out of view",
		);
		await page.screenshot({
			path: `/private/tmp/explore-apps-scrolled-${width}.png`,
		});
		await open("/store/packages?developer");
		unchanged(
			appsHeader,
			await rect("[data-explore-header]"),
			`${width}px shared header`,
		);
		unchanged(
			appsToolbar,
			await rect("[data-explore-toolbar]"),
			`${width}px shared toolbar`,
		);
		unchanged(
			appsSearch,
			await rect('input[type="search"]'),
			`${width}px shared search`,
		);
		assert.ok(
			Math.abs(appsContent.y - (await rect("[data-explore-content]")).y) <= 1,
			"Content starts at the same height",
		);
		await fits();
		await page.screenshot({
			path: `/private/tmp/explore-packages-${width}.png`,
		});
		await page.locator("[data-explore-scroll]").evaluate((el) => {
			el.scrollTop = 600;
		});
		const packageHeader = await rect("[data-explore-header]");
		assert.ok(
			packageHeader.y + packageHeader.height <= 0,
			"Entire Packages header scrolls out of view",
		);
	}
	await page.setViewportSize({ width: 1440, height: 1000 });
	await page.goto(`${base}/store/packages?deferred-profile`, {
		waitUntil: "networkidle",
	});
	assert.equal(
		await page.getByText("No packages found", { exact: true }).count(),
		0,
	);
	const loadingToolbar = await rect("[data-explore-toolbar]");
	const loadingContent = await rect("[data-explore-content]");
	await page.evaluate(() => window.exploreQa.restore());
	await waitLoaded();
	unchanged(
		loadingToolbar,
		await rect("[data-explore-toolbar]"),
		"Packages profile loading toolbar",
	);
	assert.equal(loadingContent.y, (await rect("[data-explore-content]")).y);
	await page.getByRole("searchbox").fill("python");
	await page.waitForFunction(() =>
		window.exploreQa.calls.some(
			(call) =>
				call.method === "registry" &&
				call.args[0].includes("query=python") &&
				call.status === "success",
		),
	);
	unchanged(
		loadingToolbar,
		await rect("[data-explore-toolbar]"),
		"Packages search toolbar",
	);
	assert.deepEqual(errors, []);
	console.log(
		"Shared Explore layout checks passed: header scrolls away, Apps/Packages geometry matches, controls remain stable during search/sort/loading, and 320–1440px layouts fit.",
	);
} catch (error) {
	console.error(error);
	await page.screenshot({ path: "/private/tmp/explore-layout-failure.png" });
	throw error;
} finally {
	await browser.close();
}
