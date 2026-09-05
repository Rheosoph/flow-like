import assert from "node:assert/strict";
import { chromium } from "playwright-core";

const browser = await chromium.launch({
	executablePath:
		process.env.CHROME_EXECUTABLE_PATH ||
		(process.platform === "darwin"
			? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
			: undefined),
	headless: true,
	args: ["--no-sandbox"],
});
const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
const errors = [];
page.on("pageerror", (error) => errors.push(error.message));
await page.route("**/*", (route) => {
	const url = new URL(route.request().url());
	return url.hostname === "127.0.0.1" ||
		["data:", "blob:"].includes(url.protocol)
		? route.continue()
		: route.abort();
});
const base = "http://127.0.0.1:4326/store/explore/apps";
const search = page.getByRole("searchbox", { name: "Search community apps…" });
const ready = () =>
	page.waitForFunction(
		() =>
			document.querySelector("#explore-results")?.getAttribute("aria-busy") ===
			"false",
	);
const open = async (query = "") => {
	await page.goto(base + query, { waitUntil: "networkidle" });
	await ready();
};
const fits = async () => {
	const sizes = await page.evaluate(() => ({
		viewport: innerWidth,
		page: document.documentElement.scrollWidth,
		content: document.querySelector("#explore-results").clientWidth,
		scroll: document.querySelector("#explore-results").scrollWidth,
	}));
	assert.ok(sizes.page <= sizes.viewport + 1, JSON.stringify(sizes));
	assert.ok(sizes.scroll <= sizes.content + 1, JSON.stringify(sizes));
};

