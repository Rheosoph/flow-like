import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
import { chromium } from "playwright-core";

const browser = await chromium.launch({
	executablePath:
		process.env.CHROME_EXECUTABLE_PATH ||
		"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
	headless: true,
	args: ["--no-sandbox"],
});
const page = await browser.newPage({ viewport: { width: 1480, height: 1050 } });
const report = { passed: [], geometry: [], errors: [], blocked: [] };
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
const frame = (type) => page.locator(`[data-widget-type="${type}"]`);
const content = (type) => page.locator(`[data-home-discovery="${type}"]`);
const types = [
	"app-spotlight",
	"app-collection-feature",
	"app-ranking",
	"model-spotlight",
];
const closePanel = async () => {
	const close = page.getByRole("button", {
		name: "Close widget panel",
		exact: true,
	});
	if (await close.count()) await close.last().click();
};
const edit = async () => {
	await page.getByRole("button", { name: "Customize", exact: true }).click();
	await closePanel();
};
const configure = async (type) => {
	await frame(type)
		.getByRole("button", { name: /^Configure / })
		.click();
	await page.getByLabel("Widget surface", { exact: true }).waitFor();
};
const save = async () => {
	await page.getByRole("button", { name: "Save", exact: true }).click();
	await page.locator('[data-home-editor][data-editing="false"]').waitFor();
};
const appearance = (type) =>
	frame(type).evaluate((element) => {
		const heading = element.querySelector("h2");
		return {
			background: getComputedStyle(element).backgroundColor,
			color: getComputedStyle(heading).color,
			accent: getComputedStyle(element).getPropertyValue("--home-accent"),
		};
	});
