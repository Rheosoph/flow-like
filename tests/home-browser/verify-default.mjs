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
	await page
		.locator('[data-widget-type="workspace-pulse"] [data-workspace-pulse]')
		.waitFor();
	await page.waitForFunction(() => window.defaultHomeQa.calls.history === 1);
	assert.equal(await page.locator("[data-home-section-heading]").count(), 1);
	assert.equal(await widgets().count(), 11);
	assert.equal(
		await page
			.locator(
				'[data-home-discovery="app-ranking"], [data-home-discovery="app-collection-feature"]',
			)
			.count(),
		0,
	);
	assert.equal(
		await page.locator('[data-widget-type="workspace-pulse"]').count(),
		1,
	);
	const guides = page.locator('[data-widget-type="information"]');
	assert.equal(
		await guides
			.getByRole("link", { name: "Build your first flow", exact: true })
			.getAttribute("href"),
		"/learn",
	);
	const guideRows = await guides.locator("article").evaluateAll((elements) =>
		elements.map((element) => {
			const rect = element.getBoundingClientRect();
			return {
				left: Math.round(rect.left),
				top: rect.top,
				bottom: rect.bottom,
			};
		}),
	);
	assert.equal(guideRows.length, 3);
	assert.match(
		await guides.locator("article").first().innerText(),
		/Keep an app open here/,
	);
	assert.equal(new Set(guideRows.map((row) => row.left)).size, 1);
	assert.ok(
		guideRows.every(
			(row, index) => index === 0 || row.top >= guideRows[index - 1].bottom - 1,
		),
		"Useful guides form one readable vertical list",
	);
	assert.equal(
		await page.evaluate(() => window.defaultHomeQa.calls.history),
		1,
		"The workspace overview uses one execution history read",
	);
	report.passed.push(
		"The workspace overview combines metrics and flagged records in one widget, with one editable discovery heading",
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
		if (scenario === "fresh") {
			const personal = page.locator('[data-widget-type="app-collection"]');
			await personal
				.getByText("Your apps will live here.", { exact: true })
				.waitFor();
			assert.equal(
				await personal
					.getByRole("link", { name: "Find your first app", exact: true })
					.getAttribute("href"),
				"/store/explore/apps",
			);
			await page
				.locator('[data-widget-type="workspace-pulse"]')
				.getByText("Start with one useful app", { exact: true })
				.waitFor();
		}
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
				const geometry = await page.evaluate(() => {
					const top = document
						.querySelector("[data-home-canvas]")
						.getBoundingClientRect().top;
					const rect = (selector) => {
						const value = document
							.querySelector(selector)
							.getBoundingClientRect();
						return { y: value.top - top, height: value.height };
					};
					return {
						actions: rect('[data-widget-type="quick-actions"]'),
						flowpilot: rect('[data-widget-type="flowpilot"]'),
						apps: rect('[data-widget-type="app-collection"]'),
						hero: rect('[data-widget-type="app-spotlight"]'),
						packages: rect('[data-widget-type="packages"]'),
					};
				});
				assert.ok(
					geometry.flowpilot.y < geometry.actions.y &&
						geometry.actions.y < geometry.apps.y,
					"FlowPilot and direct creation/import actions appear before personal apps",
				);
				assert.ok(
					geometry.apps.y < geometry.hero.y,
					"Personal apps appear before discovery",
				);
				if (width === 390) {
					assert.ok(
						geometry.apps.y < 1000,
						"Personal apps are within the first mobile scroll",
					);
					assert.ok(
						geometry.packages.height < 900,
						"Compact packages avoid a long mobile card wall",
					);
					assert.ok(
						geometry.hero.height < 500,
						"Compact mobile hero leaves room for other content",
					);
				}
				if (scenario === "returning" && [1480, 390].includes(width)) {
					const dates = await page
						.locator("[data-home-app-updated]")
						.evaluateAll((elements) =>
							elements.map((element) => {
								const date = element.getBoundingClientRect();
								const row = element.parentElement.getBoundingClientRect();
								const card =
									element.parentElement.firstElementChild.getBoundingClientRect();
								const frame = element
									.closest("[data-home-widget]")
									.getBoundingClientRect();
								return {
									x: Math.round(row.left),
									dateTop: date.top,
									dateBottom: date.bottom,
									cardBottom: card.bottom,
									rowBottom: row.bottom,
									frameBottom: frame.bottom,
								};
							}),
						);
					assert.equal(dates.length, 4);
					assert.equal(
						new Set(dates.map((item) => item.x)).size,
						width === 1480 ? 2 : 1,
					);
					assert.ok(
						dates.every(
							(item) =>
								item.dateTop >= item.cardBottom - 1 &&
								item.dateBottom <= item.rowBottom + 1 &&
								item.dateBottom <= item.frameBottom + 1,
						),
						"App update dates stay below their cards and inside the measured frame",
					);
				}
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
	await page
		.locator('[data-home-widget="default-discover-heading"]')
		.getByRole("button", { name: /^Configure / })
		.click();
	await page.getByLabel("Title", { exact: true }).fill("My studio");
	await page
		.getByLabel("Description", { exact: true })
		.fill("Apps and records for my current projects.");
	await page
		.locator('[data-widget-type="app-collection"]')
		.getByRole("button", { name: /^Configure / })
		.click();
	await page.getByLabel("Maximum columns", { exact: true }).selectOption("1");
	await page.getByRole("button", { name: "Save", exact: true }).click();
	await page.locator('[data-home-editor][data-editing="false"]').waitFor();
	assert.match(await greeting().innerText(), /, Sam$/);
	await page.getByRole("heading", { name: "My studio", exact: true }).waitFor();
	assert.equal(
		await page
			.locator('[data-home-widget="default-discover-heading"]')
			.getByRole("link", { name: "Explore apps", exact: true })
			.getAttribute("href"),
		"/store/explore/apps",
	);
	assert.equal(
		await page.evaluate(
			() =>
				window.defaultHomeQa.saved.widgets.find(
					(item) => item.type === "greeting",
				).config.name,
		),
		"Sam",
	);
	assert.equal(
		await page.evaluate(
			() =>
				window.defaultHomeQa.saved.widgets.find(
					(item) => item.type === "app-collection",
				).config.maxColumns,
		),
		1,
	);
	report.passed.push(
		"The greeting name and section heading text remain editable and persist through the real editor save callback",
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
			.getByRole("link", { name: "Library", exact: true })
			.getAttribute("href"),
		"/library",
	);
	await emptyHero
		.getByRole("button", { name: "Build with FlowPilot", exact: true })
		.click();
	await page.waitForFunction(
		() => window.defaultHomeQa.flowPilotState() === "overlay",
	);
	await emptyHero.evaluate((element) =>
		element.scrollIntoView({ block: "center" }),
	);
	await screenshot("empty-catalog-dark-1480-top");
	await page.setViewportSize({ width: 390, height: 844 });
	await emptyHero.evaluate((element) =>
		element.scrollIntoView({ block: "center" }),
	);
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
	assert.equal(
		await unratedHero.getByText("Rating", { exact: true }).count(),
		0,
	);
	assert.equal(
		await unratedHero.getByText("Downloads", { exact: true }).count(),
		0,
	);
	report.passed.push(
		"Owned apps open their runtime and unowned apps open their store page",
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