try {
	await open();
	await page.getByRole("heading", { name: "Popular right now" }).waitFor();
	assert.equal(
		await page.getByRole("navigation", { name: "Explore sections" }).count(),
		0,
	);
	assert.equal(
		await page
			.getByRole("link", { name: "Knowledge Chat", exact: true })
			.first()
			.getAttribute("href"),
		"/store?id=explore-app-1",
	);
	await fits();
	await page.screenshot({ path: "/private/tmp/explore-desktop.png" });
	await page.getByRole("button", { name: "New arrivals", exact: true }).click();
	await page.waitForURL(/sort=newest/);
	await ready();
	await page
		.getByRole("heading", { name: "Newest first", exact: true })
		.waitFor();
	await page
		.getByRole("button", { name: "Clear filters", exact: true })
		.click();
	await ready();

	await search.fill("invoice");
	await page.waitForURL(/q=invoice/);
	await ready();
	await page
		.getByRole("heading", { name: "Search results", exact: true })
		.waitFor();
	assert.ok(
		(await page.locator("#explore-results").innerText()).includes(
			"Invoice Desk",
		),
	);
	await page.getByRole("button", { name: "Business", exact: true }).click();
	await page.waitForURL(/category=Business/);
	await ready();
	await page.getByRole("button", { name: "Clear search", exact: true }).click();
	assert.equal(
		await search.evaluate((element) => element === document.activeElement),
		true,
	);
	assert.equal(await search.inputValue(), "");
	await ready();

	await open("?sort=toString&category=invalid&source=qa");
	assert.equal(new URL(page.url()).searchParams.get("sort"), null);
	assert.equal(new URL(page.url()).searchParams.get("source"), "qa");
	await page.getByRole("heading", { name: "Popular right now" }).waitFor();
	await page.evaluate(() => {
		history.pushState({}, "", "?q=%20invoice%20&category=Business&sort=rated");
		dispatchEvent(new PopStateEvent("popstate"));
	});
	await page.waitForFunction(
		() =>
			document.querySelector('input[type="search"]')?.value.trim() ===
			"invoice",
	);
	await ready();
	await page.getByRole("heading", { name: "Search results" }).waitFor();
	await page.evaluate(() => history.back());
	await page.getByRole("heading", { name: "Popular right now" }).waitFor();

	await open();
	await page
		.getByRole("button", { name: "Load more", exact: true })
		.scrollIntoViewIfNeeded();
	await page.evaluate(() => {
		window.exploreQa.error = true;
	});
	await page.getByRole("button", { name: "Load more", exact: true }).click();
	await page
		.getByRole("alert")
		.filter({ hasText: "More apps could not be loaded" })
		.waitFor();
	assert.ok(
		await page
			.getByRole("link", { name: "Knowledge Chat", exact: true })
			.count(),
	);
	const retry = page.getByRole("button", { name: "Retry", exact: true });
	const retryBox = await retry.boundingBox();
	assert.ok(
		retryBox.y >= 0 && retryBox.y < 1000,
		"Pagination retry remains in the visible viewport",
	);
	await page.evaluate(() => window.exploreQa.restore());
	await retry.click();
	await ready();
	await page.getByRole("alert").waitFor({ state: "detached" });
	assert.equal(
		await page.getByRole("button", { name: "Load more", exact: true }).count(),
		0,
	);
	await page.getByRole("button", { name: "Business", exact: true }).click();
	await ready();
	assert.equal(
		await page
			.locator("[data-explore-scroll]")
			.evaluate((element) => element.scrollTop),
		0,
	);

	await page.setViewportSize({ width: 390, height: 844 });
	await open();
	await fits();
	await page.screenshot({ path: "/private/tmp/explore-mobile.png" });
	await page
		.getByRole("button", { name: "More categories", exact: true })
		.click();
	await page
		.getByRole("menuitemradio", { name: "Finance", exact: true })
		.click();
	await page.waitForURL(/category=Finance/);
	await ready();
	const chip = page.getByRole("button", { name: "Finance", exact: true });
	assert.equal(await chip.getAttribute("aria-pressed"), "true");
	const chipBox = await chip.boundingBox();
	assert.ok(chipBox.x >= 0 && chipBox.x + chipBox.width <= 390);
	await fits();
	await page.screenshot({ path: "/private/tmp/explore-mobile-filter.png" });
	await page.setViewportSize({ width: 1440, height: 1000 });
	await page.setViewportSize({ width: 320, height: 844 });
	await page.waitForFunction(() => {
		const selected = document.querySelector(
			'fieldset button[aria-pressed="true"]',
		);
		const rect = selected?.getBoundingClientRect();
		return rect && rect.x >= 0 && rect.right <= innerWidth;
	});
	await fits();
	await page.setViewportSize({ width: 390, height: 844 });
	await search.fill("no-such-app");
	await page.waitForURL(/q=no-such-app/);
	await ready();
	await page.getByText("No apps match your filters", { exact: true }).waitFor();
	await page
		.getByRole("button", { name: "Clear filters", exact: true })
		.last()
		.click();
	await ready();
	await page.getByRole("heading", { name: "Popular right now" }).waitFor();

	for (const width of [320, 768, 1024, 1440]) {
		await page.setViewportSize({ width, height: width < 768 ? 844 : 1000 });
		await open("?light");
		await fits();
		if (width === 1440)
			await page.screenshot({ path: "/private/tmp/explore-light.png" });
	}
	await open("?error");
	await page
		.getByRole("alert")
		.filter({ hasText: "Apps could not be loaded" })
		.waitFor();
	assert.equal(
		await page.getByText("No apps found", { exact: true }).count(),
		0,
	);
	await page.evaluate(() => window.exploreQa.restore());
	await page.getByRole("button", { name: "Retry", exact: true }).click();
	await ready();
	await page.getByRole("heading", { name: "Popular right now" }).waitFor();
	await open("?empty");
	await page.getByText("No apps found", { exact: true }).waitFor();
	await page.goto(`${base}?loading`, { waitUntil: "networkidle" });
	assert.equal(
		await page.locator("#explore-results").getAttribute("aria-busy"),
		"true",
	);
	assert.equal(
		await page.getByText("No apps found", { exact: true }).count(),
		0,
	);
	await page.evaluate(() => window.exploreQa.restore());
	await ready();
	await page.getByRole("heading", { name: "Popular right now" }).waitFor();
	await open("?runnable");
	await page
		.getByRole("link", { name: "Knowledge Chat", exact: true })
		.first()
		.click();
	await page.waitForURL(/\/use\?id=explore-app-1&eventId=explore-app-1-event/);
	assert.deepEqual(errors, []);
	console.log(
		"Explore browser checks passed: discovery, search, categories, URL history, sorting, pagination and retry, native links, owned app launch, loading/empty states, and 320–1440px layouts.",
	);
} finally {
	await browser.close();
}
