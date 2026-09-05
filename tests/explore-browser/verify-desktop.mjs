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
const loadErrors = [];
const measurements = [];
page.on("pageerror", (error) => errors.push(error.message));
page.on("console", (message) => { if (message.type() === "error") loadErrors.push(message.text()); });
page.on("response", (response) => { if (response.status() >= 400) loadErrors.push(`${response.status()} ${response.url()}`); });
await page.route("**/*", (route) => {
	const url = new URL(route.request().url());
	return url.hostname === "127.0.0.1" ||
		["data:", "blob:"].includes(url.protocol)
		? route.continue()
		: route.abort();
});
const base = "http://127.0.0.1:4326";
const rect = (selector) => page.locator(selector).boundingBox();
const waitForCall = (method, command) =>
	page.waitForFunction(
		({ method, command }) =>
			window.exploreQa?.calls.some(
				(call) =>
					call.method === method &&
					call.status === "success" &&
					(!command || call.args[0] === command),
			),
		{ method, command },
	);
const oneHeading = async (label) => {
	assert.equal(
		await page
			.getByRole("heading", { level: 1, name: "Explore", exact: true })
			.count(),
		1,
		`${label}: one Explore heading`,
	);
	assert.equal(
		await page.locator("[data-explore-header]").count(),
		1,
		`${label}: one shared header`,
	);
	assert.equal(
		await page.locator("[data-explore-scroll]").count(),
		1,
		`${label}: one scrolling catalog`,
	);
};
const sameGeometry = (expected, actual, label) => {
	for (const key of ["x", "y", "width", "height"])
		assert.ok(
			Math.abs(expected[key] - actual[key]) <= 1,
			`${label} ${key}: ${expected[key]} vs ${actual[key]}`,
		);
};
const snapshot = async () => ({
	header: await rect("[data-explore-header]"),
	toolbar: await rect("[data-explore-toolbar]"),
	search: await rect('input[type="search"]'),
	content: await rect("[data-explore-content]"),
});
const compare = async (apps, label) => {
	await oneHeading(label);
	const current = await snapshot();
	for (const part of ["header", "toolbar", "search"])
		sameGeometry(apps[part], current[part], `${label} ${part}`);
	for (const key of ["x", "y", "width"])
		assert.ok(
			Math.abs(apps.content[key] - current.content[key]) <= 1,
			`${label} content ${key}: ${apps.content[key]} vs ${current.content[key]}`,
		);
	assert.equal(
		await page.getByRole("tablist", { name: "Package views" }).count(),
		1,
		`${label}: one desktop tab bar`,
	);
	const dimensions = await page.evaluate(() => ({
		screen: innerWidth,
		page: document.documentElement.scrollWidth,
		viewport: document.querySelector("[data-explore-scroll]").clientWidth,
		content: document.querySelector("[data-explore-scroll]").scrollWidth,
	}));
	assert.ok(
		dimensions.page <= dimensions.screen + 1 &&
			dimensions.content <= dimensions.viewport + 1,
		`${label}: horizontal overflow ${JSON.stringify(dimensions)}`,
	);
	return current;
};

try {
	for (const width of [320, 390, 1440]) {
		await page.setViewportSize({ width, height: width < 768 ? 844 : 1000 });
		await page.goto(`${base}/store/explore/apps?desktop&developer`, {
			waitUntil: "networkidle",
		});
		await waitForCall("searchApps");
		await oneHeading(`${width}px Apps`);
		const apps = await snapshot();
		await page.getByRole("link", { name: "Packages", exact: true }).click();
		await page.waitForURL(`${base}/store/packages`);
		await waitForCall("registry");
		assert.equal(
			await page
				.getByRole("tab", { name: "Explore", exact: true })
				.getAttribute("aria-selected"),
			"true",
		);
		const explore = await compare(apps, `${width}px desktop Explore`);
		await page.screenshot({
			path: `/private/tmp/explore-desktop-packages-${width}.png`,
		});
		await page.getByRole("tab", { name: "Installed", exact: true }).click();
		await page.waitForURL(`${base}/store/packages?tab=installed`);
		await waitForCall("native", "registry_get_installed_packages");
		assert.equal(
			await page
				.getByRole("tab", { name: "Installed", exact: true })
				.getAttribute("aria-selected"),
			"true",
		);
		await page
			.getByRole("heading", { name: "Document Toolkit", exact: true })
			.waitFor();
		const installed = await compare(apps, `${width}px desktop Installed`);
		await page.screenshot({
			path: `/private/tmp/explore-desktop-installed-${width}.png`,
		});
		await page.getByRole("tab", { name: "Explore", exact: true }).click();
		await page.waitForURL(`${base}/store/packages`);
		assert.equal(
			await page
				.getByRole("tab", { name: "Explore", exact: true })
				.getAttribute("aria-selected"),
			"true",
		);
		await oneHeading(`${width}px returned Explore`);
		measurements.push({
			width,
			apps: {
				x: apps.header.x,
				width: apps.header.width,
				searchY: apps.search.y,
			},
			explore: {
				x: explore.header.x,
				width: explore.header.width,
				searchY: explore.search.y,
			},
			installed: {
				x: installed.header.x,
				width: installed.header.width,
				searchY: installed.search.y,
			},
		});
	}
	assert.deepEqual(errors, []);
	console.log(
		JSON.stringify(
			{ result: "Desktop route checks passed", measurements },
			null,
			2,
		),
	);
} catch (error) {
	console.error({ url: page.url(), errors, loadErrors });
	await page.screenshot({ path: "/private/tmp/explore-desktop-failure.png" });
	throw error;
} finally {
	await browser.close();
}
