import { describe, expect, test } from "bun:test";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import {
	authenticatedWebhookUrl,
	webhookCurlExamples,
} from "./bot-webhook-examples";

const url = "https://api.example.com/api/v1/bots/hooks/bh_example";

async function curlArguments(command: string): Promise<string[]> {
	// Replace curl locally so shell parsing can be checked without sending a request.
	const result = await promisify(execFile)("/bin/sh", [
		"-c",
		`curl() { printf '%s\\0' "$@"; }\n${command}`,
	]);
	return result.stdout.split("\0").slice(0, -1);
}

describe("bot webhook curl examples", () => {
	test("relative adapter paths keep their path and existing query", () => {
		expect(
			authenticatedWebhookUrl(
				"/bots/hooks/bh_example?wait=30#reply",
				"whsec_example",
			),
		).toBe("/bots/hooks/bh_example?wait=30&secret=whsec_example#reply");
	});
	test("encodes the URL secret while preserving wait and replacing an old secret", () => {
		const result = new URL(
			authenticatedWebhookUrl(`${url}?wait=30&secret=old`, "whsec_a+b&c=d"),
		);
		expect(result.searchParams.get("wait")).toBe("30");
		expect(result.searchParams.getAll("secret")).toEqual(["whsec_a+b&c=d"]);
	});

	test("the header request sends the exact secret and JSON without a signature", async () => {
		const secret = "whsec_quote' $(printf injected) `printf injected`";
		const arguments_ = await curlArguments(
			webhookCurlExamples(url, secret).header,
		);
		expect(arguments_).toEqual([
			"--fail-with-body",
			url,
			"--header",
			`X-Flow-Like-Webhook-Secret: ${secret}`,
			"--header",
			"Content-Type: application/json",
			"--data-raw",
			'{"event":"example","message":"Hello from a webhook"}',
		]);
	});

	test("the URL request carries its secret without authentication headers", async () => {
		const secret = "whsec_query&with+special=characters";
		const arguments_ = await curlArguments(
			webhookCurlExamples(`${url}?wait=30`, secret).url,
		);
		const target = new URL(arguments_[1]);
		expect(target.searchParams.get("secret")).toBe(secret);
		expect(target.searchParams.get("wait")).toBe("30");
		expect(
			arguments_.filter((argument) => argument === "--header"),
		).toHaveLength(1);
		expect(arguments_).toContain("Content-Type: application/json");
	});
});
