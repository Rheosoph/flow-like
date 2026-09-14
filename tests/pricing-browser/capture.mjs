import { mkdir, mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import puppeteer from "puppeteer";

const output = resolve(
	process.env.SCREENSHOT_OUTPUT ?? "output/pricing-ui-screenshots",
);
await mkdir(output, { recursive: true });
const profile = await mkdtemp(join(tmpdir(), "flow-like-pricing-chrome-"));
const browser = await puppeteer.launch({
	headless: true,
	executablePath:
		"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
	args: ["--no-sandbox", "--disable-gpu", "--lang=en-GB"],
	userDataDir: profile,
	protocolTimeout: 120000,
});
const captures = [];
const errors = [];
const scenarios = [
	{
		name: "runtime-calculator-personal",
		title: "Run calculator · Your recent average",
		group: "Plans",
		query: "view=subscription",
		action: "runtime-calculator",
	},
	{
		name: "runtime-calculator-example",
		title: "Run calculator · New account example",
		group: "Plans",
		query: "view=subscription&scenario=empty&tier=FREE&light",
		action: "runtime-calculator",
	},
	{
		name: "runtime-calculator-remaining",
		title: "Run calculator · Remaining allowance",
		group: "Usage",
		query: "view=usage",
		action: "runtime-calculator",
	},
	{
		name: "runtime-calculator-mobile",
		title: "Run calculator · Mobile",
		group: "Mobile",
		query: "view=subscription&light",
		action: "runtime-calculator",
		mobile: true,
	},
	{
		name: "runtime-estimate-usage",
		title: "Usage · Estimated remaining cloud runs",
		group: "Usage",
		query: "view=usage",
	},
	{
		name: "subscription-usage-tab",
		title: "Subscription · Usage tab",
		group: "Usage",
		query: "view=subscription&tab=usage",
		action: "usage-tab",
	},
	{
		name: "plan-comparison-light",
		title: "Plans · Full allowance comparison",
		group: "Plans",
		query: "view=subscription&light",
		action: "comparison",
	},
	{
		name: "usage-other-limits",
		title: "Usage · Other limits expanded",
		group: "Usage",
		query: "view=usage",
		action: "secondary",
	},
	{
		name: "upgrade-other-plans",
		title: "Upgrade · Other plans expanded",
		group: "Upgrade dialogs",
		query: "view=upgrade",
		action: "dialog-options",
		viewportOnly: true,
	},
	{
		name: "usage-details-mobile",
		title: "Operation details · Mobile panel",
		group: "Mobile",
		query: "view=history",
		action: "history-details",
		mobile: true,
		viewportOnly: true,
	},
	{
		name: "plan-overview-premium-dark",
		title: "Plan overview · Premium",
		group: "Plans",
		query: "view=subscription",
	},
	{
		name: "plan-overview-free-light",
		title: "Plan overview · Free",
		group: "Plans",
		query: "view=subscription&tier=FREE&light",
	},
	{
		name: "plan-overview-annual-dark",
		title: "Plan overview · Annual billing",
		group: "Plans",
		query: "view=subscription",
		action: "annual",
	},
	{
		name: "tier-cards-monthly-dark",
		title: "All four tiers · Monthly",
		group: "Plans",
		query: "view=cards&tier=FREE",
	},
	{
		name: "tier-cards-yearly-light",
		title: "All four tiers · Annual",
		group: "Plans",
		query: "view=cards&tier=FREE&interval=year&light",
	},
	{
		name: "tier-cards-max-current-dark",
		title: "Max · Current plan",
		group: "Plans",
		query: "view=cards&tier=MAX",
	},
	{
		name: "plan-overview-mobile",
		title: "Plan overview · Mobile",
		group: "Mobile",
		query: "view=subscription",
		mobile: true,
	},
	{
		name: "usage-overview-dark",
		title: "Usage overview · Warnings and reservations",
		group: "Usage",
		query: "view=usage",
	},
	{
		name: "usage-overview-light",
		title: "Usage overview · Light",
		group: "Usage",
		query: "view=usage&light",
	},
	{
		name: "usage-filtered-app-model",
		title: "Usage filtered by app and model",
		group: "Usage",
		query: "view=usage",
		action: "filter",
	},
	{
		name: "usage-empty",
		title: "Usage · New account",
		group: "Usage",
		query: "view=usage&scenario=empty&tier=FREE&light",
	},
	{
		name: "usage-unavailable",
		title: "Usage · API unavailable",
		group: "Usage",
		query: "view=usage&scenario=error&light",
	},
	{
		name: "usage-first-tracking-period",
		title: "Usage · First tracking period",
		group: "Usage",
		query: "view=usage&scenario=cutover",
	},
	{
		name: "usage-overview-mobile",
		title: "Usage overview · Mobile",
		group: "Mobile",
		query: "view=usage&light",
		mobile: true,
	},
	{
		name: "usage-history",
		title: "Operation history · Completed and pending",
		group: "Usage details",
		query: "view=history",
		action: "history",
	},
	{
		name: "usage-history-expanded",
		title: "Operation history · Expanded cost details",
		group: "Usage details",
		query: "view=history",
		action: "history-details",
	},
	{
		name: "usage-cost-details",
		title: "Hosted AI · Cost and token details",
		group: "Usage details",
		query: "view=details",
		action: "details",
		height: 760,
	},
	{
		name: "usage-embedding-details",
		title: "Embeddings · Estimated metering",
		group: "Usage details",
		query: "view=details&scenario=embedding&light",
		action: "details",
		height: 800,
	},
	{
		name: "usage-byok-details",
		title: "Own model · No hosted AI usage",
		group: "Usage details",
		query: "view=details&scenario=byok",
		action: "details",
		height: 620,
	},
	{
		name: "usage-pending-details",
		title: "Hosted AI · Pending provider reconciliation",
		group: "Usage details",
		query: "view=details&scenario=pending",
		action: "details",
		height: 760,
	},
	...[75, 90, 100].map((level) => ({
		name: `quota-warning-${level}`,
		title: `Quota notification · ${level}%`,
		group: "Quota warnings",
		query: `view=warning&level=${level}`,
		action: "warning",
		height: 1100,
	})),
	{
		name: "quota-warning-mobile",
		title: "Quota notification · Mobile",
		group: "Mobile",
		query: "view=warning&level=75&light",
		action: "warning",
		mobile: true,
		viewportOnly: true,
	},
	...[
		"ai-budget",
		"runtime",
		"ai-calls",
		"executions",
		"concurrency",
		"storage",
		"project-limit",
		"model-tier",
		"generic",
	].map((reason) => ({
		name: `upgrade-${reason}`,
		title: `Upgrade dialog · ${reason}`,
		group: "Upgrade dialogs",
		query: `view=upgrade&reason=${reason}`,
		action: "dialog",
		height: 1280,
		viewportOnly: true,
	})),
	{
		name: "upgrade-ai-budget-light",
		title: "Upgrade dialog · Light",
		group: "Upgrade dialogs",
		query: "view=upgrade&light",
		action: "dialog",
		height: 1280,
		viewportOnly: true,
	},
	{
		name: "upgrade-shared-owner",
		title: "Shared app · Billing owner limit",
		group: "Upgrade edge cases",
		query: "view=upgrade&scenario=shared&reason=runtime",
		action: "dialog",
		viewportOnly: true,
	},
	{
		name: "upgrade-shared-model",
		title: "Shared app · Model access",
		group: "Upgrade edge cases",
		query: "view=upgrade&scenario=shared&reason=model-tier&light",
		action: "dialog",
		viewportOnly: true,
	},
	{
		name: "upgrade-max-contact",
		title: "Max · Enterprise capacity",
		group: "Upgrade edge cases",
		query: "view=upgrade&tier=MAX",
		action: "dialog",
		viewportOnly: true,
	},
	{
		name: "upgrade-enterprise-model",
		title: "Model requires an Enterprise agreement",
		group: "Upgrade edge cases",
		query: "view=upgrade&reason=model-tier&required=ENTERPRISE",
		action: "dialog",
		viewportOnly: true,
	},
	{
		name: "upgrade-pricing-unavailable",
		title: "Upgrade options · API unavailable",
		group: "Upgrade edge cases",
		query: "view=upgrade&scenario=pricing-error",
		action: "dialog",
		viewportOnly: true,
	},
	{
		name: "upgrade-mobile-top",
		title: "Upgrade dialog · Mobile, top",
		group: "Mobile",
		query: "view=upgrade",
		action: "dialog",
		mobile: true,
		viewportOnly: true,
	},
	{
		name: "upgrade-mobile-plans",
		title: "Upgrade dialog · Mobile, plans",
		group: "Mobile",
		query: "view=upgrade",
		action: "dialog-plans",
		mobile: true,
		viewportOnly: true,
	},
	{
		name: "upgrade-mobile-bottom",
		title: "Upgrade dialog · Mobile, bottom",
		group: "Mobile",
		query: "view=upgrade",
		action: "dialog-bottom",
		mobile: true,
		viewportOnly: true,
	},
];

try {
	for (const scenario of scenarios.filter(
		(item) =>
			!process.env.ONLY || process.env.ONLY.split(",").includes(item.name),
	)) {
		const page = await browser.newPage();
		page.setDefaultTimeout(120000);
		page.on("pageerror", (error) =>
			errors.push({ screenshot: scenario.name, error: error.message }),
		);
		await page.setRequestInterception(true);
		page.on("request", (request) => {
			if (/^(http:\/\/127\.0\.0\.1:4330\/|data:|blob:)/.test(request.url()))
				request.continue();
			else request.abort();
		});
		const viewport = scenario.mobile
			? {
					width: 390,
					height: 844,
					deviceScaleFactor: 2,
					isMobile: true,
					hasTouch: true,
				}
			: { width: 1440, height: scenario.height ?? 1100, deviceScaleFactor: 2 };
		await page.setViewport(viewport);
		await page.emulateTimezone("Europe/Berlin");
		await page.goto(`http://127.0.0.1:4330/?${scenario.query}`, {
			waitUntil: "networkidle0",
		});
		await page.waitForSelector("[data-preview-ready]");
		await page.evaluate(() => document.fonts.ready);
		const clickText = async (selector, text) => {
			await page.waitForFunction(
				(selector, text) =>
					[...document.querySelectorAll(selector)].some(
						(node) => node.textContent.trim() === text,
					),
				{},
				selector,
				text,
			);
			await page.evaluate(
				(selector, text) =>
					[...document.querySelectorAll(selector)]
						.find((node) => node.textContent.trim() === text)
						.click(),
				selector,
				text,
			);
		};
		if (scenario.action === "annual") await clickText("button", "Annual");
		if (scenario.action === "runtime-calculator")
			await page.$eval("[data-runtime-calculator] summary", (element) =>
				element.click(),
			);
		if (scenario.action === "usage-tab") {
			await page.waitForSelector(
				'[role="tab"][data-state="active"][value="usage"], [role="tabpanel"] [aria-label="Current usage"]',
			);
		}
		if (scenario.action === "comparison")
			await clickText("summary", "Compare all limits");
		if (scenario.action === "secondary")
			await page.evaluate(() =>
				[...document.querySelectorAll("summary")]
					.find((node) => node.textContent.includes("Other plan limits"))
					.click(),
			);
		if (scenario.action === "filter") {
			await page.select('select[aria-label="Filter by app"]', "invoice-review");
			await page.select('select[aria-label="Filter by model"]', "hosted-small");
		}
		if (scenario.action?.startsWith("history")) {
			await clickText("summary", "Operation history and pending usage");
			await page.waitForFunction(() =>
				document.body.innerText.includes("Invoice Review"),
			);
			if (scenario.action === "history-details")
				await clickText("button", "Cost and token details");
		}
		if (scenario.action === "details")
			await clickText("button", "Cost and token details");
		if (
			scenario.action === "details" ||
			scenario.action === "history-details"
		) {
			await page.waitForSelector('[role="dialog"]');
			await page.waitForFunction(
				() => !document.querySelector('[role="dialog"] [role="status"]'),
			);
		}
		if (scenario.action === "warning")
			await page.waitForSelector("[data-sonner-toast]");
		if (scenario.action?.startsWith("dialog")) {
			await page.waitForSelector('[role="dialog"]');
			await page.waitForFunction(
				() => !document.querySelector('[role="dialog"] .animate-spin'),
			);
			if (scenario.action === "dialog-bottom")
				await page.$eval('[role="dialog"]', (element) => {
					element.scrollTop = element.scrollHeight;
				});
			if (
				scenario.action === "dialog-plans" ||
				scenario.action === "dialog-options"
			) {
				await page.evaluate(() =>
					[...document.querySelectorAll('[role="dialog"] summary')]
						.find((node) => node.textContent.includes("See other plans"))
						.click(),
				);
				await page.$eval('[role="dialog"]', (element) => {
					element.scrollTop = Math.max(
						0,
						element.scrollHeight - element.clientHeight,
					);
				});
			}
		}
		await new Promise((resolve) => setTimeout(resolve, 450));
		const overflow = await page.evaluate(() => {
			const dialog = document.querySelector('[role="dialog"]');
			return (
				document.documentElement.scrollWidth > window.innerWidth + 1 ||
				(dialog && dialog.scrollWidth > dialog.clientWidth + 1)
			);
		});
		if (overflow)
			errors.push({
				screenshot: scenario.name,
				error: "Page or dialog has horizontal overflow",
			});
		await page.screenshot({
			path: join(output, `${scenario.name}.png`),
			fullPage:
				!scenario.viewportOnly &&
				scenario.action !== "details" &&
				scenario.action !== "history-details",
		});
		captures.push({
			...scenario,
			viewport,
			file: `${scenario.name}.png`,
			fixture: true,
		});
		if (scenario.action === "runtime-calculator") {
			const calculator = await page.$("[data-runtime-calculator]");
			await calculator.screenshot({
				path: join(output, `${scenario.name}-detail.png`),
			});
			captures.push({
				...scenario,
				viewport,
				file: `${scenario.name}-detail.png`,
				title: `${scenario.title} · Calculator detail`,
				fixture: true,
			});
		}
		if (scenario.action === "warning") {
			const notice = await page.$("[data-sonner-toast]");
			await notice.screenshot({
				path: join(output, `${scenario.name}-detail.png`),
			});
			captures.push({
				...scenario,
				title: `${scenario.title} · Notification detail`,
				viewport,
				file: `${scenario.name}-detail.png`,
				fixture: true,
			});
		}
		console.log(`Captured ${scenario.name}`);
		await page.close();
	}
} finally {
	await browser.close();
	await rm(profile, { recursive: true, force: true });
	await writeFile(
		join(
			output,
			process.env.ONLY ? "app-preview-manifest.json" : "app-manifest.json",
		),
		JSON.stringify({ captures, errors }, null, 2),
	);
}
if (errors.length) {
	console.error(JSON.stringify(errors, null, 2));
	process.exitCode = 1;
}
