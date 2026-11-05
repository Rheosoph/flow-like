import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import {
	getInviteDeepLink,
	getInviteLink,
	getRoomInviteLink,
} from "./invite-link";

const originalApi = process.env.NEXT_PUBLIC_API_URL;
const originalRuntime = process.env.NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG;
const originalWindow = Object.getOwnPropertyDescriptor(globalThis, "window");

function setEnvironment(name: string, value: string | undefined) {
	if (value === undefined) Reflect.deleteProperty(process.env, name);
	else process.env[name] = value;
}

beforeEach(() => {
	Reflect.deleteProperty(process.env, "NEXT_PUBLIC_API_URL");
	Reflect.deleteProperty(process.env, "NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG");
});

afterEach(() => {
	setEnvironment("NEXT_PUBLIC_API_URL", originalApi);
	setEnvironment("NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG", originalRuntime);
	if (originalWindow)
		Object.defineProperty(globalThis, "window", originalWindow);
	else Reflect.deleteProperty(globalThis, "window");
});

describe("shared invitation links", () => {
	test("uses the hosted API by default", () => {
		expect(getInviteLink("app", "invite")).toBe(
			"https://api.flow-like.com/join?appId=app&token=invite",
		);
	});

	test("uses the profile hub and its configured HTTP protocol", () => {
		expect(
			getInviteLink("app", "invite", {
				hub: "localhost:8080/tenant/api/v1///",
				secure: false,
			}),
		).toBe("http://localhost:8080/tenant/join?appId=app&token=invite");
	});

	test("normalizes configured API origins and versioned bases without losing prefixes", () => {
		for (const [base, expected] of [
			["https://api.example.test/", "https://api.example.test/join"],
			["https://api.example.test/api/v1/", "https://api.example.test/join"],
			[
				"https://api.example.test/service/api/v1",
				"https://api.example.test/service/join",
			],
			[
				"https://api.example.test/service",
				"https://api.example.test/service/join",
			],
			[
				"https://api.example.test/api/v10",
				"https://api.example.test/api/v10/join",
			],
			["api.example.test", "https://api.example.test/join"],
		]) {
			process.env.NEXT_PUBLIC_API_URL = base;
			expect(
				getInviteLink("app", "invite", { hub: "ignored.example.test" }),
			).toBe(`${expected}?appId=app&token=invite`);
		}
	});

	test("container runtime API settings take precedence over build and profile settings", () => {
		process.env.NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG = "1";
		process.env.NEXT_PUBLIC_API_URL = "https://build.example.test";
		Object.defineProperty(globalThis, "window", {
			configurable: true,
			value: {
				location: { origin: "https://app.example.test" },
				__FLOW_LIKE_PUBLIC_CONFIG__: {
					version: 1,
					apiUrl: "https://runtime.example.test/tenant/api/v1/",
				},
			},
		});
		expect(
			getInviteLink("app", "invite", { hub: "profile.example.test" }),
		).toBe("https://runtime.example.test/tenant/join?appId=app&token=invite");
	});

	test("encodes invitation values identically for HTTPS and desktop links", () => {
		const appId = "project/one?&name=二";
		const token = "invite+ &?/#=é";
		for (const link of [
			getInviteLink(appId, token),
			getInviteDeepLink(appId, token),
		]) {
			const url = new URL(link);
			expect(url.searchParams.get("appId")).toBe(appId);
			expect(url.searchParams.get("token")).toBe(token);
			expect([...url.searchParams.keys()]).toEqual(["appId", "token"]);
			expect(url.hash).toBe("");
		}
		expect(getInviteDeepLink("app", "invite")).toBe(
			"flow-like://join?appId=app&token=invite",
		);
	});

	test("refuses to copy credentials or unrelated query parameters from the API base", () => {
		for (const base of [
			"https://user:secret@api.example.test",
			"https://api.example.test?access_token=secret",
			"https://api.example.test#secret",
		]) {
			process.env.NEXT_PUBLIC_API_URL = base;
			expect(() => getInviteLink("app", "invite")).toThrow(
				"The invitation API URL must be an HTTP(S) base URL",
			);
		}
	});
});

describe("room invite links", () => {
	test("point at the public join page with the token as room", () => {
		expect(getRoomInviteLink("abc")).toBe(
			"https://api.flow-like.com/join?room=abc",
		);
	});

	test("keep the deployment prefix and encode the token", () => {
		process.env.NEXT_PUBLIC_API_URL =
			"https://api.example.test/service/api/v1/";
		const url = new URL(getRoomInviteLink("a+b &/?#é"));
		expect(url.pathname).toBe("/service/join");
		expect(url.searchParams.get("room")).toBe("a+b &/?#é");
		expect([...url.searchParams.keys()]).toEqual(["room"]);
	});

	test("share the base URL validation", () => {
		process.env.NEXT_PUBLIC_API_URL = "https://user:secret@api.example.test";
		expect(() => getRoomInviteLink("abc")).toThrow(
			"The invitation API URL must be an HTTP(S) base URL",
		);
	});
});