const choose = async (name) => {
	await page.getByLabel("Featured collection", { exact: true }).fill(name);
	await page
		.getByRole("checkbox", {
			name: `${name} ${name === "Field Notes" ? "Explore" : "Library"}`,
			exact: true,
		})
		.check();
};
const waitForHome = async () => {
	await page
		.getByRole("button", { name: "Customize", exact: true })
		.waitFor({ timeout: 60_000 });
	await content("app-spotlight").locator("h2").waitFor();
	await content("model-spotlight").locator("h2").waitFor();
	await page.waitForTimeout(350);
};
try {
	await page.goto(
		"http://127.0.0.1:4318/default-fixture?persist=1&customization=1",
		{
			waitUntil: "domcontentloaded",
		},
	);
	await waitForHome();
	await edit();
	for (const type of types) {
		console.log(`Checking appearance: ${type}`);
		await configure(type);
		await page
			.getByLabel("Widget surface", { exact: true })
			.selectOption("solid");
		await page
			.getByRole("button", { name: "blue accent", exact: true })
			.click();
		const blue = await appearance(type);
		assert.equal(
			blue.background,
			"rgb(96, 165, 250)",
			`${type}: solid blue reaches the visible frame`,
		);
		assert.equal(
			blue.color,
			"rgb(22, 19, 29)",
			`${type}: solid accent has readable dark text`,
		);
		await page
			.getByRole("button", { name: "emerald accent", exact: true })
			.click();
		const green = await appearance(type);
		assert.equal(
			green.background,
			"rgb(52, 211, 153)",
			`${type}: changing accent changes the card background`,
		);
		await page
			.getByLabel("Widget surface", { exact: true })
			.selectOption("tinted");
		const tinted = await appearance(type);
		assert.notEqual(
			tinted.background,
			green.background,
			`${type}: tinted differs from solid`,
		);
		await page
			.getByLabel("Widget surface", { exact: true })
			.selectOption("borderless");
		await closePanel();
		await save();
		assert.equal(
			(await appearance(type)).background,
			"rgba(0, 0, 0, 0)",
			`${type}: borderless removes the surface`,
		);
		await edit();
		await configure(type);
		await page
			.getByLabel("Widget surface", { exact: true })
			.selectOption("card");
		const card = await appearance(type);
		assert.notEqual(
			card.background,
			"rgba(0, 0, 0, 0)",
			`${type}: card restores a surface`,
		);
		await page
			.getByLabel("Widget surface", { exact: true })
			.selectOption(type === "app-collection-feature" ? "solid" : "tinted");
		await page
			.getByRole("button", { name: "blue accent", exact: true })
			.click();
	}
	report.passed.push(
		"Every discovery card responds to accent and solid, tinted, card, and borderless surfaces through the actual editor",
	);
	await configure("app-collection-feature");
	await page.getByLabel("Category", { exact: true }).selectOption("Business");
	await page.getByLabel("Search filter", { exact: true }).fill("Invoice");
	await page.getByLabel("Number of apps", { exact: true }).fill("1");
	await page
		.getByLabel("Apps to feature", { exact: true })
		.selectOption("manual");
	assert.equal(
		await page.getByLabel("Category", { exact: true }).count(),
		0,
		"Automatic category filtering is hidden for explicit selections",
	);
	await choose("Field Notes");
	await choose("Knowledge Chat");
	assert.equal(
		await page.getByLabel("Number of apps", { exact: true }).inputValue(),
		"2",
		"Selecting more apps expands the visible count",
	);
	await page
		.getByRole("button", { name: "Move Knowledge Chat up", exact: true })
		.click();
	await page.getByLabel("Number of apps", { exact: true }).fill("2");
	await page.getByLabel("Eyebrow", { exact: true }).fill("MY SELECTED APPS");
	await page.getByLabel("Title", { exact: true }).fill("A collection I chose");
	await page
		.getByLabel("Description", { exact: true })
		.fill("Personal apps and a useful community app, in my chosen order.");
	await save();
	const feature = content("app-collection-feature");
	await feature
		.getByRole("heading", { name: "A collection I chose", exact: true })
		.waitFor();
	const selectedLinks = () =>
		feature.locator('a[href^="/use?id="], a[href^="/store?id="]');
	assert.deepEqual(
		await selectedLinks().evaluateAll((links) =>
			links.map((link) => link.getAttribute("href")),
		),
		["/use?id=default-fixture-app-0", "/store?id=default-fixture-app-5"],
	);
	assert.match(await feature.innerText(), /MY SELECTED APPS/);
	const saved = await page.evaluate(() =>
		window.defaultHomeQa.saved.widgets.find(
			(widget) => widget.type === "app-collection-feature",
		),
	);
	assert.equal(saved.config.source, "manual");
	assert.deepEqual(saved.config.appIds, [
		"default-fixture-app-0",
		"default-fixture-app-5",
	]);
	assert.equal(saved.appearance.variant, "solid");
	assert.equal(saved.appearance.accent, "blue");
	report.passed.push(
		"A user can select community and owned apps, preserve chosen order, and override stale automatic filters; links open the correct store or runtime destination",
	);
	await page.reload({ waitUntil: "domcontentloaded" });
	await waitForHome();
	await feature
		.getByRole("heading", { name: "A collection I chose", exact: true })
		.waitFor();
	assert.equal(
		(await appearance("app-collection-feature")).background,
		"rgb(96, 165, 250)",
	);
	assert.deepEqual(
		await selectedLinks().evaluateAll((links) =>
			links.map((link) => link.getAttribute("href")),
		),
		["/use?id=default-fixture-app-0", "/store?id=default-fixture-app-5"],
	);
	await edit();
	await configure("app-collection-feature");
	assert.equal(
		await page.getByLabel("Widget surface", { exact: true }).inputValue(),
		"solid",
	);
	assert.equal(
		await page.getByLabel("Apps to feature", { exact: true }).inputValue(),
		"manual",
	);
	assert.equal(
		await page
			.getByRole("button", { name: "blue accent", exact: true })
			.getAttribute("aria-pressed"),
		"true",
	);
	await save();
	for (const theme of ["dark", "light"]) {
		await page.evaluate(
			(value) =>
				document.documentElement.classList.toggle("dark", value === "dark"),
			theme,
		);
		assert.equal(
			(await appearance("app-collection-feature")).color,
			"rgb(22, 19, 29)",
		);
		await feature.scrollIntoViewIfNeeded();
		await page.screenshot({
			path: `/private/tmp/home-customization-${theme}.png`,
		});
	}
	report.passed.push(
		"Color, surface, selected apps, order and copy survive a full reload; solid card text remains legible in dark and light themes",
	);
	await page.goto("http://127.0.0.1:4318/default-fixture?selection=legacy", {
		waitUntil: "domcontentloaded",
	});
	await waitForHome();
	assert.deepEqual(
		await selectedLinks().evaluateAll((links) =>
			links.map((link) => link.getAttribute("href")),
		),
		["/store?id=default-fixture-app-5", "/use?id=default-fixture-app-0"],
	);
	report.passed.push(
		"Existing manual collections ignore saved automatic query, category and tag filters and retain their selected app order",
	);
	await edit();
	await configure("app-spotlight");
	await page.getByLabel("Height", { exact: true }).selectOption("6");
	await configure("information");
	await page.getByLabel("Height", { exact: true }).selectOption("content");
	await save();
	const shortGuide = await frame("information").boundingBox();
	const ranking = await frame("app-spotlight").boundingBox();
	assert.ok(
		shortGuide.y + shortGuide.height < ranking.y + ranking.height - 40,
		"Fit content opts a widget out of matching its row's height",
	);
	await edit();
	await configure("information");
	await page.getByLabel("Height", { exact: true }).selectOption("auto");
	await save();
	const alignedGuide = await frame("information").boundingBox();
	const alignedRanking = await frame("app-spotlight").boundingBox();
	assert.ok(
		Math.abs(alignedGuide.height - alignedRanking.height) <= 1,
		"Match row restores equal heights",
	);
	report.passed.push(
		"The Height setting distinguishes Fit content from Match row and updates the actual rendered card height",
	);
	for (const catalog of ["normal", "varied"]) {
		await page.goto(
			`http://127.0.0.1:4318/default-fixture?catalog=${catalog}`,
			{ waitUntil: "domcontentloaded" },
		);
		await waitForHome();
		for (const width of [1480, 1024, 768, 390]) {
			await page.setViewportSize({ width, height: width < 768 ? 844 : 1050 });
			await page.waitForTimeout(350);
			const geometry = await page.evaluate(() => ({
				width: innerWidth,
				pageWidth: document.documentElement.scrollWidth,
				widgets: [...document.querySelectorAll("[data-home-widget]")].map(
					(element) => {
						const rect = element.getBoundingClientRect();
						return {
							type: element.getAttribute("data-widget-type"),
							x: rect.left,
							top: rect.top,
							bottom: rect.bottom,
							width: rect.width,
							height: rect.height,
						};
					},
				),
			}));
			report.geometry.push({ catalog, ...geometry });
			assert.ok(
				geometry.pageWidth <= width + 1,
				`${catalog}/${width}: no horizontal overflow`,
			);
			const rows = new Map();
			for (const widget of geometry.widgets) {
				const key = Math.round(widget.top);
				rows.set(key, [...(rows.get(key) ?? []), widget]);
			}
			for (const row of rows.values()) {
				if (row.length < 2) continue;
				assert.ok(
					Math.max(...row.map((widget) => widget.bottom)) -
						Math.min(...row.map((widget) => widget.bottom)) <=
						1,
					`${catalog}/${width}: cards sharing a row have matching bottoms: ${JSON.stringify(row)}`,
				);
			}
			for (let index = 1; index < geometry.widgets.length; index++) {
				assert.ok(
					geometry.widgets[index].top >= geometry.widgets[index - 1].top - 1,
					`${catalog}/${width}: DOM order follows visual row order`,
				);
			}
			if (width === 390) {
				assert.ok(
					geometry.widgets.find((widget) => widget.type === "information")
						.height < 500,
					`${catalog}: mobile guide fits its own content`,
				);
				assert.ok(
					geometry.widgets.find((widget) => widget.type === "app-spotlight")
						.height < 600,
					`${catalog}: mobile spotlight avoids stretched desktop height`,
				);
			}
			await content("app-spotlight").scrollIntoViewIfNeeded();
			await page.screenshot({
				path: `/private/tmp/home-customization-${catalog}-${width}.png`,
			});
		}
	}
	report.passed.push(
		"Normal and varied content align into rows at 1480, 1024 and 768 pixels; mobile cards return to content height and fit without horizontal overflow",
	);
	assert.deepEqual(report.errors, []);
	assert.deepEqual(report.blocked, []);
} catch (error) {
	report.failure = String(error.stack ?? error);
	await page.screenshot({
		path: "/private/tmp/home-customization-failure.png",
	});
	console.log((await page.locator("body").innerText()).slice(-8000));
} finally {
	await writeFile(
		"/private/tmp/home-customization-report.json",
		JSON.stringify(report, null, 2),
	);
	console.log(JSON.stringify(report, null, 2));
	await browser.close();
}
if (report.failure) process.exitCode = 1;
