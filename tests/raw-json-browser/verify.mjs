import assert from "node:assert/strict";
import puppeteer from "puppeteer";

// Start bunx vite --config tests/raw-json-browser/vite.config.ts first.
const origin = process.env.RAW_JSON_ORIGIN || "http://127.0.0.1:4354";
const browser = await puppeteer.launch({
	headless: true,
	executablePath:
		process.env.CHROME_EXECUTABLE_PATH ||
		(process.platform === "darwin"
			? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
			: undefined),
	args: ["--no-sandbox"],
});
const errors = [];
const passed = [];

try {
	const page = await browser.newPage();
	page.setDefaultTimeout(30000);
	page.on("pageerror", (error) => errors.push(error.message));
	await page.setViewport({ width: 1280, height: 800 });
	await page.setRequestInterception(true);
	page.on("request", (request) => {
		const url = new URL(request.url());
		if (url.origin === origin || ["data:", "blob:"].includes(url.protocol)) {
			void request.continue();
		} else void request.abort();
	});
	await browser
		.defaultBrowserContext()
		.overridePermissions(origin, [
			"clipboard-read",
			"clipboard-write",
			"clipboard-sanitized-write",
		]);
	await page.goto(origin, { waitUntil: "networkidle0" });
	await page.waitForSelector(".fixture-toolbar button");
	await page.click(".fixture-toolbar button");
	await page.evaluate(() => {
		window.rawJsonQa.underlyingClicks = 0;
	});
	await page.waitForFunction(
		() => window.monaco?.editor.getEditors().length === 1,
	);
	await page.waitForSelector(".monaco-editor .view-lines");
	const focused = () =>
		page.evaluate(() => window.monaco.editor.getEditors()[0].hasTextFocus());
	const button = (label) =>
		page.waitForFunction(
			(label) =>
				[...document.querySelectorAll("button")].find(
					(node) => node.textContent.trim() === label,
				),
			{},
			label,
		);
	const clickButton = async (label) => {
		const handle = await button(label);
		await handle.asElement().click();
		await handle.dispose();
	};
	const assertBlocked = async () => {
		assert.equal(
			await page.evaluate(() => window.rawJsonQa.underlyingClicks),
			0,
			"Dialog clicks must not reach the underlying page",
		);
		assert.equal(
			await page.evaluate(() => window.getSelection().toString()),
			"",
			"Dialog clicks must not select underlying text",
		);
	};

	// This coordinate overlaps body-portalled builder chrome above the builder's stacking context.
	await page.mouse.click(400, 680, { clickCount: 2 });
	await assertBlocked();
	assert.equal(
		await focused(),
		true,
		"Lower blank editor space must focus Monaco",
	);
	passed.push(
		"Dialog escapes the builder stacking context and blocks page selection",
	);

	const dialog = await page.$('[role="dialog"]');
	assert.ok(dialog, "Raw JSON uses the shared modal dialog");
	const layout = await page.evaluate(() => {
		const editor = document
			.querySelector(".monaco-editor")
			.getBoundingClientRect();
		const dialog = document
			.querySelector('[role="dialog"]')
			.getBoundingClientRect();
		return { editorBottom: editor.bottom, dialogBottom: dialog.bottom };
	});
	assert.ok(
		layout.dialogBottom - layout.editorBottom < 80,
		"Editor fills the dialog body",
	);
	const format = await button("Format");
	for (const point of [
		{ x: 600, y: 30 },
		{ x: 1200, y: 770 },
	]) {
		await format.asElement().focus();
		assert.equal(await focused(), false);
		await page.mouse.click(point.x, point.y);
		assert.equal(
			await focused(),
			true,
			"Blank dialog chrome must refocus Monaco",
		);
		await assertBlocked();
	}
	await format.dispose();
	passed.push("Blank header and footer space refocus Monaco");

	await page.evaluate(() => {
		const editor = window.monaco.editor.getEditors()[0];
		editor.setValue('{"components":[],"widgetRefs":{}}');
		editor.setPosition({ lineNumber: 1, column: 2 });
		editor.focus();
	});
	await page.keyboard.type('"name":"keyboard",');
	assert.equal(
		await page.evaluate(() => window.monaco.editor.getEditors()[0].getValue()),
		'{"name":"keyboard","components":[],"widgetRefs":{}}',
		"Monaco accepts typing",
	);
	await clickButton("Format");
	await page.waitForFunction(() =>
		window.monaco.editor.getEditors()[0].getValue().includes('\n  "name"'),
	);
	await clickButton("Copy");
	await button("Copied");
	assert.equal(
		await page.evaluate(() => navigator.clipboard.readText()),
		await page.evaluate(() => window.monaco.editor.getEditors()[0].getValue()),
	);
	passed.push("Typing, Format, and Copy remain functional");

	const drag = await page.evaluate(() => {
		const editor = window.monaco.editor.getEditors()[0];
		const bounds = editor.getDomNode().getBoundingClientRect();
		const start = editor.getScrolledVisiblePosition({
			lineNumber: 2,
			column: 3,
		});
		const end = editor.getScrolledVisiblePosition({ lineNumber: 2, column: 9 });
		return {
			x1: bounds.x + start.left,
			x2: bounds.x + end.left,
			y: bounds.y + start.top + start.height / 2,
		};
	});
	await page.mouse.move(drag.x1, drag.y);
	await page.mouse.down();
	await page.mouse.move(drag.x2, drag.y, { steps: 8 });
	await page.mouse.up();
	assert.ok(
		await page.evaluate(() => {
			const editor = window.monaco.editor.getEditors()[0];
			return editor
				.getModel()
				.getValueInRange(editor.getSelection())
				.includes("name");
		}),
		"Monaco text remains selectable by dragging",
	);
	await assertBlocked();
	passed.push("Monaco drag selection remains functional");

	await page.mouse.move(4, 30);
	await page.mouse.down();
	await page.mouse.move(4, 700, { steps: 12 });
	await page.mouse.up();
	await page.mouse.click(4, 30, { clickCount: 2 });
	await assertBlocked();
	assert.ok(
		await page.$('[role="dialog"]'),
		"Backdrop preserves the open editor",
	);
	passed.push("Backdrop clicks and dragging cannot select the underlying page");
	if (process.env.SCREENSHOT_OUTPUT) {
		await page.screenshot({ path: process.env.SCREENSHOT_OUTPUT });
	}

	const expectApplied = async (count, name) => {
		await page.waitForSelector('[role="dialog"]', { hidden: true });
		await page.waitForSelector('[data-sonner-toast][data-visible="true"]', {
			visible: true,
		});
		assert.equal(
			await page.$eval("[data-sonner-toast] [data-title]", (node) =>
				node.textContent.trim(),
			),
			"Changes applied",
			"Successful apply provides visible confirmation outside the closed dialog",
		);
		assert.equal(
			await page.evaluate(() => window.rawJsonQa.saved.length),
			count,
		);
		assert.equal(
			JSON.parse(
				await page.evaluate(
					(index) => window.rawJsonQa.saved[index],
					count - 1,
				),
			).name,
			name,
		);
		await page.waitForFunction(
			() =>
				document.activeElement ===
				document.querySelector(".fixture-toolbar button"),
			{ timeout: 5000 },
		);
	};

	// Monaco debounces word highlighting; let the pending request finish before disposal.
	await page.evaluate(
		() => new Promise((resolve) => setTimeout(resolve, 1000)),
	);
	await clickButton("Apply Changes");
	await expectApplied(1, "keyboard");
	passed.push(
		"Successful apply closes the dialog, shows confirmation, and restores focus",
	);

	await clickButton("Open Raw JSON");
	await page.waitForSelector('[role="dialog"]');
	await page.waitForFunction(
		() => window.monaco.editor.getEditors().length === 1,
	);
	await button("Apply Changes");
	assert.equal(
		await page.evaluate(
			() => JSON.parse(window.monaco.editor.getEditors()[0].getValue()).name,
		),
		"keyboard",
		"Reopening loads the successfully applied JSON",
	);
	assert.equal(
		await page.evaluate(() =>
			[...document.querySelectorAll('[role="dialog"] button')].some(
				(node) => node.textContent.trim() === "Saved",
			),
		),
		false,
		"Reopening does not retain the previous Saved state",
	);
	const malformedJson = '{"name":"recovered","components":[],"widgetRefs":{}';
	await page.evaluate((value) => {
		window.monaco.editor.getEditors()[0].setValue(value);
	}, malformedJson);
	await clickButton("Apply Changes");
	await page.waitForSelector('[role="dialog"] [role="alert"]', {
		visible: true,
	});
	assert.match(
		await page.$eval(
			'[role="dialog"] [role="alert"]',
			(node) => node.textContent,
		),
		/Could not apply changes/,
	);
	assert.equal(await page.evaluate(() => window.rawJsonQa.saved.length), 1);
	assert.equal(
		await page.evaluate(() => window.monaco.editor.getEditors()[0].getValue()),
		malformedJson,
		"Failed apply preserves the draft for correction",
	);
	await page.evaluate(() => {
		const editor = window.monaco.editor.getEditors()[0];
		const lastLine = editor.getModel().getLineCount();
		editor.setPosition({
			lineNumber: lastLine,
			column: editor.getModel().getLineMaxColumn(lastLine),
		});
		editor.focus();
	});
	await page.keyboard.type("}");
	await page.waitForSelector('[role="dialog"] [role="alert"]', {
		hidden: true,
	});
	passed.push(
		"Failed apply shows an alert, preserves the draft, and clears the error on edit",
	);
	await page.evaluate(
		() => new Promise((resolve) => setTimeout(resolve, 1000)),
	);
	await clickButton("Apply Changes");
	await expectApplied(2, "recovered");
	passed.push("Correcting invalid JSON allows a successful apply");

	await clickButton("Open Raw JSON");
	await page.waitForSelector('[role="dialog"]');
	await page.waitForFunction(
		() => window.monaco.editor.getEditors().length === 1,
	);
	await page.setViewport({ width: 1280, height: 420 });
	await page.waitForFunction(() => {
		const editor = document
			.querySelector(".monaco-editor")
			.getBoundingClientRect();
		const footer = document
			.querySelector('[data-slot="dialog-description"]')
			.getBoundingClientRect();
		return (
			editor.height > 100 &&
			editor.bottom <= footer.top + 1 &&
			footer.bottom <= innerHeight
		);
	});
	passed.push("Editor and footer fit a short viewport");
	await page.keyboard.press("Escape");
	assert.ok(
		await page.$('[role="dialog"]'),
		"Escape preserves the open editor",
	);
	// Monaco debounces word highlighting; let the pending request finish before disposal.
	await page.evaluate(
		() => new Promise((resolve) => setTimeout(resolve, 1000)),
	);
	await page.click('[role="dialog"] button[aria-label="Close"]');
	await page.waitForSelector('[role="dialog"]', { hidden: true });
	await page.waitForFunction(
		() =>
			document.activeElement ===
			document.querySelector(".fixture-toolbar button"),
		{ timeout: 5000 },
	);
	passed.push("Close, reopen, restored trigger focus, and Escape behavior");

	await page.setViewport({ width: 1280, height: 800 });
	const devModeButton = 'button[aria-label="Dev Mode (JSON Editor)"]';
	const closeDevMode = async () => {
		await page.waitForFunction(
			() => window.monaco.editor.getEditors().length === 1,
		);
		// Let Monaco's pending word highlighting finish before disposal.
		await page.evaluate(
			() => new Promise((resolve) => setTimeout(resolve, 1000)),
		);
		await page.click('[role="dialog"] button[aria-label="Close"]');
		await page.waitForSelector('[role="dialog"]', { hidden: true });
	};
	// Hold the entry animation where the tooltip overlaps the button's lower edge.
	// This makes the first-click regression independent of browser frame timing.
	const pausedTooltip = await page.addStyleTag({
		content:
			'[data-slot="tooltip-content"][data-state$="open"] { animation-play-state: paused !important; }',
	});
	for (let attempt = 0; attempt < 3; attempt++) {
		await page.mouse.move(900, 400);
		await page.hover(devModeButton);
		await page.waitForSelector('[data-slot="tooltip-content"]', {
			visible: true,
		});
		const trigger = await page.$(devModeButton);
		const bounds = await trigger.boundingBox();
		assert.ok(bounds);
		const point = {
			x: bounds.x + bounds.width / 2,
			y: bounds.y + bounds.height - 2,
		};
		await page.waitForFunction(
			({ x, y }) =>
				[
					...document.querySelectorAll(
						'[data-slot="tooltip-content"], [data-slot="tooltip-content"] *',
					),
				].some((node) => {
					const rect = node.getBoundingClientRect();
					return (
						rect.left <= x && x < rect.right && rect.top <= y && y < rect.bottom
					);
				}),
			{},
			point,
		);
		await page.mouse.click(point.x, point.y);
		await page.waitForSelector('[role="dialog"]', {
			visible: true,
			timeout: 2000,
		});
		await closeDevMode();
		await trigger.dispose();
	}
	await pausedTooltip.evaluate((node) => node.remove());
	passed.push(
		"Every first click opens Dev Mode while its tooltip overlaps the button",
	);

	for (const key of ["Enter", "Space"]) {
		await page.focus(devModeButton);
		await page.keyboard.press(key);
		await page.waitForSelector('[role="dialog"]', { visible: true });
		await closeDevMode();
	}
	passed.push("Enter and Space still open Dev Mode");
	assert.deepEqual(errors, [], "No browser exceptions");
	console.log(JSON.stringify({ passed, pageErrors: errors }, null, 2));
} finally {
	await browser.close();
}
