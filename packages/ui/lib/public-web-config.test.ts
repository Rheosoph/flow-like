import { afterEach, describe, expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { runInNewContext } from "node:vm";
import { getApiOrigin, getApiUrl } from "./api-url";
import {
	getPublicApiUrl,
	getPublicWebConfig,
	resolvePublicWebConfig,
} from "./public-web-config";

const originalEnvironment = {
	runtime: process.env.NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG,
	api: process.env.NEXT_PUBLIC_API_URL,
	redirect: process.env.NEXT_PUBLIC_REDIRECT_URL,
	logout: process.env.NEXT_PUBLIC_REDIRECT_LOGOUT_URL,
};
const originalWindow = Object.getOwnPropertyDescriptor(globalThis, "window");

function setEnvironment(key: string, value: string | undefined) {
	if (value === undefined) Reflect.deleteProperty(process.env, key);
	else process.env[key] = value;
}

afterEach(() => {
	setEnvironment(
		"NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG",
		originalEnvironment.runtime,
	);
	setEnvironment("NEXT_PUBLIC_API_URL", originalEnvironment.api);
	setEnvironment("NEXT_PUBLIC_REDIRECT_URL", originalEnvironment.redirect);
	setEnvironment("NEXT_PUBLIC_REDIRECT_LOGOUT_URL", originalEnvironment.logout);
	if (originalWindow)
		Object.defineProperty(globalThis, "window", originalWindow);
	else Reflect.deleteProperty(globalThis, "window");
});

function browser(config: unknown, origin = "https://web.example.test") {
	Object.defineProperty(globalThis, "window", {
		configurable: true,
		value: { location: { origin }, __FLOW_LIKE_PUBLIC_CONFIG__: config },
	});
}

describe("public web runtime configuration", () => {
	test("startup script executes safely and supplies the shared API resolver", () => {
		const directory = mkdtempSync(
			join(tmpdir(), "flow-like-public-config-test-"),
		);
		try {
			const output = join(directory, "runtime-config.js");
			const apiUrl =
				"https://api.example.test/';globalThis.injected=true;//</script><script>alert(1)</script>&\"";
			const result = spawnSync(
				"python3",
				[
					"-B",
					resolve(
						import.meta.dir,
						"../../../apps/backend/docker-compose/web/runtime-config.py",
					),
					"--output",
					output,
				],
				{
					encoding: "utf8",
					env: {
						NODE_ENV: process.env.NODE_ENV,
						PATH: process.env.PATH,
						FLOW_LIKE_WEB_API_URL: apiUrl,
						DATABASE_URL: "private-test-marker",
					},
				},
			);
			expect(result.status).toBe(0);
			expect(result.stdout).toBe("");
			expect(result.stderr).toBe("");
			const script = readFileSync(output, "utf8");
			expect(script).not.toContain("</script>");
			expect(script).not.toContain("private-test-marker");
			const context: {
				window: { __FLOW_LIKE_PUBLIC_CONFIG__?: unknown };
				injected?: boolean;
			} = { window: {} };
			runInNewContext(script, context);
			expect(context.injected).toBeUndefined();
			expect(Object.isFrozen(context.window.__FLOW_LIKE_PUBLIC_CONFIG__)).toBe(
				true,
			);
			process.env.NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG = "1";
			browser(context.window.__FLOW_LIKE_PUBLIC_CONFIG__);
			expect(getApiOrigin()).toBe(apiUrl);
			expect(getPublicWebConfig().redirectUrl).toBe(
				"https://web.example.test/callback",
			);
		} finally {
			rmSync(directory, { recursive: true, force: true });
		}
	});

	test("same artifact takes deployment API and derives callback/logout from browser origin", () => {
		process.env.NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG = "1";
		process.env.NEXT_PUBLIC_API_URL = "https://ignored-build.example.test";
		for (const tenant of ["one", "two"]) {
			browser(
				{ version: 1, apiUrl: `https://api.${tenant}.test/prefix///` },
				`https://web.${tenant}.test`,
			);
			expect(getPublicWebConfig()).toEqual({
				apiUrl: `https://api.${tenant}.test/prefix`,
				redirectUrl: `https://web.${tenant}.test/callback`,
				logoutUrl: `https://web.${tenant}.test/`,
			});
			expect(getPublicApiUrl()).toBe(`https://api.${tenant}.test/prefix`);
			expect(getApiOrigin({ hub: "ignored-profile.example.test" })).toBe(
				`https://api.${tenant}.test/prefix`,
			);
			expect(getApiUrl(null, "/profile")).toBe(
				`https://api.${tenant}.test/prefix/api/v1/profile`,
			);
		}
	});

	test("explicit authentication URLs override browser-origin defaults", () => {
		expect(
			resolvePublicWebConfig({
				runtimeEnabled: true,
				browserOrigin: "https://web.example.test",
				buildConfig: {},
				runtimeConfig: {
					version: 1,
					apiUrl: "https://api.example.test",
					redirectUrl: "https://login.example.test/callback",
					logoutUrl: "https://login.example.test/done",
				},
			}),
		).toEqual({
			apiUrl: "https://api.example.test",
			redirectUrl: "https://login.example.test/callback",
			logoutUrl: "https://login.example.test/done",
		});
	});

	test("container prerender has no runtime values or hosted fallback", () => {
		process.env.NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG = "1";
		process.env.NEXT_PUBLIC_API_URL = "https://ignored-build.example.test";
		Reflect.deleteProperty(globalThis, "window");
		expect(getPublicWebConfig()).toEqual({});
		expect(() => getPublicApiUrl()).toThrow("only available in the browser");
		expect(() => getApiOrigin()).toThrow();
	});

	test("missing, unsupported and unsafe browser settings fail closed", () => {
		process.env.NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG = "1";
		for (const config of [
			undefined,
			{},
			{ version: 2, apiUrl: "https://api.example.test" },
			...[
				"",
				"/api",
				"javascript:alert(1)",
				"https://user:secret@example.test",
				"https://api.example.test/?secret=value",
				"https://api.example.test/#fragment",
				"https://api.example.test/\n",
			].map((apiUrl) => ({ version: 1, apiUrl })),
		]) {
			browser(config);
			expect(() => getPublicApiUrl()).toThrow();
			expect(() =>
				getApiOrigin({ hub: "must-not-be-used.example.test" }),
			).toThrow();
		}
		browser({
			version: 1,
			apiUrl: "https://api.example.test",
			redirectUrl: "javascript:alert(1)",
		});
		expect(() => getPublicWebConfig()).toThrow();
	});

	test("noncontainer builds preserve env/profile/hosted precedence and unset redirects", () => {
		setEnvironment("NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG", undefined);
		setEnvironment("NEXT_PUBLIC_API_URL", undefined);
		setEnvironment("NEXT_PUBLIC_REDIRECT_URL", undefined);
		setEnvironment("NEXT_PUBLIC_REDIRECT_LOGOUT_URL", undefined);
		browser({ version: 1, apiUrl: "https://ignored-runtime.example.test" });
		expect(getPublicApiUrl()).toBe("https://api.flow-like.com");
		expect(getPublicWebConfig().redirectUrl).toBeUndefined();
		expect(getPublicWebConfig().logoutUrl).toBeUndefined();
		expect(getApiOrigin({ hub: "profile.example.test", secure: false })).toBe(
			"http://profile.example.test",
		);
		process.env.NEXT_PUBLIC_API_URL = "https://build.example.test";
		process.env.NEXT_PUBLIC_REDIRECT_URL =
			"https://build.example.test/callback";
		process.env.NEXT_PUBLIC_REDIRECT_LOGOUT_URL = "https://build.example.test/";
		expect(getPublicApiUrl()).toBe("https://build.example.test");
		expect(getApiOrigin({ hub: "profile.example.test" })).toBe(
			"https://build.example.test",
		);
		expect(getPublicWebConfig().redirectUrl).toBe(
			"https://build.example.test/callback",
		);
		expect(getPublicWebConfig().logoutUrl).toBe("https://build.example.test/");
	});
});
