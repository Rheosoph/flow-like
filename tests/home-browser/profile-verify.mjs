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
page.setDefaultTimeout(15_000);
const report = { passed: [], errors: [], blocked: [], screenshots: [] };
const origin = process.env.PROFILE_FIXTURE_URL || "http://127.0.0.1:4318";
await page.route("**/*", async (route) => {
	const url = new URL(route.request().url());
	if (url.pathname.startsWith("/profile-fixture-media/")) {
		const base64 = await page.evaluate(
			(path) => window.profileQa.state.media[path],
			url.pathname,
		);
		return route.fulfill({
			status: base64 ? 200 : 404,
			contentType: url.pathname.endsWith(".png") ? "image/png" : "image/webp",
			body: base64 ? Buffer.from(base64, "base64") : "",
		});
	}
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
const snap = async (name, fullPage = false) => {
	await page.waitForFunction(
		() => !document.querySelector('[data-sonner-toast][data-visible="true"]'),
	);
	const path = `/private/tmp/profile-qa-${name}.png`;
	await page.screenshot({ path, fullPage });
	report.screenshots.push(path);
};
const saved = () => page.evaluate(() => window.profileQa.getSaved());
const writes = () => page.evaluate(() => window.profileQa.state.writes.length);
const group = (name) => page.getByRole("group", { name, exact: true });
const tab = (name) =>
	page.getByRole("tab", { name: new RegExp(`^${name}`) }).click();
const checkWidth = async () =>
	assert.ok(
		await page.evaluate(
			() => document.documentElement.scrollWidth <= innerWidth + 1,
		),
		"No horizontal page overflow",
	);
const makeImage = async (width, height, mime = "image/png") =>
	Buffer.from(
		await page.evaluate(
			({ width, height, mime }) => {
				const canvas = document.createElement("canvas");
				canvas.width = width;
				canvas.height = height;
				const ctx = canvas.getContext("2d");
				const gradient = ctx.createLinearGradient(0, 0, width, height);
				gradient.addColorStop(0, "#6533b4");
				gradient.addColorStop(0.55, "#185a7b");
				gradient.addColorStop(1, "#f5a46e");
				ctx.fillStyle = gradient;
				ctx.fillRect(0, 0, width, height);
				ctx.fillStyle = "#ffffffcc";
				ctx.font = `600 ${Math.round(width / 12)}px sans-serif`;
				ctx.fillText("Research", width / 12, height / 2);
				return canvas.toDataURL(mime).split(",")[1];
			},
			{ width, height, mime },
		),
		"base64",
	);
try {
	await page.goto(`${origin}/admin/profiles`, {
		waitUntil: "domcontentloaded",
		timeout: 60_000,
	});
	await page
		.getByRole("heading", { name: "Starter profiles", exact: true })
		.waitFor({ timeout: 60_000 });
	assert.equal(await page.locator("article").count(), 2);
	await page.getByLabel("Search profiles").fill("Research");
	assert.equal(await page.locator("article").count(), 1);
	await page.getByLabel("Search profiles").clear();
	await page.getByRole("link", { name: "Create profile", exact: true }).click();
	await page
		.getByLabel("Profile name", { exact: true })
		.fill("Research studio");
	await page
		.getByLabel("Description", { exact: true })
		.fill(
			"A thoughtful home for exploring sources and turning useful findings into shared knowledge.\nStart with selected models, apps, and a personal workspace.",
		);
	await page.getByLabel("Tags", { exact: true }).fill("Research");
	await page.getByRole("button", { name: "Add tags", exact: true }).click();
	await page.getByLabel("Interests", { exact: true }).fill("Data analysis");
	await page.getByLabel("Interests", { exact: true }).press("Enter");
	const png = await makeImage(1200, 800);
	await page.evaluate(() => {
		window.profileQa.holdUpload = true;
	});
	await page
		.getByLabel("Choose Profile icon file", { exact: true })
		.setInputFiles({
			name: "research.png",
			mimeType: "image/png",
			buffer: png,
		});
	await page.waitForFunction(
		() => window.profileQa.uploadStarted && window.profileQa.releaseUpload,
	);
	assert.equal(
		await page
			.getByRole("button", { name: "Create profile", exact: true })
			.isDisabled(),
		true,
	);
	await tab("Bits");
	await page.evaluate(() => {
		window.profileQa.holdUpload = false;
		window.profileQa.releaseUpload();
	});
	await page.waitForFunction(() => window.profileQa.state.uploads.length === 1);
	await tab("Identity");
	await page.waitForFunction(
		() =>
			document.querySelector('input[aria-label="Choose Profile icon file"]')
				?.disabled === false,
	);
	assert.match(
		await page.getByLabel("Profile icon URL", { exact: true }).inputValue(),
		/profile-fixture-media\/1.webp$/,
	);
	const upload = await page.evaluate(() => window.profileQa.state.uploads[0]);
	assert.deepEqual(
		[upload.type, upload.width, upload.height, upload.signature],
		["image/webp", 512, 341, "RIFF"],
	);
	const jpeg = await makeImage(2400, 900, "image/jpeg");
	const drag = await page.evaluateHandle((bytes) => {
		const transfer = new DataTransfer();
		transfer.items.add(
			new File(
				[Uint8Array.from(atob(bytes), (character) => character.charCodeAt(0))],
				"cover.jpg",
				{ type: "image/jpeg" },
			),
		);
		return transfer;
	}, jpeg.toString("base64"));
	await page
		.getByRole("button", { name: "Upload Cover image", exact: true })
		.dispatchEvent("drop", { dataTransfer: drag });
	await drag.dispose();
	await page.waitForFunction(() => window.profileQa.state.uploads.length === 2);
	assert.deepEqual(
		await page
			.evaluate(() => window.profileQa.state.uploads[1])
			.then((value) => [value.type, value.width, value.height]),
		["image/webp", 1600, 600],
	);
	await tab("Bits");
	await page
		.getByRole("button", {
			name: "Include Research language model",
			exact: true,
		})
		.click();
	await page
		.getByRole("button", { name: "Include Document embeddings", exact: true })
		.click();
	await page.getByText("Add a custom bit reference", { exact: true }).click();
	await page
		.getByLabel("Bit reference", { exact: true })
		.fill("private.hub:custom-model");
	await page
		.getByRole("button", { name: "Add reference", exact: true })
		.click();
	await tab("Apps");
	await page
		.getByRole("button", { name: "Knowledge Chat", exact: true })
		.click();
	await page.getByRole("switch", { name: "Favorite", exact: true }).check();
	await page.getByRole("switch", { name: "Pin", exact: true }).check();
	await tab("Defaults");
	await page
		.getByLabel("Flow connection style", { exact: true })
		.selectOption("straight");
	await tab("Identity");
	await snap("editor-desktop", true);
	await page
		.getByRole("button", { name: "Create profile", exact: true })
		.click();
	await page.waitForURL(/\/admin\/profiles\/add\?id=/);
	const id = new URL(page.url()).searchParams.get("id");
	const created = (await saved()).find((value) => value.id === id);
	assert.equal(created.name, "Research studio");
	assert.equal(created.bits.length, 3);
	assert.deepEqual(created.apps, [
		{ app_id: "fixture-app-0", favorite: true, pinned: true },
	]);
	assert.equal(created.settings.connection_mode, "straight");
	assert.equal(await writes(), 1);
	await page.reload({ waitUntil: "domcontentloaded" });
	await page.getByLabel("Profile name", { exact: true }).waitFor();
	assert.equal(
		await page.getByLabel("Profile name", { exact: true }).inputValue(),
		"Research studio",
	);
	assert.equal(
		await page.getByLabel("Profile icon URL", { exact: true }).inputValue(),
		created.icon,
	);
	report.passed.push(
		"Create and reload preserve identity, converted WebP artwork, selected/catalog/custom bits, app favorites/pins, and connection style; icon upload survives tab change.",
	);
	await page
		.getByLabel("Profile name", { exact: true })
		.fill("Research studio updated");
	await page.evaluate(async () => {
		await window.profileQa.refetch();
	});
	assert.equal(
		await page.getByLabel("Profile name", { exact: true }).inputValue(),
		"Research studio updated",
	);
	await page.evaluate(() => {
		window.profileQa.failSave = true;
	});
	await page.getByRole("button", { name: "Save changes", exact: true }).click();
	await page
		.getByRole("alert")
		.filter({ hasText: "Fixture save failed" })
		.waitFor();
	assert.equal(await writes(), 1);
	assert.equal(
		await page.getByLabel("Profile name", { exact: true }).inputValue(),
		"Research studio updated",
	);
	await page.evaluate(() => {
		window.profileQa.failSave = false;
	});
	await page.getByRole("button", { name: "Save changes", exact: true }).click();
	await page.waitForFunction(() => window.profileQa.state.writes.length === 2);
	await page.evaluate(() => {
		window.profileQa.failUpload = true;
	});
	await page
		.getByLabel("Choose Profile icon file", { exact: true })
		.setInputFiles({
			name: "replacement.png",
			mimeType: "image/png",
			buffer: png,
		});
	await page
		.getByRole("alert")
		.filter({ hasText: "Fixture image upload failed" })
		.waitFor();
	assert.equal(
		await page.getByLabel("Profile icon URL", { exact: true }).inputValue(),
		created.icon,
	);
	await page.evaluate(() => {
		window.profileQa.failUpload = false;
	});
	await page
		.getByLabel("Choose Profile icon file", { exact: true })
		.setInputFiles({
			name: "fake.png",
			mimeType: "image/png",
			buffer: Buffer.from("<svg></svg>"),
		});
	await group("Profile icon").getByRole("alert").waitFor();
	assert.equal(
		await page.getByLabel("Profile icon URL", { exact: true }).inputValue(),
		created.icon,
	);
	await page
		.getByLabel("Profile icon URL", { exact: true })
		.fill("javascript:alert(1)");
	await group("Profile icon")
		.getByRole("button", { name: "Use URL", exact: true })
		.click();
	assert.equal(
		await page
			.getByLabel("Profile icon URL", { exact: true })
			.getAttribute("aria-invalid"),
		"true",
	);
	await page.getByLabel("Profile icon URL", { exact: true }).fill(created.icon);
	await page.getByLabel("Profile icon URL", { exact: true }).press("Enter");
	report.passed.push(
		"Background refetch and save/upload failures preserve drafts and existing images; spoofed image bytes and unsafe image URLs are rejected.",
	);
	for (const width of [390, 768, 1480]) {
		await page.setViewportSize({ width, height: width === 390 ? 844 : 1050 });
		await checkWidth();
		await page.evaluate(() => scrollTo(0, 0));
		await snap(`editor-${width}`);
		await tab("Bits");
		await checkWidth();
		await snap(`bits-${width}`);
		await tab("Identity");
	}
	await page
		.getByRole("button", { name: "Back to starter profiles", exact: true })
		.click();
	await page
		.getByRole("heading", { name: "Starter profiles", exact: true })
		.waitFor();
	for (const width of [390, 768, 1480]) {
		await page.setViewportSize({ width, height: width === 390 ? 844 : 1050 });
		await checkWidth();
		await snap(`list-${width}`, width === 1480);
	}
	await page
		.getByRole("link", {
			name: "Duplicate Research studio updated",
			exact: true,
		})
		.click();
	await page.getByLabel("Profile name", { exact: true }).waitFor();
	assert.equal(
		await page.getByLabel("Profile name", { exact: true }).inputValue(),
		"Research studio updated copy",
	);
	assert.equal(await writes(), 2);
	await page
		.getByRole("button", { name: "Create profile", exact: true })
		.click();
	await page.waitForFunction(() => window.profileQa.state.writes.length === 3);
	const copyId = new URL(page.url()).searchParams.get("id");
	assert.notEqual(copyId, id);
	await tab("Defaults");
	await page
		.getByRole("button", { name: "Edit default home", exact: true })
		.click();
	await page.getByTestId("home-destination").waitFor();
	assert.equal(new URL(page.url()).searchParams.get("default"), copyId);
	report.passed.push(
		"Duplicate creates no record until Save, then gets its own ID; per-profile default-home link carries that ID. Identity, bits and list views fit 390/768/1480 widths.",
	);
	await page.goto(`${origin}/admin/profiles`, {
		waitUntil: "domcontentloaded",
		timeout: 60_000,
	});
	await page
		.getByRole("heading", { name: "Starter profiles", exact: true })
		.waitFor();
	await page.evaluate(() => {
		window.profileQa.failDelete = true;
	});
	await page
		.getByRole("button", {
			name: "Delete Research studio updated copy",
			exact: true,
		})
		.click();
	const dialog = page.getByRole("alertdialog");
	await dialog
		.getByRole("button", { name: "Delete profile", exact: true })
		.click();
	await dialog
		.getByRole("alert")
		.filter({ hasText: "Fixture deletion failed" })
		.waitFor();
	assert.equal((await saved()).length, 4);
	await snap("delete-failure");
	await page.evaluate(() => {
		window.profileQa.failDelete = false;
	});
	await dialog
		.getByRole("button", { name: "Delete profile", exact: true })
		.click();
	await dialog.waitFor({ state: "hidden" });
	assert.equal((await saved()).length, 3);
	report.passed.push(
		"Deletion failure remains in the confirmation dialog and preserves the record; retry succeeds.",
	);
	assert.deepEqual(report.errors, []);
	assert.deepEqual(report.blocked, []);
} catch (error) {
	report.failure = error.stack;
	await page.screenshot({
		path: "/private/tmp/profile-qa-failure.png",
		fullPage: true,
	});
	throw error;
} finally {
	await writeFile(
		"/private/tmp/profile-qa-report.json",
		JSON.stringify(report, null, 2),
	);
	console.log(JSON.stringify(report, null, 2));
	await browser.close();
}
