import { describe, expect, test } from "bun:test";
import { ILogLevel } from "../../../lib/schema/flow/log";
import { formatJson, formatMarkdown, formatText } from "./copy-format";
import { formatAbsolute } from "./log-format";
import { makeLog } from "./test-fixtures";

const T0 = new Date(2026, 8, 25, 14, 32, 7, 121).getTime() * 1_000;
const T1 = new Date(2026, 8, 25, 14, 32, 9, 16).getTime() * 1_000;

const http = makeLog({
	log_level: ILogLevel.Info,
	node_id: "http",
	operation_id: "op_7Hq2",
	message: "POST https://overpass-api.de\nstatus: 200 OK",
	start: T0,
	end: T0 + 1_840_000,
	stats: { token_in: 12, token_out: 3 },
});
const upsert = makeLog({
	log_level: ILogLevel.Error,
	node_id: "upsert",
	message: "Failed to execute node: invalid type: null",
	start: T1,
});
const print = makeLog({
	log_level: ILogLevel.Info,
	message: '{"type":"way","id":1}',
	start: T1,
});

describe("formatText", () => {
	test("one line per log with time, level and node", () => {
		expect(
			formatText([
				{ log: upsert, node: "Upsert", repeat: 986 },
				{ log: print },
			]),
		).toBe(
			`${formatAbsolute(T1)} ERROR [Upsert] Failed to execute node: invalid type: null (×986)\n${formatAbsolute(T1)} INFO  {"type":"way","id":1}`,
		);
		expect(formatAbsolute(T1)).toBe("14:32:09.016");
	});
});

describe("formatJson", () => {
	test("an array of plain objects", () => {
		const parsed = JSON.parse(
			formatJson([{ log: http, node: "HTTP Request" }]),
		);
		expect(parsed).toEqual([
			{
				time: "14:32:07.121",
				level: "INFO",
				node: "HTTP Request",
				node_id: "http",
				message: "POST https://overpass-api.de\nstatus: 200 OK",
				start: T0,
				end: T0 + 1_840_000,
				operation_id: "op_7Hq2",
				token_in: 12,
				token_out: 3,
			},
		]);
	});
});

describe("formatMarkdown", () => {
	test("fences multi-line and JSON messages, inlines the rest", () => {
		const md = formatMarkdown(
			[
				{ log: http, node: "HTTP Request" },
				{ log: print, node: "Print" },
				{ log: upsert, node: "Upsert", repeat: 986 },
			],
			"Retrieve Entities In Area",
		);
		expect(md).toBe(
			[
				"### Retrieve Entities In Area",
				"",
				"- `14:32:07.121` **INFO** HTTP Request · 1.84 s",
				"  ```text",
				"  POST https://overpass-api.de",
				"  status: 200 OK",
				"  ```",
				"- `14:32:09.016` **INFO** Print",
				"  ```json",
				"  {",
				'    "type": "way",',
				'    "id": 1',
				"  }",
				"  ```",
				"- `14:32:09.016` **ERROR** Upsert · ×986",
				"  `Failed to execute node: invalid type: null`",
			].join("\n"),
		);
	});

	test("backticks in a message never break the code span", () => {
		const log = makeLog({ message: "use `x` here", start: T1 });
		expect(formatMarkdown([{ log }])).toBe(
			"- `14:32:09.016` **INFO**\n  ``use `x` here``",
		);
	});
});
