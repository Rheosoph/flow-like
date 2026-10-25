import assert from "node:assert/strict";
import puppeteer from "puppeteer";

// With the preview running: node tests/onboarding-browser/verify.mjs

const browser = await puppeteer.launch({
	headless: true,
	executablePath:
		process.env.CHROME_EXECUTABLE_PATH ||
		(process.platform === "darwin"
			? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
			: undefined),
	args: ["--no-sandbox"],
});
const page = await browser.newPage();
const errors = [];
const passed = [];
page.on("pageerror", (error) => errors.push(error.message));
await page.setViewport({ width: 1280, height: 800, deviceScaleFactor: 1 });
await page.setRequestInterception(true);
page.on("request", (request) => {
	const url = new URL(request.url());
	if (
		["127.0.0.1", "localhost"].includes(url.hostname) ||
		["data:", "blob:"].includes(url.protocol)
	)
		void request.continue();
	else void request.abort();
});
const load = async (query = "") => {
	await page.goto(`http://127.0.0.1:4346/onboarding${query}`, {
		waitUntil: "networkidle0",
	});
};
const text = () => page.evaluate(() => document.body.innerText);
const waitText = (value) =>
	page.waitForFunction(
		(needle) => document.body.innerText.includes(needle),
		{},
		value,
	);
const clickButton = async (label) => {
	const buttons = await page.$$("button");
	for (const button of buttons) {
		if (
			(await button.evaluate((element) => element.innerText)).trim() === label
		) {
			await button.click();
			return;
		}
	}
	throw new Error(`Button not found: ${label}`);
};
const card = async (index) => (await page.$$("button[aria-pressed]"))[index];
const selected = () =>
	page.$$eval('button[aria-pressed="true"]', (elements) => elements.length);
const href = () =>
	page.$eval('a[href^="/onboarding/download"]', (element) =>
		element.getAttribute("href"),
	);
