import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
import { chromium } from "playwright-core";
const browser = await chromium.launch({
	executablePath:
		process.env.CHROME_EXECUTABLE_PATH ||
		"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
	headless: true,
	args: ["--disable-dev-shm-usage", "--no-sandbox"],
});
const page = await browser.newPage({ viewport: { width: 1440, height: 1080 } });
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
const waitUntil = async (check) => {
	for (let n = 0; n < 50; n++) {
		if (await check()) return;
		await page.waitForTimeout(100);
	}
	throw new Error("Condition did not settle");
};
const nativeApps = page.getByTestId("native-apps");
const nativeModels = page.getByTestId("native-models");
const profileModels = page.getByTestId("profile-models");
const cardStyle = page.getByLabel("Card style", { exact: true });
const detailButton = (container, name) =>
	container.getByRole("button", {
		name: `View Fixture ${name} model model details`,
		exact: true,
	});
try {
	await page.goto("http://127.0.0.1:4318/collections-fixture", {
		waitUntil: "domcontentloaded",
	});
	await page
		.getByRole("heading", {
			name: "Native collections · local fixture",
			exact: true,
		})
		.waitFor({ timeout: 90000 });
	await nativeApps
		.getByText("Fixture Knowledge Chat", { exact: true })
		.waitFor();
	await detailButton(nativeModels, "reasoning").waitFor();
	assert.equal(
		await nativeApps.getByText("Fixture Hidden App", { exact: true }).count(),
		0,
	);
	assert.equal(
		await nativeModels
			.getByRole("button", { name: "In profile", exact: true })
			.count(),
		1,
	);
	assert.equal(await detailButton(profileModels, "reasoning").count(), 1);
	assert.equal(await detailButton(profileModels, "embedding").count(), 0);
	assert.equal(
		await nativeApps
			.locator(
				'[data-home-collection-rendering="standard"] [data-href="/use?id=collection-app-0"]',
			)
			.count(),
		1,
	);
	assert.equal(
		await page
			.getByTestId("native-editorial")
			.locator('a[href="/store?id=collection-app-3"]')
			.count(),
		1,
	);
	report.passed.push(
		"Standard renders canonical library/model cards with current-profile visibility and correct owned/store destinations despite a poisoned unscoped profile cache",
	);
	await nativeModels
		.getByRole("button", { name: "Add to profile", exact: true })
		.click();
	await detailButton(profileModels, "embedding").waitFor();
	assert.deepEqual(await page.evaluate(() => window.collectionsQa.writes[0]), {
		action: "add",
		profile: "collection-profile-a",
		bit: "collection-model-1",
	});
	const added = nativeModels
		.locator("article")
		.filter({ hasText: "Fixture embedding model" });
	await added.getByRole("button", { name: "In profile", exact: true }).click();
	await waitUntil(
		async () => (await detailButton(profileModels, "embedding").count()) === 0,
	);
	assert.equal(
		await page.evaluate(() => window.collectionsQa.writes[1].action),
		"remove",
	);
	report.passed.push(
		"Native Add/remove writes the captured profile and refreshes the separate profile-model widget immediately",
	);
	const modelTitle = detailButton(nativeModels, "reasoning");
	await modelTitle.focus();
	await page.keyboard.press("Enter");
	await page.getByRole("dialog").waitFor();
	assert.ok(
		(await page.getByRole("dialog").innerText()).includes(
			"Fixture reasoning model",
		),
	);
	await page.keyboard.press("Escape");
	await waitUntil(async () => (await page.getByRole("dialog").count()) === 0);
	await nativeModels
		.getByRole("button", { name: "Model actions", exact: true })
		.first()
		.click();
	assert.equal(
		await page.getByRole("menuitem", { name: /^Download|^Remove$/ }).count(),
		0,
	);
	await page.keyboard.press("Escape");
	report.passed.push(
		"Keyboard opens the real model detail sheet and hosted model menus omit irrelevant local-file actions",
	);
	await page.getByLabel("Fixture profile").selectOption("b");
	await nativeApps.getByText("Fixture Invoice OCR", { exact: true }).waitFor();
	await detailButton(profileModels, "embedding").waitFor();
	assert.equal(await detailButton(profileModels, "reasoning").count(), 0);
	await nativeModels
		.getByRole("button", { name: "Add to profile", exact: true })
		.click();
	await detailButton(profileModels, "reasoning").waitFor();
	assert.equal(
		await page.evaluate(() => window.collectionsQa.writes[2].profile),
		"collection-profile-b",
	);
	await page.getByLabel("Fixture profile").selectOption("a");
	await nativeApps
		.getByText("Fixture Knowledge Chat", { exact: true })
		.waitFor();
	assert.equal(await detailButton(profileModels, "embedding").count(), 0);
	report.passed.push(
		"Switching profiles isolates library visibility, model membership, and mutation targets",
	);
	for (const width of [1440, 390]) {
		await page.setViewportSize({ width, height: 1080 });
		for (const rendering of [
			"standard",
			"compact",
			"list",
			"editorial",
			"icons",
			"carousel",
		]) {
			await cardStyle.nth(0).selectOption(rendering);
			await nativeApps
				.locator(`[data-home-collection-rendering="${rendering}"]`)
				.waitFor();
			await cardStyle
				.nth(1)
				.selectOption(rendering === "list" ? "list" : "standard");
			await page.waitForTimeout(100);
			const overflow = await page.evaluate(() => ({
				scroll: document.documentElement.scrollWidth,
				client: document.documentElement.clientWidth,
			}));
			assert.ok(
				overflow.scroll <= overflow.client + 1,
				`${rendering} fits ${width}px: ${JSON.stringify(overflow)}`,
			);
			const nestedScrollers = await nativeApps.evaluate(
				(element) =>
					[...element.querySelectorAll("*")].filter(
						(node) =>
							["auto", "scroll"].includes(getComputedStyle(node).overflowY) &&
							node.scrollHeight > node.clientHeight + 1,
					).length,
			);
			assert.equal(
				nestedScrollers,
				0,
				`${rendering} has no nested vertical scrollbar`,
			);
		}
		await cardStyle.nth(0).selectOption("standard");
		await cardStyle.nth(1).selectOption("standard");
		await page.screenshot({
			path: `/private/tmp/home-native-collections-${width}.png`,
			fullPage: true,
		});
	}
	await page.getByLabel("Fixture surface").selectOption("tinted");
	assert.equal(await cardStyle.nth(0).inputValue(), "standard");
	report.passed.push(
		"All six app rendering choices and native model list/grid fit narrow desktop containers and 390px screens, without nested vertical scrollers; surface changes preserve the card choice",
	);
	await page.getByLabel("Fixture scenario").selectOption("empty");
	await nativeApps
		.getByText("No apps match this collection yet.", { exact: true })
		.waitFor();
	await nativeModels
		.getByText(
			"No models match this search. Try a different search in widget settings.",
			{ exact: true },
		)
		.waitFor();
	await page.getByLabel("Fixture scenario").selectOption("error");
	await nativeApps
		.getByRole("button", { name: "Try again", exact: true })
		.waitFor();
	await nativeModels
		.getByRole("button", { name: "Try again", exact: true })
		.waitFor();
	await page.getByLabel("Fixture scenario").selectOption("populated");
	await nativeApps
		.getByText("Fixture Knowledge Chat", { exact: true })
		.waitFor();
	await detailButton(nativeModels, "reasoning").waitFor();
	report.passed.push(
		"Empty, offline, retry and recovery states remain visible for native collections",
	);
	assert.deepEqual(report.blocked, []);
	assert.deepEqual(report.errors, []);
} finally {
	await writeFile(
		"/private/tmp/home-native-collections-report.json",
		JSON.stringify(report, null, 2),
	);
	console.log(JSON.stringify(report, null, 2));
	await browser.close();
}
