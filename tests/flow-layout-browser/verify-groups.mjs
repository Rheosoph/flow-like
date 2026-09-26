import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import puppeteer from "puppeteer";

const origin = process.env.FLOW_GROUP_QA_URL || "http://127.0.0.1:4336";
const output = join(tmpdir(), "flow-like-group-verification");
await mkdir(output, { recursive: true });
const browser = await puppeteer.launch({
	headless: true,
	executablePath:
		process.env.CHROME_PATH ||
		"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
	args: ["--no-sandbox", "--disable-gpu"],
	userDataDir: `${output}/browser-${process.pid}`,
});
const page = await browser.newPage();
const errors = [];
page.on("pageerror", (error) => errors.push(error.message));
await page.setViewport({ width: 1600, height: 1000, deviceScaleFactor: 1 });
const snapshot = () => page.evaluate(() => window.groupQa.snapshot());
async function click(label) {
	const handle = await page.evaluateHandle(
		(text) =>
			[...document.querySelectorAll("button")].find(
				(button) =>
					button.textContent.trim() === text ||
					button.getAttribute("aria-label") === text,
			),
		label,
	);
	assert.ok(handle.asElement(), `Missing button: ${label}`);
	await handle.asElement().click();
	await handle.dispose();
}
async function settled() {
	await page.evaluate(
		() =>
			new Promise((resolve) =>
				requestAnimationFrame(() => requestAnimationFrame(resolve)),
			),
	);
}
try {
	await page.goto(`${origin}/groups.html`, {
		waitUntil: "domcontentloaded",
		timeout: 60000,
	});
	await page.waitForFunction(
		() =>
			window.groupQa &&
			document.querySelectorAll(".react-flow__node").length === 12,
	);
	await page.evaluate(() => window.groupQa.fit());
	await settled();
	const initial = await snapshot();
	assert.equal(initial.open, false);
	await click("Find groups");
	await page.waitForSelector("[data-group-outline]");
	const detected = await snapshot();
	assert.equal(detected.suggestions.length, 2);
	for (const candidate of detected.suggestions) {
		assert.equal(candidate.nodeCount, 4);
		assert.equal(candidate.inputCount, 1);
		assert.equal(candidate.outputCount, 1);
	}
	assert.deepEqual(detected.board, initial.board, "Detection is read-only");
	await settled();
	assert.equal(
		await page.evaluate(() => {
			const strip = document.querySelector('[aria-label="Group suggestions"]');
			const bounds = strip.getBoundingClientRect();
			return [...document.querySelectorAll("[data-group-outline] button")].some(
				(button) => {
					const badge = button.getBoundingClientRect();
					return (
						badge.left < bounds.right &&
						badge.right > bounds.left &&
						badge.top < bounds.bottom &&
						badge.bottom > bounds.top
					);
				},
			);
		}),
		false,
		"Suggestion badges avoid the review strip",
	);
	await page.screenshot({ path: `${output}/suggestions.png` });
	const member = detected.suggestions[0].memberIds[0];
	const target = await page.$(`[data-id="${member}"].react-flow__node`);
	const rect = await target.boundingBox();
	const hitNode = await page.evaluate(
		({ x, y }) =>
			document
				.elementFromPoint(x, y)
				?.closest(".react-flow__node")
				?.getAttribute("data-id"),
		{ x: rect.x + rect.width / 2, y: rect.y + 8 },
	);
	assert.equal(
		hitNode,
		member,
		"Outlines allow pointer events through to nodes",
	);
	await page.mouse.click(rect.x + rect.width / 2, rect.y + 8);
	await page.waitForFunction(
		(id) =>
			document
				.querySelector(`[data-id="${id}"].react-flow__node`)
				?.classList.contains("selected"),
		{},
		member,
	);
	assert.equal(
		(await snapshot()).suggestions.length,
		2,
		"Canvas selection does not replace suggestions during review",
	);
	const outlineBefore = await page.$eval(
		"[data-group-outline]",
		(element) => element.style.cssText,
	);
	const moving = detected.suggestions[0].memberIds.find((id) =>
		id.endsWith("-b"),
	);
	assert.ok(moving);
	const movedFrom = initial.board.nodes[moving].coordinates;
	await page.evaluate(({ id, x, y }) => window.groupQa.move(id, x, y), {
		id: moving,
		x: movedFrom[0] + 20,
		y: movedFrom[1] - 40,
	});
	await page.waitForFunction(
		(before) =>
			document.querySelector("[data-group-outline]").style.cssText !== before,
		{},
		outlineBefore,
	);
	await page.evaluate(({ id, x, y }) => window.groupQa.move(id, x, y), {
		id: moving,
		x: movedFrom[0],
		y: movedFrom[1],
	});
	await settled();
	await click("Preview");
	await page.waitForSelector("[data-group-preview]");
	assert.ok(
		await page.$$eval(
			"[data-group-preview-wires] path",
			(paths) => paths.length >= 2,
		),
		"Preview renders boundary wires",
	);
	assert.deepEqual(
		(await snapshot()).board,
		initial.board,
		"Preview does not mutate graph",
	);
	await page.screenshot({ path: `${output}/preview.png` });
	const previewPosition = await page.$eval(
		"[data-group-preview]",
		(element) => [
			Number.parseFloat(element.style.left),
			Number.parseFloat(element.style.top),
		],
	);
	await click("Collapse");
	await page.waitForFunction(() => window.groupQa.snapshot().revision === 1);
	const collapsed = await snapshot();
	assert.equal(collapsed.commands.length, 1);
	assert.equal(collapsed.commands[0].command_type, "UpsertLayer");
	assert.deepEqual(
		collapsed.commands[0].layer.coordinates.slice(0, 2),
		previewPosition,
		"The collapsed group starts at the preview position",
	);
	assert.deepEqual(
		[...collapsed.commands[0].node_ids].sort(),
		[...detected.suggestions[0].memberIds].sort(),
	);
	assert.equal(Object.keys(collapsed.board.layers).length, 1);
	await page.waitForFunction(
		() => document.querySelectorAll(".react-flow__node").length === 9,
	);
	await page.screenshot({ path: `${output}/collapsed.png` });
	await click("Undo");
	await page.waitForFunction(() => window.groupQa.snapshot().revision === 2);
	assert.deepEqual(
		(await snapshot()).board,
		initial.board,
		"One undo restores board topology",
	);
	await click("Dismiss");
	await page.waitForFunction(
		() => window.groupQa.snapshot().suggestions.length === 0,
	);
	await page.keyboard.press("Escape");
	await page.waitForFunction(() => !window.groupQa.snapshot().open);
	await page.evaluate(
		(ids) => window.groupQa.setSelected(ids),
		detected.suggestions[0].memberIds,
	);
	await settled();
	await click("Find groups");
	await page.waitForFunction(
		() => window.groupQa.snapshot().suggestions.length === 1,
	);
	assert.deepEqual(
		(await snapshot()).suggestions[0].memberIds,
		detected.suggestions[0].memberIds,
	);
	await page.evaluate(() => window.groupQa.setReadOnly(true));
	await page.waitForFunction(() => !window.groupQa.snapshot().open);
	assert.equal(
		await page.$eval("header button", (button) => button.disabled),
		true,
	);
	await page.evaluate(() => {
		window.groupQa.setReadOnly(false);
		window.groupQa.setSelected([]);
	});
	await settled();
	await click("Auto Layout");
	await page.waitForFunction(() =>
		[...document.querySelectorAll("button")].some((button) =>
			button.textContent.includes("Review 2 grouping suggestions"),
		),
	);
	assert.equal(
		(await snapshot()).open,
		false,
		"Auto layout offers review without opening the mode",
	);
	await click("Review 2 grouping suggestions");
	await page.waitForFunction(() => window.groupQa.snapshot().open);
	await page.keyboard.press("Escape");
	await page.waitForFunction(() => !window.groupQa.snapshot().open);
	assert.deepEqual(errors, []);
	assert.deepEqual((await snapshot()).errors, []);
	await writeFile(
		`${output}/results.json`,
		JSON.stringify(
			{
				passed: true,
				checks: [
					"detection",
					"badge and review strip spacing",
					"pointer passthrough",
					"canvas selection",
					"drag tracking",
					"preview ports and wires",
					"preview read-only",
					"single command collapse",
					"preview and collapse position",
					"undo",
					"dismiss",
					"selection scope",
					"read-only",
					"post-layout review",
					"Escape",
				],
				errors,
			},
			null,
			2,
		),
	);
	console.log(
		`PASS grouping browser verification: detection, click-through, live bounds, preview, collapse/undo, dismiss, selection, read-only, post-layout offer, and Escape. Artifacts: ${output}`,
	);
} catch (error) {
	console.error(error.message);
	await page.screenshot({ path: `${output}/failure.png` }).catch(() => {});
	await writeFile(
		`${output}/failure.json`,
		JSON.stringify(
			{
				errors,
				message: error.message,
				snapshot: await snapshot().catch(() => undefined),
			},
			null,
			2,
		),
	);
	throw error;
} finally {
	await browser.close();
}
