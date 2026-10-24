import puppeteer from "puppeteer";

// With the preview running: node tests/onboarding-browser/capture.mjs

const browser = await puppeteer.launch({
	headless: true,
	executablePath:
		process.env.CHROME_EXECUTABLE_PATH ||
		(process.platform === "darwin"
			? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
			: undefined),
	args: ["--no-sandbox"],
});
try {
	const page = await browser.newPage();
	page.on("pageerror", (error) => console.error(error.message));
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
	const settle = () =>
		page.evaluate(async () => {
			await new Promise((resolve) =>
				requestAnimationFrame(() => requestAnimationFrame(resolve)),
			);
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
	for (const [width, height] of [
		[1280, 800],
		[1440, 1000],
		[390, 844],
	]) {
		await page.setViewport({ width, height, deviceScaleFactor: 1 });
		await page.goto("http://127.0.0.1:4346/", { waitUntil: "networkidle0" });
		await page.waitForSelector('button[aria-pressed="false"]');
		await settle();
		if (width === 1280) {
			await page.click('button[aria-label="Replay welcome animation"]');
			await page.evaluate(
				() => new Promise((resolve) => setTimeout(resolve, 850)),
			);
			await page.screenshot({ path: "/tmp/onboarding-intro-1280.png" });
			await settle();
		}
		for (const theme of ["dark", "light"]) {
			await page.mouse.move(0, 0);
			await page.evaluate(
				(value) =>
					document.documentElement.classList.toggle("dark", value === "dark"),
				theme,
			);
			await settle();
			await page.screenshot({ path: `/tmp/onboarding-${theme}-${width}.png` });
			if (width < 600) {
				const contentHeight = await page.$eval("main", (element) =>
					Math.ceil(element.scrollHeight),
				);
				await page.setViewport({
					width,
					height: contentHeight,
					deviceScaleFactor: 1,
				});
				await settle();
				await page.screenshot({
					path: `/tmp/onboarding-${theme}-${width}-full.png`,
					fullPage: true,
				});
				await page.setViewport({ width, height, deviceScaleFactor: 1 });
			}
			const cards = await page.$$("button[aria-pressed]");
			await cards[0].click();
			await cards[1].click();
			await page.mouse.move(0, 0);
			await settle();
			await page.screenshot({
				path: `/tmp/onboarding-${theme}-${width}-selected.png`,
			});
			await cards[0].click();
			await cards[1].click();
			await page.evaluate(() => {
				document.querySelector("main")?.parentElement?.scrollTo(0, 0);
			});
			await page.mouse.move(0, 0);
			await settle();
		}
		console.log(
			width,
			await page.evaluate(() => ({
				overflow: document.documentElement.scrollWidth > innerWidth,
				headingTop: document.querySelector("h1").getBoundingClientRect().top,
				cards: [...document.querySelectorAll("button[aria-pressed]")].map(
					(card) => {
						const { top, bottom, left, right } = card.getBoundingClientRect();
						return { top, bottom, left, right };
					},
				),
			})),
		);
	}
} finally {
	await browser.close();
}
