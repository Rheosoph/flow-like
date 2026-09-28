import { describe, expect, test } from "bun:test";
import { installCspReporting } from "./csp-reporting";

function createDocument() {
	return Object.assign(new EventTarget(), {
		location: new URL(
			"https://app.example.test/callback?code=secret#token=secret",
		),
	}) as unknown as Document;
}

function emit(document: Document, overrides: Record<string, unknown> = {}) {
	document.dispatchEvent(
		Object.assign(new Event("securitypolicyviolation"), {
			disposition: "report",
			effectiveDirective: "script-src-elem",
			blockedURI: "https://cdn.example.test/script.js?token=secret",
			documentURI: document.location.href,
			referrer: "https://private.example.test/?secret=referrer",
			sourceFile: "https://private.example.test/user-content.js",
			sample: "const token = 'secret'",
			originalPolicy: "script-src https://private.example.test",
			...overrides,
		}),
	);
}

describe("CSP telemetry", () => {
	test("sends only finite categories, excluding URLs and source details", () => {
		const document = createDocument();
		const reports: unknown[] = [];
		const remove = installCspReporting(document, (report) =>
			reports.push(report),
		);
		emit(document);
		expect(reports).toEqual([
			{ directive: "script-src-elem", resource: "cross-origin" },
		]);
		remove();
	});

	test("classifies browser resource identifiers without transmitting them", () => {
		const document = createDocument();
		const reports: unknown[] = [];
		const remove = installCspReporting(document, (report) =>
			reports.push(report),
		);
		for (const blockedURI of [
			"inline",
			"eval",
			"wasm-eval",
			"https://app.example.test/private.js?secret=yes",
			"blob:https://app.example.test/private-id",
			"data:text/javascript,secret",
			"not a URL with secrets",
		]) {
			emit(document, { blockedURI });
		}
		expect(reports).toEqual(
			[
				"inline",
				"eval",
				"wasm-eval",
				"same-origin",
				"blob",
				"data",
				"other",
			].map((resource) => ({ directive: "script-src-elem", resource })),
		);
		remove();
	});

	test("recognizes scheme-only identifiers emitted by browsers", () => {
		const document = createDocument();
		const reports: unknown[] = [];
		const remove = installCspReporting(document, (report) =>
			reports.push(report),
		);
		emit(document, { blockedURI: "data" });
		emit(document, { blockedURI: "blob" });
		expect(reports).toEqual([
			{ directive: "script-src-elem", resource: "data" },
			{ directive: "script-src-elem", resource: "blob" },
		]);
		remove();
	});

	test("ignores enforced policies and unrecognized directives", () => {
		const document = createDocument();
		const reports: unknown[] = [];
		const remove = installCspReporting(document, (report) =>
			reports.push(report),
		);
		emit(document, { disposition: "enforce" });
		emit(document, { disposition: "secret" });
		emit(document, { effectiveDirective: "connect-src" });
		emit(document, { effectiveDirective: "script-src https://secret.example" });
		expect(reports).toEqual([]);
		remove();
	});

	test("bounds malformed or unusually long resource identifiers", () => {
		for (const blockedURI of [undefined, {}, "x".repeat(100_000)]) {
			const document = createDocument();
			const reports: unknown[] = [];
			const remove = installCspReporting(document, (report) =>
				reports.push(report),
			);
			emit(document, { blockedURI });
			expect(reports).toEqual([
				{ directive: "script-src-elem", resource: "other" },
			]);
			remove();
		}
	});

	test("deduplicates categories across URLs, remounts, and concurrent listeners", () => {
		const document = createDocument();
		const reports: unknown[] = [];
		const capture = (report: unknown) => reports.push(report);
		const removeFirst = installCspReporting(document, capture);
		const removeSecond = installCspReporting(document, capture);
		emit(document);
		emit(document, { blockedURI: "https://another.example/private.js" });
		removeFirst();
		removeSecond();
		const removeThird = installCspReporting(document, capture);
		emit(document);
		expect(reports).toHaveLength(1);
		removeThird();
	});

	test("caps reports across remounts for the full document lifetime", () => {
		const document = createDocument();
		const reports: unknown[] = [];
		for (const effectiveDirective of [
			"script-src",
			"script-src-elem",
			"script-src-attr",
			"worker-src",
			"base-uri",
			"object-src",
		]) {
			const remove = installCspReporting(document, (report) =>
				reports.push(report),
			);
			for (const blockedURI of ["inline", "eval", "wasm-eval", "data:secret"]) {
				emit(document, { effectiveDirective, blockedURI });
			}
			remove();
		}
		expect(reports).toHaveLength(20);
	});

	test("removes listeners on opt-out or unmount", () => {
		const document = createDocument();
		const reports: unknown[] = [];
		const remove = installCspReporting(document, (report) =>
			reports.push(report),
		);
		remove();
		emit(document);
		expect(reports).toEqual([]);
	});

	test("contains failures in the telemetry sink", () => {
		const document = createDocument();
		const remove = installCspReporting(document, () => {
			throw new Error("unavailable");
		});
		expect(() => emit(document)).not.toThrow();
		remove();
	});
});
