import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { chromium } from "playwright-core";
import { rows } from "./data.ts";

const baseUrl = process.env.QUERY_PREVIEW_URL || "http://127.0.0.1:4345/";
const output = process.env.QUERY_SCREENSHOT_DIR || "/tmp/query-workbench-qa";
await mkdir(output, { recursive: true });
const browser = await chromium.launch({
	executablePath:
		process.env.CHROME_EXECUTABLE_PATH ||
		(process.platform === "darwin"
			? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
			: undefined),
	headless: true,
	args: ["--no-sandbox"],
});
const errors = [];
const checks = [];
let currentPage;

async function openPage(width = 1440, height = 1000, query = "") {
	const context = await browser.newContext({
		viewport: { width, height },
		deviceScaleFactor: 1,
		permissions: ["clipboard-read", "clipboard-write"],
	});
	const page = await context.newPage();
	currentPage = page;
	page.setDefaultTimeout(15_000);
	page.on("pageerror", (error) => errors.push(error.message));
	page.on("console", (message) => {
		if (message.type() === "error") errors.push(message.text());
	});
	const url = new URL(baseUrl);
	if (query) url.search = query;
	await page.goto(url.href, { waitUntil: "networkidle", timeout: 120_000 });
	await page.getByRole("textbox", { name: "SQL query editor" }).waitFor();
	return page;
}

async function capture(page, name) {
	await page.mouse.move(0, 0);
	await page.screenshot({
		path: `${output}/${name}.png`,
		fullPage: true,
		animations: "disabled",
	});
}

async function fits(page) {
	const size = await page.evaluate(() => ({
		width: innerWidth,
		content: document.documentElement.scrollWidth,
	}));
	assert.ok(size.content <= size.width + 1, JSON.stringify(size));
}

async function chooseSaved(page, name) {
	const library = page.getByRole("button", { name: "Toggle saved queries" });
	if ((await library.getAttribute("aria-expanded")) !== "true")
		await library.click();
	await page.getByRole("button", { name: new RegExp(`^${name}`) }).click();
	await page.getByRole("heading", { name, exact: true }).waitFor();
	if ((await library.getAttribute("aria-expanded")) === "true")
		await library.click();
}

const runButton = (page) =>
	page.getByRole("button", { name: /^Run(?:\s+⌘↵)?$/ });
const resultTable = (page) =>
	page.getByRole("table", { name: "Query results", exact: true });
async function expectRows(page, count) {
	await page.waitForFunction(
		(expected) =>
			document
				.querySelector("table[aria-rowcount]")
				?.getAttribute("aria-rowcount") === String(expected + 1),
		count,
	);
}
async function runSaved(page, name, count) {
	await chooseSaved(page, name);
	await runButton(page).click();
	if (count) await expectRows(page, count);
}
async function closeDialog(page) {
	await page
		.getByRole("dialog")
		.getByRole("button", { name: "Close", exact: true })
		.click();
	await page.getByRole("dialog").waitFor({ state: "hidden" });
}
async function assertClipboard(page, expected) {
	await page.waitForFunction(
		async (text) => (await navigator.clipboard.readText()) === text,
		expected,
	);
}

