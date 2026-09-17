import { describe, expect, test } from "bun:test";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { contractToJson } from "../src/contract-types";
import { buildCsp } from "../src/csp";
import { extractContract } from "../src/extract";
import { HELLO_WIDGET_CONFIG, tmpDir } from "./helpers";

describe("widget capabilities", () => {
	test("workers enable bundled fetches, never remote provider access", () => {
		const csp = buildCsp(null, {
			workers: true,
			media: true,
			wasm: true,
		});
		expect(csp).toContain("worker-src blob:");
		expect(csp).toContain("media-src blob:");
		expect(csp).toContain(
			"connect-src 'self' flow-widget: http://flow-widget.localhost blob:",
		);
		expect(csp).toContain("'wasm-unsafe-eval'");
		expect(csp).not.toContain("'unsafe-eval'");
		expect(csp).not.toContain("https:");
	});
	test("declarations are opt-in and survive contract extraction", () => {
		const path = join(tmpDir("widget-caps"), "widget.config.ts");
		writeFileSync(
			path,
			HELLO_WIDGET_CONFIG.replace(
				'id: "hello-widget",',
				'id: "hello-widget", capabilities: {workers:true,media:true,microphone:false},',
			),
		);
		const result = extractContract(path);
		expect(JSON.parse(contractToJson(result.contract)).capabilities).toEqual({
			workers: true,
			media: true,
			microphone: false,
		});
		expect(buildCsp(null)).toContain("worker-src 'none'; media-src 'none'");
	});
	test("rejects the removed resources capability", () => {
		const path = join(tmpDir("widget-caps-resources"), "widget.config.ts");
		writeFileSync(
			path,
			HELLO_WIDGET_CONFIG.replace(
				'id: "hello-widget",',
				'id: "hello-widget", capabilities: {resources:true},',
			),
		);
		expect(() => extractContract(path)).toThrow(/Invalid widget capability/);
		expect(buildCsp(null, { resources: true } as never)).toContain(
			"connect-src 'none'",
		);
	});
	test("rejects unknown permissions and CSP directive injection", () => {
		const path = join(tmpDir("widget-caps-bad"), "widget.config.ts");
		writeFileSync(
			path,
			HELLO_WIDGET_CONFIG.replace(
				'id: "hello-widget",',
				'id: "hello-widget", capabilities: {sameOrigin:true},',
			),
		);
		expect(() => extractContract(path)).toThrow("Invalid widget capability");
		for (const prefix of [
			"https://safe.example.org/; script-src *",
			"https://safe.example.org/ 'unsafe-eval'",
			"data:",
		])
			expect(() => buildCsp(prefix)).toThrow("Invalid widget CSP source");
	});
	test("host lists passed where capabilities belong grant nothing", () => {
		const csp = buildCsp(null, ["https://api.example.com"] as never);
		expect(csp).toContain("connect-src 'none'");
		expect(csp).not.toContain("api.example.com");
	});
});
