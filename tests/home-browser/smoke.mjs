import { chromium } from "playwright-core";
const browser = await chromium.launch({
	executablePath:
		"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
	headless: true,
	args: ["--disable-dev-shm-usage", "--no-sandbox"],
});
const page = await browser.newPage({ viewport: { width: 1480, height: 1050 } });
await page.route("**/*", (route) => {
	const url = new URL(route.request().url());
	return url.hostname === "127.0.0.1" || url.protocol === "data:"
		? route.continue()
		: route.abort();
});
page.on("pageerror", (error) => console.log("PAGE ERROR:", error.stack));
page.on("console", (message) => {
	if (message.type() === "error") console.log("CONSOLE ERROR:", message.text());
});
await page.goto("http://127.0.0.1:4318/", { waitUntil: "domcontentloaded" });
try {
	await page
		.getByRole("button", { name: "Customize", exact: true })
		.waitFor({ timeout: 60_000 });
} catch {}
await page.screenshot({
	path: "/private/tmp/home-qa-initial.png",
	fullPage: true,
});
console.log((await page.locator("body").innerText()).slice(0, 5000));
await browser.close();
