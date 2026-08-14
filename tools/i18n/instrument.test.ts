import { describe, expect, test } from "bun:test";
import { isProse } from "./instrument";

describe("instrument prose classifier", () => {
	test.each([
		"{{appId}}:{{boardId}}",
		"{{STORAGE_PREFIX}}{{eventId}}",
		"report-{{stamp}}-{{nonce}}",
		"{{baseUrl}}/store?id={{appId}}",
		"rgb({{red}},{{green}},{{blue}})",
		"CAST(NULL AS {{sqlType}})",
		"CAST({{literal}} AS {{sqlType}})",
		"has_{{api_name}}",
		"page-{{pageId}}",
		"[E2E {{stamp}} {{caseId}} {{nonce}}]",
		"draftPending={{value}}",
		"{{hours}}h {{minutes}}m",
		'<link rel="stylesheet" href="{{siteUrl}}/tailwind.css">',
		"Bearer {{authToken}}",
		"Authorization: Bearer {{authToken}}",
		"{{value}}px",
		"```sh\nbrew install ngrok\n```",
		"brew install ngrok",
		"winget install ngrok.ngrok",
		"choco install ngrok",
		"Authorization: Bearer token",
	])("rejects behavior-sensitive interpolated values: %s", (value) => {
		expect(isProse(value)).toBe(false);
	});

	test.each([
		"{{count}} packages updated",
		"{{code}} collection failed: {{message}}",
		"{{label}} upload failed ({{status}}): {{message}}",
		"CLI repeat must be an integer from 1 to {{maximum}}.",
		"This preset installs model weights from {{modelId}}.",
		"At {{hour}}:{{minute}} every day{{timezone}}",
		"in {{minutes}}m",
	])("keeps human-facing interpolated prose: %s", (value) => {
		expect(isProse(value)).toBe(true);
	});
});