try {
	const page = await openPage();
	await capture(page, "01-ready");
	await page
		.getByRole("button", { name: "Browse tables", exact: true })
		.first()
		.click();
	const picker = page.getByRole("dialog", {
		name: "Browse tables",
		exact: true,
	});
	await picker
		.getByRole("textbox", { name: "Search tables and views", exact: true })
		.fill("not-a-table");
	await picker
		.getByText("No matching tables or views", { exact: true })
		.waitFor();
	await picker
		.getByRole("button", { name: "Clear search", exact: true })
		.click();
	await picker
		.getByRole("textbox", { name: "Search tables and views", exact: true })
		.fill("data");
	await capture(page, "17-browse-tables");
	await picker.getByRole("button", { name: "Query data", exact: true }).click();
	await runButton(page).click();
	await expectRows(page, 8);
	assert.match(
		await page.evaluate(
			() =>
				window.queryWorkbenchQa.calls
					.filter((call) => call.method === "executeSql")
					.at(-1).args[0].sql,
		),
		/SELECT\s+\*\s+FROM\s+"data"/i,
	);
	checks.push("Browse tables searches the catalog and creates runnable SQL");
	await runSaved(page, "Process documents", 8);
	const rowHeights = await resultTable(page)
		.locator("tbody tr[aria-rowindex]")
		.evaluateAll((items) =>
			items.map((item) => item.getBoundingClientRect().height),
		);
	assert.equal(rowHeights.length, 8);
	assert.ok(
		rowHeights.every((height) => Math.abs(height - 33) < 0.1),
		JSON.stringify(rowHeights),
	);
	await fits(page);
	await capture(page, "02-results-dark");
	checks.push("Saved query runs and returns 8 records");

	const filter = page.getByRole("textbox", { name: "Filter result rows" });
	await filter.fill("BS_0034_2010");
	await expectRows(page, 1);
	assert.equal(
		await page.getByRole("button", { name: "Export all", exact: true }).count(),
		1,
	);
	await filter.fill("no-document-matches-this");
	await page.getByText("No rows match", { exact: false }).waitFor();
	await page
		.getByRole("button", { name: "Clear row filter", exact: true })
		.click();
	await expectRows(page, 8);
	checks.push("Filtering, no matches, and clearing preserve the full result");

	await resultTable(page)
		.getByRole("button", { name: "Preview content", exact: true })
		.first()
		.focus();
	await page.keyboard.press("Enter");
	let dialog = page.getByRole("dialog", {
		name: "Preview content",
		exact: true,
	});
	await dialog
		.getByRole("heading", { name: "Collar joining: cold forming", exact: true })
		.waitFor();
	await capture(page, "03-content-formatted");
	await dialog.getByRole("button", { name: "Raw", exact: true }).click();
	assert.equal(await dialog.locator("pre").textContent(), rows[0].content);
	await dialog.getByRole("button", { name: "Copy", exact: true }).click();
	await assertClipboard(page, rows[0].content);
	await capture(page, "04-content-raw");
	await dialog.getByRole("button", { name: "Formatted", exact: true }).click();
	await dialog
		.getByRole("heading", { name: "Collar joining: cold forming", exact: true })
		.waitFor();
	await closeDialog(page);
	checks.push(
		"Keyboard opens a cell; Markdown, raw content, and complete clipboard copy agree",
	);

	await page
		.getByRole("button", { name: "Inspect row 1", exact: true })
		.click();
	dialog = page.getByRole("dialog", { name: "Row details", exact: true });
	await dialog.getByText("10 columns", { exact: true }).waitFor();
	await dialog
		.getByRole("button", { name: "Copy as JSON", exact: true })
		.click();
	await assertClipboard(page, JSON.stringify(rows[0], null, 2));
	await capture(page, "05-row-inspector");
	await closeDialog(page);
	checks.push("Row inspector exposes all fields and copies the complete row");

	await page.getByRole("tab", { name: "JSON", exact: true }).click();
	assert.deepEqual(
		JSON.parse(
			await page
				.getByRole("tabpanel", { name: "JSON", exact: true })
				.locator("pre")
				.innerText(),
		),
		rows,
	);
	await page.getByRole("tab", { name: "Table", exact: true }).click();
	await expectRows(page, 8);
	const idHeader = resultTable(page)
		.getByRole("columnheader")
		.filter({ has: page.getByRole("button", { name: "id", exact: true }) });
	await idHeader.getByRole("button", { name: "id", exact: true }).click();
	assert.equal(await idHeader.getAttribute("aria-sort"), "ascending");
	await idHeader.getByRole("button", { name: "id", exact: true }).click();
	assert.equal(await idHeader.getAttribute("aria-sort"), "descending");
	assert.equal(
		await resultTable(page)
			.getByRole("button", { name: "Preview id", exact: true })
			.first()
			.innerText(),
		rows.at(-1).id,
	);
	checks.push(
		"JSON contains every row and column sorting changes the visible order",
	);

	const editor = page.locator('[data-panel-id="qw-editor"]');
	const beforeHeight = (await editor.boundingBox()).height;
	await page
		.getByRole("button", { name: "Expand results", exact: true })
		.click();
	await page
		.getByRole("button", { name: "Restore editor", exact: true })
		.waitFor();
	assert.ok((await editor.boundingBox()).height < 1);
	await capture(page, "06-expanded-results");
	await page
		.getByRole("button", { name: "Restore editor", exact: true })
		.click();
	assert.ok(Math.abs((await editor.boundingBox()).height - beforeHeight) < 2);
	checks.push("Results expand and restore the previous editor height");

	await runSaved(page, "Empty result");
	await page.getByText("No rows returned", { exact: true }).waitFor();
	assert.equal(
		await page
			.getByRole("button", { name: "Export", exact: true })
			.isDisabled(),
		true,
	);
	await capture(page, "07-empty-results");
	await runSaved(page, "Query error");
	await page
		.getByRole("alert")
		.getByText(/missing_table/)
		.waitFor();
	await capture(page, "08-query-error");
	checks.push("Empty and failed queries show dedicated result states");

	await page.getByRole("button", { name: "New query", exact: true }).click();
	await page.getByRole("button", { name: "Save", exact: true }).click();
	dialog = page.getByRole("dialog", { name: "Save query", exact: true });
	await dialog
		.getByRole("textbox", { name: "Name", exact: true })
		.fill("Verification query");
	await dialog
		.getByRole("textbox", { name: "Description", exact: true })
		.fill("Saved through the real query form");
	await dialog.getByRole("button", { name: "Save", exact: true }).click();
	await dialog.waitFor({ state: "hidden" });
	await page
		.getByRole("heading", { name: "Verification query", exact: true })
		.waitFor();
	assert.equal(
		await page.evaluate(() =>
			window.queryWorkbenchQa.library.some(
				(item) => item.name === "Verification query" && item.sql === "SELECT 1",
			),
		),
		true,
	);
	checks.push("New query saves through the form and becomes the active query");

	await runSaved(page, "Process documents", 8);
	await page
		.getByRole("button", { name: "Toggle color theme", exact: true })
		.click();
	await page.waitForFunction(
		() => !document.documentElement.classList.contains("dark"),
	);
	await capture(page, "09-results-light");
	await fits(page);
	checks.push("Results render in the light theme without page overflow");
	await page.getByRole("tab", { name: "Record", exact: true }).click();
	await page.getByText("Row 1 of 8", { exact: true }).waitFor();
	assert.equal(
		await page
			.getByRole("button", { name: "Previous row", exact: true })
			.isDisabled(),
		true,
	);
	await page.getByRole("button", { name: "Next row", exact: true }).click();
	await page.getByText("Row 2 of 8", { exact: true }).waitFor();
	await page.getByRole("button", { name: "Copy row", exact: true }).click();
	await assertClipboard(page, JSON.stringify(rows[1], null, 2));
	await page.getByRole("button", { name: "Previous row", exact: true }).click();
	await page.getByText("Row 1 of 8", { exact: true }).waitFor();
	checks.push(
		"Record view browses rows and copies the selected complete record",
	);
	await page
		.getByRole("button", { name: "Expand results", exact: true })
		.click();
	await page
		.locator("[data-sonner-toast]")
		.first()
		.waitFor({ state: "hidden" });
	await capture(page, "19-record-light");
	await page
		.getByRole("button", { name: "Restore editor", exact: true })
		.click();
	await runSaved(page, "Find a document");
	await page.getByText("Row 1 of 1", { exact: true }).waitFor();
	assert.equal(
		await page
			.getByRole("tab", { name: "Record", exact: true })
			.getAttribute("aria-selected"),
		"true",
	);
	assert.equal(
		await page
			.getByRole("button", { name: "Next row", exact: true })
			.isDisabled(),
		true,
	);
	await page
		.getByRole("button", { name: "Toggle color theme", exact: true })
		.click();
	await page.waitForFunction(() =>
		document.documentElement.classList.contains("dark"),
	);
	await page
		.getByRole("button", { name: "Expand results", exact: true })
		.click();
	await capture(page, "13-single-record-1440");
	await page
		.getByRole("heading", { name: "Collar joining: cold forming", exact: true })
		.scrollIntoViewIfNeeded();
	await capture(page, "14-single-record-content");
	await page.getByRole("tab", { name: "Table", exact: true }).click();
	await expectRows(page, 1);
	checks.push(
		"A single result opens in Record view and can switch back to Table",
	);

	const compact = await openPage(1000, 900);
	await runSaved(compact, "Process documents", 8);
	await fits(compact);
	await capture(compact, "15-results-1000");
	await runSaved(compact, "Find a document");
	await compact.getByText("Row 1 of 1", { exact: true }).waitFor();
	await compact
		.getByRole("button", { name: "Expand results", exact: true })
		.click();
	await fits(compact);
	await capture(compact, "16-single-record-1000");

	const narrow = await openPage(390, 844);
	await chooseSaved(narrow, "Documents by process");
	if (
		!(await narrow
			.getByRole("dialog", { name: "Query parameters", exact: true })
			.isVisible())
	)
		await narrow
			.getByRole("button", { name: "Parameters (1)", exact: true })
			.click();
	dialog = narrow.getByRole("dialog", {
		name: "Query parameters",
		exact: true,
	});
	await dialog
		.getByRole("combobox", { name: "Process *", exact: true })
		.click();
	await narrow.getByRole("option", { name: "Forming", exact: true }).click();
	await capture(narrow, "10-narrow-parameters");
	await dialog.getByRole("button", { name: "Done", exact: true }).click();
	await runButton(narrow).click();
	await expectRows(narrow, 2);
	await fits(narrow);
	await capture(narrow, "11-narrow-results");
	assert.equal(
		await narrow.evaluate(
			() =>
				window.queryWorkbenchQa.calls
					.filter((call) => call.method === "executeSql")
					.at(-1).args[0].params.process,
		),
		"Forming",
	);
	checks.push(
		"390px layout opens parameters in a sheet and Forming returns 2 rows",
	);
	await runSaved(narrow, "Find a document");
	await narrow.getByText("Row 1 of 1", { exact: true }).waitFor();
	await narrow
		.getByRole("button", { name: "Expand results", exact: true })
		.click();
	await fits(narrow);
	await capture(narrow, "18-single-record-narrow");
	await narrow
		.getByRole("heading", { name: "Collar joining: cold forming", exact: true })
		.scrollIntoViewIfNeeded();
	await fits(narrow);
	await capture(narrow, "20-record-content-narrow");

	const many = await openPage(1440, 1000, "many");
	await runSaved(many, "Process documents", 400);
	const table = resultTable(many);
	assert.ok((await table.locator("tbody tr[aria-rowindex]").count()) < 100);
	await table.evaluate((element) => {
		const scroller = element.parentElement;
		scroller.scrollTop = scroller.scrollHeight;
	});
	await many
		.getByRole("button", { name: "Inspect row 400", exact: true })
		.waitFor();
	assert.equal(
		await table
			.getByRole("button", { name: "Preview id", exact: true })
			.last()
			.innerText(),
		"QA_0400",
	);
	assert.ok((await table.locator("tbody tr[aria-rowindex]").count()) < 100);
	const lastRow = await table
		.locator("tbody tr[aria-rowindex]")
		.last()
		.boundingBox();
	assert.ok(Math.abs(lastRow.height - 33) < 0.1);
	await capture(many, "12-virtualized-final-row");
	checks.push(
		"400-row result virtualizes and reaches its final 33px row without drift",
	);

	assert.deepEqual(errors, [], "Browser console and page errors");
	console.log(
		JSON.stringify(
			{ checks, screenshots: output, browserErrors: errors },
			null,
			2,
		),
	);
} catch (error) {
	if (currentPage) {
		await capture(currentPage, "failure").catch(() => {});
		console.error(
			(await currentPage.locator("body").innerText()).slice(0, 6000),
		);
	}
	console.error(
		JSON.stringify({ completed: checks, browserErrors: errors }, null, 2),
	);
	throw error;
} finally {
	await browser.close();
}