try {
	await load();
	await page.waitForSelector("button[aria-pressed]");
	assert.equal(
		await page.$eval("h1", (heading) => heading.textContent),
		"Make your ideas flow.",
	);
	assert.equal(
		await page.$eval("h1 span", (accent) => accent.textContent),
		"ideas flow.",
	);
	assert.equal(await selected(), 0);
	assert.equal(await page.$('a[href^="/onboarding/download"]'), null);
	await page.evaluate(async () => {
		await Promise.all(
			document
				.getAnimations()
				.filter(
					(animation) =>
						animation.effect?.getTiming().iterations !==
						Number.POSITIVE_INFINITY,
				)
				.map((animation) => animation.finished.catch(() => {})),
		);
	});
	await page.click('button[aria-label="Replay welcome animation"]');
	await page.waitForFunction(() =>
		document
			.getAnimations()
			.some((animation) => animation.playState === "running"),
	);
	assert.equal(await selected(), 0);
	passed.push(
		"Welcome animation can be replayed without changing the selection",
	);
	await (await card(0)).click();
	await (await card(1)).click();
	assert.equal(await selected(), 2);
	assert.ok((await text()).includes("2 profiles selected"));
	assert.deepEqual(
		new URL(await href(), "http://fixture").searchParams.getAll("profiles"),
		["starter-chatting", "starter-gaming"],
	);
	await (await card(0)).click();
	assert.equal(await selected(), 1);
	assert.ok((await text()).includes("1 profile selected"));
	await (await card(1)).click();
	assert.equal(await selected(), 0);
	assert.equal(await page.$('a[href^="/onboarding/download"]'), null);
	await (await card(0)).focus();
	await page.keyboard.press("Space");
	assert.equal(await selected(), 1);
	passed.push(
		"Profile multi-select, deselect, disabled continuation, keyboard selection",
	);
	await clickButton("Sign in");
	assert.equal(await page.evaluate(() => window.onboardingQa.signIns), 1);
	await waitText("Finish signing in in your browser");
	await page.evaluate(() => {
		window.onboardingQa.failSignIn = true;
	});
	await clickButton("Open sign-in again");
	await waitText("Sign-in could not open");
	passed.push("Header sign-in and recoverable sign-in failure");

	await load("?encoded");
	await page.waitForSelector("button[aria-pressed]");
	await (await card(0)).click();
	assert.equal(
		new URL(await href(), "http://fixture").searchParams.get("profiles"),
		"starter + chat&one",
	);
	passed.push("Profile IDs are encoded in continuation URLs");

	await load();
	await page.waitForSelector("button[aria-pressed]");
	await page.evaluate(() => {
		window.onboardingQa.holdSizes = true;
	});
	await (await card(0)).click();
	await page.waitForFunction(
		() => window.onboardingQa.sizeRequests.length === 1,
	);
	await (await card(1)).click();
	await page.waitForFunction(
		() => window.onboardingQa.sizeRequests.length === 3,
	);
	await page.evaluate(() => {
		window.onboardingQa.sizeRequests[1].resolve(100);
		window.onboardingQa.sizeRequests[2].resolve(200);
	});
	await waitText("300 B to download");
	await page.evaluate(() => {
		window.onboardingQa.sizeRequests[0].resolve(9999);
	});
	await page.evaluate(
		() =>
			new Promise((resolve) =>
				requestAnimationFrame(() => requestAnimationFrame(resolve)),
			),
	);
	assert.ok((await text()).includes("300 B to download"));
	passed.push("Late size results cannot overwrite the current selection total");

	await load("?loading");
	await waitText("Finding your starting profiles");
	assert.equal(await page.$('a[href^="/onboarding/download"]'), null);
	await page.evaluate(() => window.onboardingQa.releaseProfiles());
	await page.waitForSelector("button[aria-pressed]");
	await load("?error");
	await waitText("We couldn't load the starting profiles");
	await page.evaluate(() => {
		window.onboardingQa.failProfiles = false;
	});
	await clickButton("Try again");
	await page.waitForSelector("button[aria-pressed]");
	await load("?empty");
	await waitText("No compatible profiles are available");
	assert.equal(await page.$('a[href^="/onboarding/download"]'), null);
	passed.push("Catalog loading, failed fetch retry, and empty states");

	await load("?authenticated&restore-error");
	await waitText("We couldn't restore your saved profiles");
	await page.evaluate(() => {
		window.onboardingQa.failRestore = false;
		window.onboardingQa.serverProfiles = [
			{
				id: "saved-profile",
				name: "Saved",
				hub: "fixture.invalid",
				created_at: "2026-01-01",
				updated_at: "2026-01-01",
			},
		];
	});
	await clickButton("Try again");
	await page.waitForFunction(() =>
		window.onboardingQa.commands.some(
			(entry) => entry.command === "set_current_profile",
		),
	);
	assert.equal(await page.evaluate(() => location.pathname), "/");
	passed.push(
		"Authenticated restore failure retries, saves profile, and opens home",
	);

	await load("?authenticated&hold-restore");
	await page.waitForFunction(() => Boolean(window.onboardingQa.releaseRestore));
	await page.evaluate(() => {
		window.onboardingQa.serverProfiles = [
			{ id: "cancelled", name: "Cancelled", hub: "fixture.invalid" },
		];
		window.onboardingQa.unmount();
	});
	await waitText("Onboarding unmounted");
	await page.evaluate(() => window.onboardingQa.releaseRestore());
	await page.evaluate(
		() =>
			new Promise((resolve) =>
				requestAnimationFrame(() => requestAnimationFrame(resolve)),
			),
	);
	assert.equal(
		await page.evaluate(
			() =>
				window.onboardingQa.commands.filter(
					(entry) => entry.command === "upsert_profile",
				).length,
		),
		0,
	);
	passed.push("Leaving during server fetch cancels profile restoration");

	await load("?authenticated&hold-restore");
	await page.waitForFunction(() => Boolean(window.onboardingQa.releaseRestore));
	await page.evaluate(() => {
		window.onboardingQa.serverProfiles = ["first", "second"].map((id) => ({
			id,
			name: id,
			hub: "fixture.invalid",
		}));
		window.onboardingQa.holdUpsert = true;
		window.onboardingQa.releaseRestore();
	});
	await page.waitForFunction(() => Boolean(window.onboardingQa.releaseUpsert));
	await page.evaluate(() => window.onboardingQa.unmount());
	await waitText("Onboarding unmounted");
	await page.evaluate(() => window.onboardingQa.releaseUpsert());
	await page.evaluate(
		() =>
			new Promise((resolve) =>
				requestAnimationFrame(() => requestAnimationFrame(resolve)),
			),
	);
	assert.deepEqual(
		await page.evaluate(() =>
			window.onboardingQa.commands
				.filter((entry) => entry.command !== "get_default_profiles")
				.map((entry) => entry.command),
		),
		["upsert_profile"],
	);
	assert.equal(await page.evaluate(() => location.pathname), "/onboarding");
	passed.push(
		"Leaving during native profile save prevents later saves and navigation",
	);

	await load("?authenticated&hold-restore");
	await waitText("Restoring your profiles");
	await page.evaluate(() => window.onboardingQa.setSignedIn(false));
	await page.waitForSelector("button[aria-pressed]");
	assert.ok(!(await text()).includes("Restoring your profiles"));
	await page.evaluate(() => window.onboardingQa.releaseRestore());
	assert.equal(await page.evaluate(() => location.pathname), "/onboarding");
	passed.push(
		"Losing authentication during restore returns to the profile picker",
	);

	await page.emulateMediaFeatures([
		{ name: "prefers-reduced-motion", value: "reduce" },
	]);
	await load();
	await page.waitForSelector("button[aria-pressed]");
	await page.click('button[aria-label="Replay welcome animation"]');
	await page.evaluate(
		() =>
			new Promise((resolve) =>
				requestAnimationFrame(() => requestAnimationFrame(resolve)),
			),
	);
	assert.equal(
		await page.evaluate(
			() =>
				document
					.getAnimations()
					.filter((animation) => animation.playState === "running").length,
		),
		0,
	);
	await page.setViewport({ width: 390, height: 844 });
	await load();
	await page.waitForSelector("button[aria-pressed]");
	await (await card(3)).click();
	assert.equal(await selected(), 1);
	assert.ok(
		await page.evaluate(
			() => document.documentElement.scrollWidth <= innerWidth,
		),
	);
	assert.ok(
		await page.$eval(
			"footer",
			(footer) => footer.getBoundingClientRect().bottom <= innerHeight + 1,
		),
	);
	passed.push(
		"Reduced motion, narrow layout, reachable cards, and visible continuation",
	);
	assert.equal(
		await page.$eval(
			"main",
			(main) => getComputedStyle(main.parentElement).paddingLeft,
		),
		"0px",
	);
	for (const pathname of ["/onboarding/download", "/onboarding/done"]) {
		await page.evaluate((path) => {
			history.pushState({}, "", path);
			window.dispatchEvent(new PopStateEvent("popstate"));
		}, pathname);
		await page.waitForFunction(() => {
			const shell = document.querySelector("main").parentElement;
			const style = getComputedStyle(shell);
			return (
				style.paddingLeft === "16px" &&
				style.paddingRight === "16px" &&
				style.alignItems === "center" &&
				shell.querySelector(':scope > div[aria-hidden="true"]') !== null
			);
		});
	}
	passed.push("Setup routes preserve their centered shell and mobile gutters");
	assert.deepEqual(errors, []);
	console.log(JSON.stringify({ passed, pageErrors: errors }, null, 2));
} finally {
	await browser.close();
}
