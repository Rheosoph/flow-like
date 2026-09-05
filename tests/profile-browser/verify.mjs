import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join } from "node:path";
import puppeteer from "puppeteer";

const require = createRequire(import.meta.url);

const output =
	process.env.PROFILE_QA_OUTPUT ||
	join(tmpdir(), "flow-like-profile-verification");
await mkdir(output, { recursive: true });
const browser = await puppeteer.launch({
	headless: true,
	executablePath:
		process.env.CHROME_PATH ||
		"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
	args: ["--no-sandbox", "--disable-gpu"],
	protocolTimeout: 30000,
	userDataDir: `${output}/browser-data-${process.pid}`,
});
const page = await browser.newPage();
const errors = [];
const accessibility = [];
const auditAccessibility = async (view, selector = "main") => {
	await page.addScriptTag({ path: require.resolve("axe-core/axe.min.js") });
	const result = await page.evaluate(
		async (selector) =>
			window.axe.run(document.querySelector(selector), {
				runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag21aa"] },
			}),
		selector,
	);
	const violations = result.violations.map(({ id, impact, nodes }) => ({
		id,
		impact,
		targets: nodes.map((node) => node.target),
	}));
	accessibility.push({ view, violations });
	assert.deepEqual(violations, [], `${view} accessibility violations`);
};
page.on("pageerror", (error) => errors.push(error.message));
await page.setRequestInterception(true);
page.on("request", (request) => {
	if (/^(http:\/\/127\.0\.0\.1:4323\/|data:|blob:)/.test(request.url()))
		request.continue();
	else request.abort();
});
try {
	await page.setViewport({ width: 1440, height: 1000 });
	await page.goto("http://127.0.0.1:4323/?view=account", {
		waitUntil: "networkidle0",
	});
	await page.waitForSelector("#account-name");
	await page.screenshot({
		path: `${output}/01-account-desktop.png`,
		fullPage: true,
	});
	const checks = [];
	const text = () => page.$eval("body", (element) => element.innerText);
	const click = async (label) => {
		const handle = await page.evaluateHandle(
			(text) =>
				[...document.querySelectorAll("button,a")].find(
					(el) => el.textContent.trim().toLowerCase() === text.toLowerCase(),
				),
			label,
		);
		assert.ok(handle.asElement(), `Missing action: ${label}`);
		await handle.asElement().click();
	};
	const fill = async (selector, value) => {
		await page.click(selector, { clickCount: 3 });
		await page.keyboard.press("Backspace");
		await page.keyboard.type(value);
	};
	const check = (name, result) => {
		assert.ok(result, name);
		checks.push(name);
		console.log(`PASS ${name}`);
	};
	check(
		"untouched profile save is disabled",
		await page.$eval('button[type="submit"]', (el) => el.disabled),
	);
	await fill("#account-name", "Alex Updated");
	await click("Save Changes");
	await page.waitForFunction(
		() => window.profileQa.state.user.name === "Alex Updated",
	);
	await page.waitForFunction(
		() => document.querySelector('button[type="submit"]').disabled,
	);
	check("display name saved", true);
	await page.evaluate(() => {
		window.profileQa.state.failSave = true;
	});
	await fill("#account-name", "Draft preserved");
	await click("Save Changes");
	await page.waitForSelector('[role="alert"]');
	check(
		"failed save keeps name draft",
		await page.$eval("#account-name", (el) => el.value === "Draft preserved"),
	);
	await page.evaluate(() => {
		window.profileQa.state.failSave = false;
	});
	await click("Save Changes");
	await page.waitForFunction(
		() => window.profileQa.state.user.name === "Draft preserved",
	);
	check("failed save can retry", true);
	await fill("#account-name", "Unsaved while email changes");
	const photoInput = await page.$('input[type="file"]');
	await photoInput.uploadFile(`${output}/01-account-desktop.png`);
	await page.waitForFunction(() =>
		window.profileQa.state.user.avatar?.startsWith("blob:"),
	);
	await page.waitForFunction(
		() => !document.querySelector('button[type="submit"]').disabled,
	);
	check(
		"photo refresh retains unsaved name",
		await page.$eval(
			"#account-name",
			(el) => el.value === "Unsaved while email changes",
		),
	);
	await click("Change email");
	await page.waitForSelector("#account-new-email");
	await page.evaluate(() => {
		window.profileQa.state.failEmail = true;
	});
	await fill("#account-new-email", "updated@example.invalid");
	await page.focus("#account-new-email");
	await page.keyboard.press("Enter");
	await page.waitForFunction(() =>
		document
			.querySelector('[role="dialog"]')
			?.innerText.includes("Check your connection"),
	);
	check("email request failure is visible", true);
	await page.evaluate(() => {
		window.profileQa.state.failEmail = false;
	});
	await page.focus("#account-new-email");
	await page.keyboard.press("Enter");
	await page.waitForSelector("#account-email-code");
	await page.screenshot({ path: `${output}/02-email-verification.png` });
	await auditAccessibility("email verification", '[role="dialog"]');
	await fill("#account-email-code", "123456");
	await page.keyboard.press("Enter");
	await page.waitForFunction(() => !document.querySelector('[role="dialog"]'));
	check("email completion closes dialog", true);
	check(
		"email refresh retains unsaved name",
		await page.$eval(
			"#account-name",
			(el) => el.value === "Unsaved while email changes",
		),
	);
	await click("Change Password");
	await page.waitForSelector("#account-password-current");
	await fill("#account-password-current", "CurrentExample!23");
	await fill("#account-password-next", "NextExample!23");
	await fill("#account-password-confirm", "NextExample!23");
	await page.evaluate(() => {
		window.profileQa.state.failPassword = true;
	});
	await page.keyboard.press("Enter");
	await page.waitForFunction(() =>
		document
			.querySelector('[role="dialog"]')
			?.innerText.includes("password requirements"),
	);
	check("password policy failure is explained", true);
	check(
		"password visibility controls are named",
		await page.$$eval('[role="dialog"] button', (els) =>
			els.every((el) =>
				Boolean(el.textContent.trim() || el.getAttribute("aria-label")),
			),
		),
	);
	await page.screenshot({ path: `${output}/03-password-feedback.png` });
	await auditAccessibility("password error", '[role="dialog"]');
	await page.evaluate(() => {
		window.profileQa.state.failPassword = false;
	});
	await page.focus("#account-password-confirm");
	await page.keyboard.press("Enter");
	await page.waitForFunction(() => !document.querySelector('[role="dialog"]'));
	check(
		"password callback completes and closes",
		await page.evaluate(() => window.profileQa.state.passwordWrites === 2),
	);
	const savedBeforePending = await page.evaluate(() => {
		window.profileQa.state.holdSave = true;
		return window.profileQa.state.profileWrites.length;
	});
	await click("Save Changes");
	await page.waitForFunction(() => Boolean(window.profileQa.state.releaseSave));
	check(
		"pending save disables duplicate submission",
		await page.$eval('button[type="submit"]', (el) => el.disabled),
	);
	await page.evaluate(() => {
		window.profileQa.state.holdSave = false;
		window.profileQa.state.releaseSave();
	});
	await page.waitForFunction(
		() => window.profileQa.state.user.name === "Unsaved while email changes",
	);
	check(
		"pending save submits once",
		(await page.evaluate(() => window.profileQa.state.profileWrites.length)) ===
			savedBeforePending + 1,
	);
	await page.goto("http://127.0.0.1:4323/?view=account&managed", {
		waitUntil: "networkidle0",
	});
	check(
		"provider-managed username is read-only",
		await page.$eval("#account-username", (el) => el.readOnly || el.disabled),
	);
	check(
		"provider-managed account omits password action",
		!(await text()).includes("Change Password"),
	);
	await page.goto("http://127.0.0.1:4323/?sub=review-user", {
		waitUntil: "networkidle0",
	});
	check(
		"own public profile has edit action",
		(await text()).toLowerCase().includes("edit profile"),
	);
	await page.screenshot({
		path: `${output}/04-public-profile-desktop.png`,
		fullPage: true,
	});
	await page.goto("http://127.0.0.1:4323/?sub=review-user&visitor", {
		waitUntil: "networkidle0",
	});
	check(
		"contact action works for visible email",
		await page.$eval(
			'a[href^="mailto:"]',
			(el) =>
				decodeURIComponent(el.getAttribute("href")) ===
				"mailto:alex@example.invalid",
		),
	);
	check(
		"visitor cannot edit another account",
		!(await text()).toLowerCase().includes("edit profile"),
	);
	await page.goto("http://127.0.0.1:4323/?sub=review-user&apps-error", {
		waitUntil: "networkidle0",
	});
	check(
		"failed apps request is not shown as empty profile",
		!(await text()).includes("No published apps yet") &&
			(await text()).includes("Apps could not be loaded"),
	);
	await page.evaluate(() => {
		window.profileQa.state.failApps = false;
	});
	await click("Retry");
	await page.waitForFunction(() =>
		document.body.innerText.includes("Knowledge Chat"),
	);
	check("apps error retries", true);
	await page.goto("http://127.0.0.1:4323/?view=workspace", {
		waitUntil: "networkidle0",
	});
	await page.waitForSelector("#workspace-profile-name");
	await fill("#workspace-profile-name", "Updated workspace");
	await page.waitForFunction(
		() =>
			window.workspaceQa.saves.at(-1)?.hub_profile.name === "Updated workspace",
	);
	check("workspace autosave saves latest value", true);
	await page.evaluate(() => {
		window.workspaceQa.failSave = true;
	});
	await fill("#workspace-profile-name", "Retained workspace draft");
	await page.waitForFunction(() =>
		document.body.innerText.includes("Changes not saved"),
	);
	check(
		"workspace failure retains draft",
		await page.$eval(
			"#workspace-profile-name",
			(el) => el.value === "Retained workspace draft",
		),
	);
	await page.evaluate(() => {
		window.workspaceQa.failSave = false;
	});
	await click("Retry save");
	await page.waitForFunction(
		() =>
			window.workspaceQa.saves.at(-1)?.hub_profile.name ===
			"Retained workspace draft",
	);
	check("workspace retry saves retained value", true);
	await click("Change image");
	await page.waitForFunction(() =>
		document.body.innerText.includes("Your previous image is still in use"),
	);
	check("workspace image failure is visible", true);
	await page.screenshot({
		path: `${output}/06-workspace-desktop.png`,
		fullPage: true,
	});
	await click("Remove from this device");
	await page.waitForSelector('[role="alertdialog"]');
	await auditAccessibility("workspace removal", '[role="alertdialog"]');
	check(
		"local removal explains cloud copy",
		(await text()).includes("Cloud copies remain"),
	);
	await page.keyboard.press("Escape");
	await page.waitForFunction(
		() => !document.querySelector('[role="alertdialog"]'),
	);
	check("deletion dialog supports Escape", true);
	await page.goto("http://127.0.0.1:4323/?view=workspace&web", {
		waitUntil: "networkidle0",
	});
	check(
		"browser omits unsupported GPU controls",
		!(await page.$("#workspace-gpu")) &&
			!(await page.$("#workspace-context-size")),
	);
	for (const view of ["account", "public", "workspace"]) {
		await page.setViewport({ width: 390, height: 844 });
		await page.goto(
			`http://127.0.0.1:4323/?${view === "public" ? "sub=review-user" : `view=${view}`}`,
			{ waitUntil: "networkidle0" },
		);
		check(
			`${view} fits mobile viewport`,
			await page.evaluate(
				() => document.documentElement.scrollWidth <= innerWidth,
			),
		);
		await page.screenshot({
			path: `${output}/05-${view}-mobile.png`,
			fullPage: true,
		});
		await auditAccessibility(`${view} dark`);
		await page.goto(`${page.url()}&light`, { waitUntil: "networkidle0" });
		await auditAccessibility(`${view} light`);
		await page.screenshot({
			path: `${output}/07-${view}-light-mobile.png`,
			fullPage: true,
		});
	}
	assert.deepEqual(errors, []);
	await writeFile(
		`${output}/results.json`,
		JSON.stringify({ checks, errors, accessibility }, null, 2),
	);
	console.log(
		JSON.stringify({ checks: checks.length, errors, accessibility }, null, 2),
	);
} catch (error) {
	await page.screenshot({ path: `${output}/failure.png`, fullPage: true });
	console.log(await page.$eval("body", (el) => el.innerText));
	console.log(
		await page.$$eval("input,textarea", (els) =>
			els.map((el) => ({
				id: el.id,
				value: el.type === "password" ? "[test password]" : el.value,
			})),
		),
	);
	throw error;
} finally {
	const stop = setTimeout(() => browser.process()?.kill("SIGKILL"), 5000);
	await browser.close().finally(() => clearTimeout(stop));
}
