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
const page = await browser.newPage({ viewport: { width: 1480, height: 1050 } });
const report = { passed: [], errors: [], blocked: [] };
await page.route("**/*", (route) => {
	const url = new URL(route.request().url());
	if (url.hostname === "127.0.0.1" || ["data:", "blob:"].includes(url.protocol))
		return route.continue();
	report.blocked.push(url.origin);
	return route.abort();
});
page.on("pageerror", (error) => {
	report.errors.push(error.message);
	console.log(error.stack);
});
page.on("console", (message) => {
	if (message.type() === "error") {
		report.errors.push(message.text());
		console.log(message.text());
	}
});
const widget = (title) =>
	page
		.locator("[data-home-widget]")
		.filter({ has: page.getByRole("heading", { name: title, exact: true }) });
const add = async (name) => {
	await page.getByRole("button", { name: "Add widget", exact: true }).click();
	await page.getByRole("button", { name: `Add ${name}`, exact: true }).click();
};
const cleanToasts = () =>
	page.waitForFunction(
		() => !document.querySelector('[data-sonner-toast][data-visible="true"]'),
	);
try {
	await page.goto("http://127.0.0.1:4318/", { waitUntil: "domcontentloaded" });
	await page
		.getByRole("button", { name: "Customize", exact: true })
		.waitFor({ timeout: 60_000 });
	await page.getByRole("button", { name: "Customize", exact: true }).click();
	await page
		.getByRole("button", { name: "Add Resource directory", exact: true })
		.click();
	await page.getByLabel("Title", { exact: true }).fill("Team resources");
	await page.getByLabel("Width", { exact: true }).selectOption("6");
	await page
		.getByRole("button", { name: /^Add (step|entry)$/, exact: false })
		.click();
	await page.getByLabel(/^(Step|Entry) 1$/).fill("Report guide");
	await page
		.getByLabel(/^(Answer|Description) 1$/)
		.fill("Review **monthly trends** before sharing the report.");
	await page.getByLabel("Label 1", { exact: true }).fill("GUIDE");
	await page
		.getByLabel("Destination 1", { exact: true })
		.fill("/use?id=fixture-app-0&route=%2Freports&period=month");
	await add("Guided steps");
	await page.getByLabel("Title", { exact: true }).fill("Monthly review");
	await page.getByRole("button", { name: "Add step", exact: true }).click();
	await page
		.getByLabel("Step 1", { exact: true })
		.fill("Review incoming invoices");
	await page
		.getByLabel(/^(Answer|Description) 1$/)
		.fill("Start with invoices that need a **second look**.");
	await page
		.getByLabel("Destination 1", { exact: true })
		.fill("/use?id=fixture-app-1");
	await add("Editorial story");
	await page.getByLabel("Title", { exact: true }).fill("A calmer start");
	await page.getByLabel("Width", { exact: true }).selectOption("6");
	await page
		.getByLabel("Content", { exact: true })
		.fill(
			"## Bring your work together\nYour home brings useful app pages and reference material into one place.",
		);
	await page.getByLabel("Label", { exact: true }).fill("WORKSPACE NOTES");
	await page
		.getByLabel("Button label", { exact: true })
		.fill("Open your library");
	await page.getByLabel("Button destination", { exact: true }).fill("/library");
	await add("Checklist");
	await page.getByLabel("Title", { exact: true }).fill("Review checklist");
	await page
		.getByLabel("Step 1", { exact: true })
		.fill("Review the monthly report");
	await page.getByRole("button", { name: "Add step", exact: true }).click();
	await page.getByLabel("Step 2", { exact: true }).fill("Share the findings");
	await page.getByRole("button", { name: "Save", exact: true }).click();
	await page.locator('[data-home-editor][data-editing="false"]').waitFor();
	await widget("Team resources")
		.getByRole("heading", { name: "Report guide", exact: true })
		.waitFor();
	assert.equal(
		await widget("Team resources")
			.getByRole("link", { name: "Open Report guide" })
			.getAttribute("href"),
		"/use?id=fixture-app-0&route=%2Freports&period=month",
	);
	assert.equal(
		await widget("Team resources").locator("strong").innerText(),
		"monthly trends",
	);
	assert.equal(
		await widget("Monthly review")
			.getByRole("link", { name: "Open Review incoming invoices" })
			.getAttribute("href"),
		"/use?id=fixture-app-1",
	);
	assert.equal(
		await widget("A calmer start")
			.getByRole("link", { name: "Open your library" })
			.getAttribute("href"),
		"/library",
	);
	await widget("A calmer start")
		.getByRole("heading", { name: "Bring your work together" })
		.waitFor();
	report.passed.push(
		"Resource directory, guided steps, editorial story, Markdown, labels, and destinations configure and render after saving",
	);
	await page.evaluate(() => {
		window.homeQa.holdSave = true;
	});
	const attempts = await page.evaluate(
		() => window.homeQa.counters.saveAttempts,
	);
	await widget("Review checklist")
		.getByRole("checkbox", { name: "Review the monthly report" })
		.check();
	await page.waitForFunction(() => !!window.homeQa.releaseSave);
	assert.equal(
		await page
			.getByRole("button", { name: "Customize", exact: true })
			.isDisabled(),
		true,
	);
	assert.equal(
		await widget("Review checklist").evaluate(
			(element) => !!element.closest("[inert]"),
		),
		true,
	);
	assert.equal(
		await page.evaluate(() => window.homeQa.counters.saveAttempts),
		attempts + 1,
	);
	await page.evaluate(() => {
		window.homeQa.holdSave = false;
		window.homeQa.releaseSave();
	});
	await page
		.getByRole("button", { name: "Customize", exact: true })
		.waitFor({ state: "visible" });
	await page.waitForFunction(
		() =>
			window.homeQa
				.getSaved()
				.widgets.find((widget) => widget.title === "Review checklist").config
				.items[0].checked,
	);
	await widget("Review checklist")
		.getByText("1 of 2 complete", { exact: true })
		.waitFor();
	report.passed.push(
		"Checklist saves runtime changes and locks editing plus canvas interaction while a save is pending",
	);
	await page.evaluate(() => {
		window.homeQa.failSave = true;
	});
	await widget("Review checklist")
		.getByRole("checkbox", { name: "Share the findings" })
		.click();
	await page
		.getByText("Could not save this change. Try again.", { exact: true })
		.waitFor();
	assert.equal(
		await widget("Review checklist")
			.getByRole("checkbox", { name: "Share the findings" })
			.isChecked(),
		false,
	);
	await page.evaluate(() => {
		window.homeQa.failSave = false;
	});
	report.passed.push("Failed checklist save restores the previous saved state");
	await cleanToasts();
	await widget("Team resources").scrollIntoViewIfNeeded();
	await page.screenshot({
		path: "/private/tmp/home-qa-rich-content.png",
		fullPage: true,
	});
	await page.goto("http://127.0.0.1:4318/?default=1", {
		waitUntil: "domcontentloaded",
	});
	await page.getByRole("button", { name: "Customize", exact: true }).waitFor();
	await page.waitForFunction(
		() => document.querySelectorAll("[data-home-widget]").length === 16,
	);
	await page
		.getByRole("heading", { name: "Community favorites", exact: true })
		.waitFor();
	for (const width of [1480, 768, 390]) {
		await page.setViewportSize({ width, height: width === 390 ? 844 : 1050 });
		await page.locator("[data-home-canvas]").evaluate((element) => {
			let parent = element.parentElement;
			while (parent) {
				parent.scrollTop = 0;
				parent = parent.parentElement;
			}
		});
		assert.equal(
			await page.evaluate(
				() => document.documentElement.scrollWidth > innerWidth,
			),
			false,
		);
		await page.screenshot({
			path: `/private/tmp/home-qa-default-${width}.png`,
			fullPage: true,
		});
	}
	report.passed.push(
		"Actual default layout renders sixteen catalog widgets at 1480px, 768px, and 390px without horizontal page overflow",
	);
	assert.deepEqual(report.errors, []);
} catch (error) {
	report.failure = String(error.stack ?? error);
	await page.screenshot({
		path: "/private/tmp/home-qa-content-failure.png",
		fullPage: true,
	});
	console.log((await page.locator("body").innerText()).slice(-8000));
} finally {
	await writeFile(
		"/private/tmp/home-qa-content-report.json",
		JSON.stringify(report, null, 2),
	);
	console.log(JSON.stringify(report, null, 2));
	await browser.close();
}
if (report.failure) process.exitCode = 1;
