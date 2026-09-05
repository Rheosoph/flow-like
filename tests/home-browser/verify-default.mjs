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
	args: ["--no-sandbox"],
});
const page = await browser.newPage({ viewport: { width: 1480, height: 1050 } });
const report = { passed: [], scenarios: {}, errors: [], blocked: [] };
page.on("pageerror", (error) =>
	report.errors.push(error.stack || error.message),
);
page.on("console", (message) => {
	if (message.type() === "error") report.errors.push(message.text());
});
await page.route("**/*", (route) => {
	const url = new URL(route.request().url());
	if (url.hostname === "127.0.0.1" || ["data:", "blob:"].includes(url.protocol))
		return route.continue();
	report.blocked.push(url.origin);
	return route.abort();
});
const widgets = () => page.locator("[data-home-widget]");
const greeting = () => page.locator("[data-home-greeting] h1");
const top = async () => {
	await widgets()
		.first()
		.evaluate((element) => element.scrollIntoView({ block: "start" }));
	await page.waitForTimeout(300);
};
const imagesReady = () =>
	page.waitForFunction(() =>
		[...document.querySelectorAll("[data-home-widget] img")].every(
			(img) => img.loading === "lazy" || img.complete,
		),
	);
const fit = async (label) => {
	const dimensions = await page.evaluate(() => ({
		width: innerWidth,
		page: document.documentElement.scrollWidth,
		canvases: [...document.querySelectorAll("[data-home-canvas]")].map(
			(element) => ({
				client: element.clientWidth,
				scroll: element.scrollWidth,
			}),
		),
	}));
	assert.ok(dimensions.page <= dimensions.width + 1, `${label}: page fits`);
	assert.ok(
		dimensions.canvases.every((item) => item.scroll <= item.client + 1),
		`${label}: canvas fits`,
	);
};
const screenshot = async (name) => {
	await imagesReady();
	await page.screenshot({ path: `/private/tmp/home-bold-${name}.png` });
};
try {
	await page.goto("http://127.0.0.1:4318/default-fixture", {
		waitUntil: "domcontentloaded",
	});
	await page
		.getByRole("button", { name: "Customize", exact: true })
		.waitFor({ timeout: 60_000 });
	await greeting().filter({ hasText: "Felix" }).waitFor();
	assert.ok(
		(await page.evaluate(() => window.defaultHomeQa.calls.account)) > 0,
		"Greeting loads the account name when claims omit it",
	);
	report.passed.push(
		"Greeting resolves Felix from the account response when JWT name claims are absent",
	);
	for (const scenario of ["returning", "fresh", "offline", "guest"]) {
		await page
			.getByLabel("Profile state", { exact: true })
			.selectOption(scenario);
		await page.waitForFunction(
			(expected) => window.defaultHomeQa?.scenario === expected,
			scenario,
		);
		await greeting().waitFor();
		if (scenario === "returning" || scenario === "fresh")
			await greeting().filter({ hasText: "Felix" }).waitFor();
		if (scenario === "offline")
			await greeting().filter({ hasText: "Cached" }).waitFor();
		const spotlight = page.locator('[data-home-discovery="app-spotlight"]');
		if (scenario === "returning" || scenario === "offline") {
			await spotlight
				.getByRole("link", { name: "Open app", exact: true })
				.waitFor();
			assert.equal(
				await spotlight
					.getByRole("link", { name: "Open app", exact: true })
					.getAttribute("href"),
				"/use?id=default-fixture-app-0",
			);
		} else {
			await spotlight
				.getByRole("link", { name: "Explore app", exact: true })
				.waitFor();
			assert.equal(
				await spotlight
					.getByRole("link", { name: "Explore app", exact: true })
					.getAttribute("href"),
				"/store?id=default-fixture-app-0",
			);
		}
		await page
			.locator('[data-home-discovery="app-ranking"]')
			.getByText("By community ratings", { exact: true })
			.waitFor();
		for (const theme of scenario === "returning"
			? ["dark", "light"]
			: ["dark"]) {
			await page.evaluate(
				(value) =>
					document.documentElement.classList.toggle("dark", value === "dark"),
				theme,
			);
			for (const width of scenario === "returning"
				? [1480, 768, 390, 320, 2048]
				: [1480, 390]) {
				await page.setViewportSize({ width, height: width < 768 ? 844 : 1050 });
				await top();
				await fit(`${scenario}/${theme}/${width}`);
				await screenshot(`${scenario}-${theme}-${width}-top`);
				if ([1480, 390].includes(width)) {
					const count = await widgets().count();
					await widgets()
						.nth(Math.floor(count / 2))
						.evaluate((element) => element.scrollIntoView({ block: "start" }));
					await page.waitForTimeout(250);
					await screenshot(`${scenario}-${theme}-${width}-middle`);
					await widgets()
						.last()
						.evaluate((element) => element.scrollIntoView({ block: "end" }));
					await page.waitForTimeout(250);
					await fit(`${scenario}/${theme}/${width}/bottom`);
					await screenshot(`${scenario}-${theme}-${width}-bottom`);
				}
			}
		}
		const state = await page.evaluate(() => ({
			calls: window.defaultHomeQa.calls,
			widgets: [...document.querySelectorAll("[data-home-widget]")].map(
				(element) => element.getAttribute("data-widget-type"),
			),
			greeting: document.querySelector("[data-home-greeting] h1")?.textContent,
		}));
		report.scenarios[scenario] = state;
		if (scenario === "offline" || scenario === "guest") {
			assert.equal(
				state.calls.history ?? 0,
				0,
				`${scenario}: account history is never queried`,
			);
			assert.equal(
				state.calls.usage ?? 0,
				0,
				`${scenario}: private usage is never queried`,
			);
			assert.equal(
				state.calls.account ?? 0,
				0,
				`${scenario}: account identity is never queried`,
			);
		}
		if (scenario === "guest")
			assert.equal(
				state.greeting.includes("Felix"),
				false,
				"Guest does not retain another session's account name",
			);
	}
	report.passed.push(
		"Returning, fresh, offline and guest defaults render without page/canvas overflow; signed-out states never fetch account history or identity",
	);
	report.passed.push(
		"Returning default fits 320, 390, 768, 1480 and 2048 pixels in dark and light themes",
	);
	await page
		.getByLabel("Profile state", { exact: true })
		.selectOption("returning");
	await page.setViewportSize({ width: 1480, height: 1050 });
	await top();
	await page.getByRole("button", { name: "Customize", exact: true }).click();
	await page
		.getByRole("button", { name: "Close widget panel", exact: true })
		.last()
		.click();
	await page
		.locator('[data-widget-type="greeting"]')
		.getByRole("button", { name: /^Configure / })
		.click();
	await page.getByLabel("Name", { exact: true }).fill("Sam");
	await page.getByRole("button", { name: "Save", exact: true }).click();
	await page.locator('[data-home-editor][data-editing="false"]').waitFor();
	assert.match(await greeting().innerText(), /, Sam$/);
	assert.equal(
		await page.evaluate(
			() =>
				window.defaultHomeQa.saved.widgets.find(
					(item) => item.type === "greeting",
				).config.name,
		),
		"Sam",
	);
	report.passed.push(
		"The greeting Name setting overrides the account name and persists through the real editor save callback",
	);
	await page.getByRole("button", { name: "Customize", exact: true }).click();
	await page
		.getByRole("button", { name: "Close widget panel", exact: true })
		.last()
		.click();
	const customIds = await widgets().evaluateAll((elements) =>
		elements.map((element) => element.getAttribute("data-home-widget")),
	);
	await page
		.getByRole("button", { name: "Layout options", exact: true })
		.click();
	await page
		.getByRole("menuitem", { name: "Use Flow-Like starter", exact: true })
		.click();
	await greeting().filter({ hasText: "Felix" }).waitFor();
	assert.equal(await widgets().count(), 11);
	assert.equal(
		await page.evaluate(
			() =>
				window.defaultHomeQa.saved.widgets.find(
					(item) => item.type === "greeting",
				).config.name,
		),
		"Sam",
		"Loading the starter does not save immediately",
	);
	await page
		.getByRole("button", { name: "Undo layout change", exact: true })
		.click();
	await greeting().filter({ hasText: "Sam" }).waitFor();
	assert.deepEqual(
		await widgets().evaluateAll((elements) =>
			elements.map((element) => element.getAttribute("data-home-widget")),
		),
		customIds,
		"Undo restores the exact personalized widget IDs",
	);
	await page
		.getByRole("button", { name: "Redo layout change", exact: true })
		.click();
	await greeting().filter({ hasText: "Felix" }).waitFor();
	await page.getByRole("button", { name: "Save", exact: true }).click();
	await page.locator('[data-home-editor][data-editing="false"]').waitFor();
	assert.equal(
		await page.evaluate(
			() =>
				window.defaultHomeQa.saved.widgets.find(
					(item) => item.type === "greeting",
				).config.name,
		),
		undefined,
	);
	assert.equal(
		await page.evaluate(() => window.defaultHomeQa.calls.resets ?? 0),
		0,
		"Starter saves a personal layout rather than changing default inheritance",
	);
	report.passed.push(
		"Use Flow-Like starter loads a draft, supports exact undo/redo, and saves the starter as the user's personal layout",
	);
	await page.goto(
		"http://127.0.0.1:4318/default-fixture?scenario=fresh&catalog=empty",
		{ waitUntil: "domcontentloaded" },
	);
	const emptyHero = page.locator('[data-home-discovery="app-spotlight"]');
	await emptyHero
		.getByRole("heading", { name: "Build your next big idea.", exact: true })
		.waitFor();
	await page
		.locator('[data-home-discovery="model-spotlight"]')
		.getByText("Find the right model", { exact: true })
		.waitFor();
	assert.equal(
		await emptyHero
			.getByRole("link", { name: "Open your library", exact: true })
			.getAttribute("href"),
		"/library",
	);
	await emptyHero
		.getByRole("button", { name: "Build with FlowPilot", exact: true })
		.click();
	await page.waitForFunction(
		() => window.defaultHomeQa.flowPilotState() === "overlay",
	);
	await top();
	await screenshot("empty-catalog-dark-1480-top");
	await page.setViewportSize({ width: 390, height: 844 });
	await top();
	await fit("empty-catalog/390");
	await screenshot("empty-catalog-dark-390-top");
	await page.goto(
		"http://127.0.0.1:4318/default-fixture?scenario=fresh&catalog=unrated",
		{ waitUntil: "domcontentloaded" },
	);
	const unratedHero = page.locator('[data-home-discovery="app-spotlight"]');
	await unratedHero
		.getByRole("heading", { name: "Knowledge Chat", exact: true })
		.waitFor();
	assert.equal(
		await unratedHero.locator("dl").count(),
		0,
		"Missing rating and download data are not rendered as metrics",
	);
	report.passed.push(
		"Owned apps open their runtime, unowned apps open their store page, and rankings explain their community-rating source",
	);
	report.passed.push(
		"A genuinely empty catalog and profile shows a usable FlowPilot onboarding hero; unrated apps show no invented rating or download metrics",
	);
	await page.goto(
		"http://127.0.0.1:4318/default-fixture?scenario=returning&ownership=deferred",
		{ waitUntil: "domcontentloaded" },
	);
	await page.waitForFunction(
		() => typeof window.defaultHomeQa?.resolveOwnership === "function",
	);
	const loadingApp = page
		.locator('[data-home-discovery="app-spotlight"]')
		.getByRole("link", { name: "Loading app…", exact: true });
	await loadingApp.waitFor();
	assert.equal(await loadingApp.getAttribute("aria-disabled"), "true");
	const pendingUrl = page.url();
	await loadingApp.click({ force: true });
	assert.equal(
		page.url(),
		pendingUrl,
		"Pending ownership cannot navigate an owned app to its store page",
	);
	await page.evaluate(() => window.defaultHomeQa.resolveOwnership());
	const readyApp = page
		.locator('[data-home-discovery="app-spotlight"]')
		.getByRole("link", { name: "Open app", exact: true });
	await readyApp.waitFor();
	assert.equal(
		await readyApp.getAttribute("href"),
		"/use?id=default-fixture-app-0",
	);
	assert.equal(await readyApp.getAttribute("aria-disabled"), null);
	report.passed.push(
		"App links stay disabled while ownership loads, then resolve to the owned app runtime without premature store navigation",
	);
	assert.deepEqual(report.errors, []);
	assert.deepEqual(report.blocked, []);
} catch (error) {
	report.failure = error.stack;
	await page.screenshot({ path: "/private/tmp/home-bold-failure.png" });
	throw error;
} finally {
	await writeFile(
		"/private/tmp/home-bold-report.json",
		JSON.stringify(report, null, 2),
	);
	console.log(JSON.stringify(report, null, 2));
	await browser.close();
}
