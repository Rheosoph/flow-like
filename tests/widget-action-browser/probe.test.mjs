import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import test from "node:test";
import puppeteer from "puppeteer";
import { installAppMeasurement } from "./app-measurement.mjs";

test("history readiness waits for matching rows and a new completed trace", async () => {
	const macChrome =
		"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
	const browser = await puppeteer.launch({
		headless: true,
		executablePath:
			process.env.CHROME_PATH ||
			(existsSync(macChrome) ? macChrome : undefined),
	});
	try {
		const page = await browser.newPage();
		await page.setContent(
			'<button id="one">Person one</button><button id="two">Person two</button><div id="history"></div>',
		);
		await page.evaluate(() => {
			window.__flowLikeRunTimings = [
				{ label: "widget action", totalMs: 1 },
				{ label: "widget action", totalMs: 2 },
			];
			for (const person of ["one", "two"])
				document.getElementById(person).addEventListener("click", () => {
					// An unrelated early update cannot satisfy the semantic table boundary.
					document.getElementById("history").textContent = "Loading";
					setTimeout(() => {
						document.getElementById("history").innerHTML =
							"<table><tbody><tr><td>Rental one</td></tr><tr><td>Rental two</td></tr></tbody></table>";
					}, 30);
					setTimeout(() => {
						document.getElementById("history").innerHTML =
							`<table><tbody><tr><td>Rental ${person}</td></tr><tr><td>Second rental</td></tr></tbody></table>`;
					}, 100);
					setTimeout(() => {
						window.__flowLikeRunTimings.push({
							label: "widget action",
							totalMs: 180,
							marks: [{ name: "dispatch", atMs: 5 }],
						});
					}, 180);
				});
		});
		await page.evaluate(installAppMeasurement, {
			timeoutMs: 2000,
			actions: ["one", "two"].map((person) => ({
				selector: `#${person}`,
				historySelector: "#history",
				expectedText: `Rental ${person}`,
				expectedRows: 2,
			})),
		});
		for (const [index, person] of ["one", "two"].entries()) {
			await page.evaluate(
				(index) => window.__flowLikeAppBenchmark.arm(index),
				index,
			);
			await page.click(`#${person}`);
			await page.waitForFunction(() => window.__flowLikeAppBenchmark.result);
			const result = await page.evaluate(
				() => window.__flowLikeAppBenchmark.result,
			);
			assert.equal(result.status, "ok");
			assert.equal(result.syntheticBrowserInputTrusted, true);
			assert.equal(result.visibleRows, 2);
			assert.ok(result.clickToHistoryDomMs >= 90);
			assert.ok(
				result.clickToHistoryFrameOpportunityMs >= result.clickToHistoryDomMs,
			);
			assert.ok(result.clickToSettledMs >= 170);
			assert.equal(result.handlerToSettledMs, 180);
			assert.equal(result.handlerToDispatchMs, 5);
		}
		await assert.rejects(
			page.evaluate(() => window.__flowLikeAppBenchmark.arm(1)),
			/already matches/,
		);
	} finally {
		await browser.close();
	}
});

test("reset readiness waits for the cards and a fresh workflow trace", async () => {
	const macChrome =
		"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
	const browser = await puppeteer.launch({
		headless: true,
		executablePath:
			process.env.CHROME_PATH ||
			(existsSync(macChrome) ? macChrome : undefined),
	});
	try {
		const page = await browser.newPage();
		await page.setContent(
			'<div id="cards" style="display:none"><button id="one">One</button><button id="two">Two</button></div><button id="back">Return to cards</button><div id="history">Rental one</div>',
		);
		await page.evaluate(() => {
			window.__flowLikeRunTimings = [{ label: "workflow action", totalMs: 1 }];
			document.getElementById("back").addEventListener("click", () => {
				setTimeout(() => {
					document.getElementById("cards").style.display = "block";
				}, 80);
				setTimeout(() => {
					window.__flowLikeRunTimings.push({
						label: "workflow action",
						totalMs: 160,
					});
				}, 160);
			});
		});
		await page.evaluate(installAppMeasurement, {
			timeoutMs: 2000,
			reset: { selector: "#back" },
			actions: ["one", "two"].map((person) => ({
				selector: `#${person}`,
				historySelector: "#history",
				expectedText: `Rental ${person}`,
			})),
		});
		await page.evaluate(() => window.__flowLikeAppBenchmark.arm(null, true));
		await page.click("#back");
		await page.waitForFunction(() => window.__flowLikeAppBenchmark.result);
		const result = await page.evaluate(
			() => window.__flowLikeAppBenchmark.result,
		);
		assert.equal(result.status, "ok");
		assert.equal(result.readyActions, 2);
		assert.equal(result.visibleRows, undefined);
		assert.equal(result.clickToHistoryDomMs, undefined);
		assert.ok(result.clickToCardsDomMs >= 70);
		assert.ok(
			result.clickToCardsFrameOpportunityMs >= result.clickToCardsDomMs,
		);
		assert.ok(result.clickToSettledMs >= 150);
		assert.equal(result.handlerToSettledMs, 160);
		await assert.rejects(
			page.evaluate(() => window.__flowLikeAppBenchmark.arm(null, true)),
			/already ready/,
		);
	} finally {
		await browser.close();
	}
});
