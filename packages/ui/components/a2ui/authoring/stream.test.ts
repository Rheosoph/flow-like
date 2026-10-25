import { describe, expect, test } from "bun:test";
import {
	type StreamSnapshot,
	createStreamSplitter,
	splitAuthoredStream,
} from "./stream";

const parsed = (snapshot: StreamSnapshot) =>
	snapshot.values.map((value) => value.value);

/** Push every prefix of `body` and check the values only ever grow, then match a one-shot split. */
function streamEveryPrefix(body: string): StreamSnapshot {
	const splitter = createStreamSplitter();
	let previous: StreamSnapshot = splitter.push("");
	for (let end = 1; end <= body.length; end++) {
		const snapshot = splitter.push(body.slice(0, end));
		expect(snapshot.values.length).toBeGreaterThanOrEqual(
			previous.values.length,
		);
		for (const [index, value] of previous.values.entries()) {
			expect(snapshot.values[index]).toBe(value);
		}
		previous = snapshot;
	}
	expect(parsed(previous)).toEqual(parsed(splitAuthoredStream(body)));
	return previous;
}

describe("stream splitter", () => {
	test("one object per line, at every chunk boundary", () => {
		const body = [
			'{"id":"root","type":"column","children":["a","b"]}',
			'{"id":"a","type":"text","content":"Brace } inside \\" a string {"}',
			'{"id":"b","type":"button","label":"Go [now]"}',
		].join("\n");
		const snapshot = streamEveryPrefix(body);
		expect(parsed(snapshot)).toEqual([
			{ id: "root", type: "column", children: ["a", "b"] },
			{ id: "a", type: "text", content: 'Brace } inside " a string {' },
			{ id: "b", type: "button", label: "Go [now]" },
		]);
		expect(snapshot.pending).toBe("");
	});

	test("an object is reported only once it closes", () => {
		const splitter = createStreamSplitter();
		const open = splitter.push('{"id":"a","type":"text","content":"Hel');
		expect(open.values).toHaveLength(0);
		expect(open.pending).toBe('{"id":"a","type":"text","content":"Hel');
		const closed = splitter.push('{"id":"a","type":"text","content":"Hello"}');
		expect(parsed(closed)).toEqual([
			{ id: "a", type: "text", content: "Hello" },
		]);
		expect(closed.pending).toBe("");
		expect(splitter.push('{"id":"a","type":"text","content":"Hello"}')).toBe(
			closed,
		);
	});

	test("objects spanning lines", () => {
		const body =
			'{\n  "id": "a",\n  "type": "divider"\n}\n{"id":"b","type":"spacer"}';
		expect(parsed(streamEveryPrefix(body))).toEqual([
			{ id: "a", type: "divider" },
			{ id: "b", type: "spacer" },
		]);
	});

	test("a wrapping array reports its objects one by one", () => {
		const body =
			'[\n {"id":"a","type":"divider"},\n {"id":"b","type":"spacer"}\n]';
		const splitter = createStreamSplitter();
		const partial = splitter.push(body.slice(0, body.indexOf("{", 5)));
		expect(parsed(partial)).toEqual([{ id: "a", type: "divider" }]);
		expect(parsed(streamEveryPrefix(body))).toEqual([
			{ id: "a", type: "divider" },
			{ id: "b", type: "spacer" },
		]);
	});

	test('a {"components":[...]} envelope reports its components', () => {
		const body =
			'{"rootComponentId":"root","components":[{"id":"root","type":"column"},{"id":"x","type":"divider"}]}';
		const splitter = createStreamSplitter();
		const partial = splitter.push(body.slice(0, body.indexOf(',{"id":"x"')));
		expect(parsed(partial)).toEqual([{ id: "root", type: "column" }]);
		expect(parsed(streamEveryPrefix(body))).toEqual([
			{ id: "root", type: "column" },
			{ id: "x", type: "divider" },
		]);
	});

	test("an object with a nested components key is not an envelope", () => {
		const body = '{"id":"t","type":"table","data":{"components":[1,2]}}';
		expect(parsed(splitAuthoredStream(body))).toEqual([
			{ id: "t", type: "table", data: { components: [1, 2] } },
		]);
	});

	test("a line cut off mid-string is abandoned without losing the next line", () => {
		const body =
			'{"id":"a","type":"text","content":"cut\n{"id":"b","type":"divider"}';
		const snapshot = streamEveryPrefix(body);
		expect(parsed(snapshot)).toEqual([{ id: "b", type: "divider" }]);
		expect(snapshot.malformed).toBe(1);
	});

	test("a mismatched bracket abandons the value", () => {
		const snapshot = splitAuthoredStream(
			'{"id":"a","children":["x"}\n{"id":"b","type":"divider"}',
		);
		expect(parsed(snapshot)).toEqual([{ id: "b", type: "divider" }]);
		expect(snapshot.malformed).toBe(1);
	});

	test("stray text, fence backticks and closers between objects are ignored", () => {
		const body =
			'Here you go:\n{"id":"a","type":"divider"}\n] } "quoted {" 42\n{"id":"b","type":"spacer"}\n``';
		expect(parsed(streamEveryPrefix(body))).toEqual([
			{ id: "a", type: "divider" },
			{ id: "b", type: "spacer" },
		]);
	});

	test("invalid JSON is reported with an error; trailing commas are repaired", () => {
		const snapshot = splitAuthoredStream(
			'{"id":"a","type":"row","children":["x","y",],}\n{"id":b}',
		);
		expect(snapshot.values[0]?.value).toEqual({
			id: "a",
			type: "row",
			children: ["x", "y"],
		});
		expect(snapshot.values[1]?.value).toBeUndefined();
		expect(snapshot.values[1]?.error).toBeString();
	});

	test("a body that does not extend the previous one restarts", () => {
		const splitter = createStreamSplitter();
		splitter.push('{"id":"a","type":"divider"}');
		const restarted = splitter.push('{"id":"z","type":"spacer"}');
		expect(parsed(restarted)).toEqual([{ id: "z", type: "spacer" }]);
	});

	test("escaped backslashes before a quote close the string", () => {
		const body =
			'{"id":"a","type":"text","content":"C:\\\\"}\n{"id":"b","type":"divider"}';
		expect(parsed(streamEveryPrefix(body))).toEqual([
			{ id: "a", type: "text", content: "C:\\" },
			{ id: "b", type: "divider" },
		]);
	});
});
