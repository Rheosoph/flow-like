#!/usr/bin/env bun
import { createHash } from "node:crypto";
import { copyFile, readFile, writeFile } from "node:fs/promises";
import { basename, relative, resolve } from "node:path";
import {
	loadDocScreenshotPlan,
	screenshotScenarioFingerprint,
} from "../lib/doc-screenshot/plan";
import type { DocScreenshotResult } from "../lib/doc-screenshot/types";

const root = resolve(import.meta.dir, "../../..");
const assets = resolve(root, "apps/docs/src/assets");
const manifestPath = resolve(assets, "docs-screenshots.manifest.json");
const digest = (bytes: Uint8Array) =>
	createHash("sha256").update(bytes).digest("hex");
const manifest = await Bun.file(manifestPath)
	.json()
	.catch(() => ({
		schema: "flow-like.docs-screenshot-manifest/v1",
		screenshots: {},
	}));
const args = Bun.argv.slice(2);
const selectedIndex = args.indexOf("--scenario");
const selectedScenario =
	selectedIndex >= 0 ? args.splice(selectedIndex, 2)[1] : undefined;
if (selectedIndex >= 0 && !selectedScenario)
	throw new Error("--scenario requires a scenario name.");

if (args.includes("--check")) {
	if (Object.keys(manifest.screenshots).length === 0)
		throw new Error("Screenshot manifest is empty.");
	for (const [name, entry] of Object.entries(manifest.screenshots) as [
		string,
		any,
	][]) {
		if (digest(await readFile(resolve(assets, name))) !== entry.sha256)
			throw new Error(`${name}: image changed without review.`);
		const plan = await loadDocScreenshotPlan(resolve(root, entry.plan));
		const scenario = plan.scenarios.find(
			(item) => item.name === entry.scenario,
		);
		if (
			!scenario ||
			!entry.sourceSha256 ||
			screenshotScenarioFingerprint(plan, scenario) !== entry.sourceSha256
		)
			throw new Error(`${name}: scenario source changed; rerun and review.`);
		for (const [path, sha] of [
			[entry.tauriFixture, entry.fixtureSha256],
			[entry.httpFixture, entry.httpFixtureSha256],
		]) {
			if (path && sha && digest(await readFile(resolve(root, path))) !== sha)
				throw new Error(
					`${name}: capture source changed (${path}); rerun and review.`,
				);
		}
	}
	console.log(
		`Verified ${Object.keys(manifest.screenshots).length} screenshot images and capture sources.`,
	);
} else {
	if (args[0] !== "--reviewed" || args.length < 2)
		throw new Error(
			"Inspect the captures, then use --reviewed <capture-result.json> [...]. Use --check to verify the manifest.",
		);
	let count = 0;
	for (const resultPath of args.slice(1)) {
		const result = (await Bun.file(resultPath).json()) as DocScreenshotResult;
		if (
			result.schema !== "flow-like.doc-screenshot-result/v1" ||
			!result.provenance
		)
			throw new Error(`${resultPath}: missing capture provenance.`);
		if (!result.passed && !selectedScenario)
			throw new Error(`${resultPath}: failed captures cannot be published.`);
		const scenarios = selectedScenario
			? result.scenarios.filter(
					(scenario) => scenario.name === selectedScenario,
				)
			: result.scenarios;
		if (!scenarios.length)
			throw new Error(`${resultPath}: unknown scenario ${selectedScenario}.`);
		for (const scenario of scenarios) {
			if (!scenario.passed || !scenario.sourceSha256)
				throw new Error(
					`${resultPath}: scenario did not pass or lacks a source fingerprint.`,
				);
			for (const artifact of scenario.artifacts) {
				const name = basename(artifact.path);
				const bytes = await readFile(artifact.path);
				if (digest(bytes) !== artifact.sha256)
					throw new Error(`${artifact.path}: changed after capture.`);
				await copyFile(artifact.path, resolve(assets, name));
				manifest.screenshots[name] = {
					...result.provenance,
					scenario: scenario.name,
					sourceSha256: scenario.sourceSha256,
					url: artifact.url,
					capturedAt: artifact.capturedAt,
					reviewedAt: new Date().toISOString(),
					sha256: artifact.sha256,
					bytes: artifact.bytes,
					mode: artifact.mode,
					selector: artifact.selector,
					pixels: artifact.pixels,
					render: scenario.render,
					browser: result.browser,
					diagnosticAllowances: scenario.diagnostics.entries.filter(
						(entry) => entry.allowance,
					),
				};
				count += 1;
			}
		}
	}
	manifest.screenshots = Object.fromEntries(
		Object.entries(manifest.screenshots).sort(([a], [b]) => a.localeCompare(b)),
	);
	await writeFile(manifestPath, `${JSON.stringify(manifest, null, "\t")}\n`);
	console.log(
		`Published ${count} reviewed screenshots and updated ${relative(root, manifestPath)}.`,
	);
}
