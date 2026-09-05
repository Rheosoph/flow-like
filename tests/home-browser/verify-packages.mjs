import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
import { chromium } from "playwright-core";

const browser = await chromium.launch({
	executablePath:
		process.env.CHROME_EXECUTABLE_PATH ||
		(process.platform === "darwin"
			? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
			: undefined),
	headless: true,
	args: ["--disable-dev-shm-usage", "--no-sandbox"],
});
const page = await browser.newPage({ viewport: { width: 1480, height: 1080 } });
const report = { passed: [], errors: [], blocked: [] };
await page.route("**/*", (route) => {
	const url = new URL(route.request().url());
	if (url.hostname === "127.0.0.1" && url.pathname === "/api/v1")
		return route.fulfill({ json: {} });
	if (url.hostname === "127.0.0.1" || ["data:", "blob:"].includes(url.protocol))
		return route.continue();
	report.blocked.push(url.origin);
	return route.abort();
});
page.on("pageerror", (error) => report.errors.push(error.message));
page.on("console", (message) => {
	if (message.type() === "error") report.errors.push(message.text());
});
const settle = async (predicate) => {
	for (let n = 0; n < 60; n++) {
		if (await predicate()) return;
		await page.waitForTimeout(100);
	}
	throw new Error("Package UI did not settle");
};
const home = page.getByTestId("home-packages");
try {
	await page.goto("http://127.0.0.1:4318/packages-fixture", {
		waitUntil: "domcontentloaded",
	});
	await home.locator("[data-package-card]").first().waitFor({ timeout: 90000 });
	assert.equal(await home.locator('[data-package-card="standard"]').count(), 3);
	const explore = page
		.getByTestId("explore-reference")
		.locator("[data-package-card]");
	assert.equal(
		await home
			.locator("[data-package-card]")
			.first()
			.evaluate((node) => node.outerHTML),
		await explore.evaluate((node) => node.outerHTML),
	);
	assert.equal(
		await home
			.locator('a[href="/store/packages?id=default-fixture-package-0"]')
			.count(),
		1,
	);
	assert.equal(await home.getByText("8.4k", { exact: true }).count(), 1);
	assert.equal(await home.getByText("4.9", { exact: true }).count(), 1);
	assert.equal(await home.getByText("Free", { exact: true }).count(), 3);
	assert.equal(await home.getByText("+1", { exact: true }).count(), 3);
	report.passed.push(
		"Home Standard is the exact canonical Explore PackageCard markup, with real artwork, capability overflow, rating, installs, price and canonical detail links",
	);
	const metadata = page.getByTestId("package-metadata-cases");
	const encodedId = "qa/paid package?mode=a&team=#1";
	for (const link of await metadata.locator("a").all()) {
		assert.equal(
			await link.getAttribute("href"),
			`/store/packages?id=${encodeURIComponent(encodedId)}`,
		);
		assert.equal(await link.getByText("€12.99", { exact: true }).count(), 1);
		assert.equal(await link.getByText("New", { exact: true }).count(), 1);
		assert.equal(
			await link.getByText("no permissions requested", { exact: true }).count(),
			1,
		);
		assert.equal(await link.getByText("v3.2.1", { exact: true }).count(), 1);
		assert.equal(await link.locator("img").count(), 0);
		assert.equal(await link.locator('[title="private"]').count(), 1);
	}
	report.passed.push(
		"All variants preserve paid price, unrated status, explicit empty capabilities, private visibility and version with absent artwork; package IDs are URL-encoded",
	);
	for (const theme of ["dark", "light"]) {
		await page.evaluate(
			(value) =>
				document.documentElement.classList.toggle("dark", value === "dark"),
			theme,
		);
		for (const width of [1480, 768, 390, 320]) {
			await page.setViewportSize({ width, height: 1080 });
			for (const variant of ["standard", "compact", "featured"]) {
				await page
					.getByLabel("Package card style", { exact: true })
					.selectOption(variant);
				await settle(
					async () =>
						(await home.locator(`[data-package-card="${variant}"]`).count()) ===
						3,
				);
				const measurement = await home.evaluate((node) => {
					const cards = [...node.querySelectorAll("[data-package-card]")].map(
						(card) => {
							const rect = card.getBoundingClientRect();
							return { left: rect.left, top: rect.top, width: rect.width };
						},
					);
					return {
						cards,
						overflow: document.documentElement.scrollWidth > innerWidth + 1,
						nested: node.querySelectorAll("a a, a button").length,
					};
				});
				assert.equal(
					measurement.overflow,
					false,
					`${variant} overflow at ${width}`,
				);
				assert.equal(measurement.nested, 0);
				if (width >= 1480)
					assert.equal(
						new Set(measurement.cards.map((card) => Math.round(card.top))).size,
						1,
					);
				if (width <= 390)
					assert.equal(
						new Set(measurement.cards.map((card) => Math.round(card.left)))
							.size,
						1,
					);
				if ([1480, 390].includes(width)) {
					await home
						.locator("[data-package-card]")
						.first()
						.evaluate((node) =>
							node.scrollIntoView({ block: "start", behavior: "instant" }),
						);
					await page.screenshot({
						path: `/private/tmp/home-native-packages-${theme}-${variant}-${width}.png`,
					});
				}
			}
		}
	}
	report.passed.push(
		"Standard, Compact and Featured remain selectable through real settings in light and dark themes at 320, 390, 768 and 1480px; desktop uses three columns and phones one, with no overflow or nested links/buttons",
	);
	await page.getByLabel("Search", { exact: true }).fill("empty");
	await home.getByText("No packages match this search.").waitFor();
	assert.equal(
		await home
			.getByRole("link", { name: "Explore all packages" })
			.getAttribute("href"),
		"/store/packages",
	);
	await page.getByLabel("Search", { exact: true }).fill("fail");
	await home.getByRole("button", { name: "Try again" }).waitFor();
	await page.getByLabel("Search", { exact: true }).fill("");
	await home.locator('[data-package-card="featured"]').first().waitFor();
	report.passed.push(
		"Empty filters provide Explore all packages, registry failures provide retry, and clearing the filter restores the selected card presentation",
	);
	await page.evaluate(() =>
		sessionStorage.removeItem("package-editor-qa-layout"),
	);
	await page.goto("http://127.0.0.1:4318/packages-fixture?editor=1", {
		waitUntil: "domcontentloaded",
	});
	await page.setViewportSize({ width: 1480, height: 1080 });
	await page.getByRole("button", { name: "Customize", exact: true }).click();
	await page
		.getByRole("button", { name: "Configure Explore packages", exact: true })
		.click();
	await page
		.getByLabel("Package card style", { exact: true })
		.selectOption("featured");
	await page.getByRole("button", { name: "Save", exact: true }).click();
	await page.locator('[data-home-editor][data-editing="false"]').waitFor();
	const saved = await page.evaluate(() =>
		JSON.parse(sessionStorage.getItem("package-editor-qa-layout")),
	);
	assert.equal(saved.widgets[0].config.rendering, "featured");
	await page.reload({ waitUntil: "domcontentloaded" });
	await page
		.locator(
			'[data-home-widget="packages-fixture"] [data-package-card="featured"]',
		)
		.first()
		.waitFor();
	assert.equal(
		await page
			.locator(
				'[data-home-widget="packages-fixture"] [data-package-card="featured"]',
			)
			.count(),
		3,
	);
	await page.screenshot({
		path: "/private/tmp/home-native-packages-editor-restored.png",
	});
	report.passed.push(
		"Production HomeEditor saves the selected package style and the serialized layout restores the same featured cards after reload",
	);
	assert.deepEqual(report.errors, []);
	assert.deepEqual(report.blocked, []);
} catch (error) {
	report.failure = error.stack || String(error);
	await page.screenshot({
		path: "/private/tmp/home-native-packages-failure.png",
	});
	process.exitCode = 1;
} finally {
	await writeFile(
		"/private/tmp/home-native-packages-report.json",
		JSON.stringify(report, null, 2),
	);
	await browser.close();
}
console.log(JSON.stringify(report, null, 2));
