/**
 * Incremental splitter for the body of a streaming ```` ```a2ui ```` fence.
 *
 * The body grows by appending, so each push scans only the new characters and
 * reports every top-level JSON object that has closed so far, exactly once and
 * in order. The scanner is string- and escape-aware and tolerates the shapes
 * models actually write: one object per line, objects spanning lines, a wrapping
 * `[...]` array and a `{"components":[...]}` envelope. A raw newline inside a
 * string or a mismatched bracket abandons the value it belongs to, so one broken
 * line never swallows the lines after it.
 */

export interface StreamValue {
	/** Exact source text of the value; stable for the lifetime of the stream. */
	readonly text: string;
	/** Offset of the value's opening brace in the body. */
	readonly start: number;
	/** Parsed JSON, or `undefined` when `error` is set. */
	readonly value: unknown;
	readonly error?: string;
}

export interface StreamSnapshot {
	/** Every closed top-level object, in source order. Append-only across pushes. */
	readonly values: readonly StreamValue[];
	/** Text of the object that has opened but not closed yet ("" when none). */
	readonly pending: string;
	/** Objects abandoned before they closed (malformed lines). */
	readonly malformed: number;
}

export interface StreamSplitter {
	/** Advance to `body`. A body that does not extend the previous one restarts the scan. */
	push(body: string): StreamSnapshot;
}

interface Frame {
	readonly type: "{" | "[";
	readonly start: number;
	/** Report this value when it closes. */
	emit: boolean;
	/** Objects directly inside are reported (wrapping array, `components` array). */
	readonly container: boolean;
	/** Objects expect a key after `{` and `,`. */
	expectKey: boolean;
	/** Last key read, tracked for top-level objects only. */
	key: string | undefined;
}

const ENVELOPE_KEY = "components";

export function createStreamSplitter(): StreamSplitter {
	let source = "";
	let cursor = 0;
	let stack: Frame[] = [];
	let inString = false;
	let escaped = false;
	let stringStart = -1;
	let values: StreamValue[] = [];
	let malformed = 0;
	let snapshot: StreamSnapshot = { values, pending: "", malformed };

	const reset = () => {
		source = "";
		cursor = 0;
		stack = [];
		inString = false;
		escaped = false;
		stringStart = -1;
		values = [];
		malformed = 0;
		snapshot = { values, pending: "", malformed };
	};

	const abandon = () => {
		if (stack.some((frame) => frame.emit)) malformed++;
		stack = [];
		inString = false;
		escaped = false;
	};

	const open = (type: "{" | "[", index: number) => {
		const parent = stack[stack.length - 1];
		let emit = false;
		let container = false;
		if (!parent) {
			emit = type === "{";
			container = type === "[";
		} else if (parent.container) {
			emit = type === "{";
		} else if (
			type === "[" &&
			stack.length === 1 &&
			parent.type === "{" &&
			!parent.expectKey &&
			parent.key === ENVELOPE_KEY
		) {
			parent.emit = false;
			container = true;
		}
		stack.push({
			type,
			start: index,
			emit,
			container,
			expectKey: type === "{",
			key: undefined,
		});
	};

	const close = (type: "}" | "]", index: number, emitted: StreamValue[]) => {
		const frame = stack[stack.length - 1];
		if (!frame) return;
		if ((type === "}") !== (frame.type === "{")) {
			abandon();
			return;
		}
		stack.pop();
		if (frame.emit) {
			emitted.push(
				parseValue(source.slice(frame.start, index + 1), frame.start),
			);
		}
	};

	const closeString = (index: number) => {
		inString = false;
		const frame = stack[stack.length - 1];
		if (frame?.type === "{" && frame.expectKey && stack.length === 1) {
			frame.key = source.slice(stringStart + 1, index);
		}
	};

	const scanStringChar = (char: string, index: number) => {
		if (escaped) escaped = false;
		else if (char === "\\") escaped = true;
		else if (char === "\n") abandon();
		else if (char === '"') closeString(index);
	};

	const separate = (expectKey: boolean) => {
		const frame = stack[stack.length - 1];
		if (frame?.type !== "{") return;
		frame.expectKey = expectKey;
		if (expectKey) frame.key = undefined;
	};

	const scanStructureChar = (
		char: string,
		index: number,
		emitted: StreamValue[],
	) => {
		switch (char) {
			case '"':
				inString = true;
				stringStart = index;
				return;
			case "{":
			case "[":
				open(char, index);
				return;
			case "}":
			case "]":
				close(char, index, emitted);
				return;
			case ":":
			case ",":
				separate(char === ",");
		}
	};

	const scan = (emitted: StreamValue[]) => {
		for (let index = cursor; index < source.length; index++) {
			const char = source[index];
			if (inString) scanStringChar(char, index);
			else scanStructureChar(char, index, emitted);
		}
		cursor = source.length;
	};

	return {
		push(body) {
			if (body === source) return snapshot;
			if (!body.startsWith(source)) reset();
			source = body;
			const emitted: StreamValue[] = [];
			const malformedBefore = malformed;
			scan(emitted);
			if (emitted.length > 0) values = [...values, ...emitted];
			const pendingFrame = stack.find((frame) => frame.emit);
			const pending = pendingFrame ? source.slice(pendingFrame.start) : "";
			if (
				emitted.length > 0 ||
				malformed !== malformedBefore ||
				pending !== snapshot.pending
			) {
				snapshot = { values, pending, malformed };
			}
			return snapshot;
		},
	};
}

/** Split a complete body in one pass. */
export function splitAuthoredStream(body: string): StreamSnapshot {
	return createStreamSplitter().push(body);
}

function parseValue(text: string, start: number): StreamValue {
	try {
		return { text, start, value: JSON.parse(text) };
	} catch (error) {
		const repaired = withoutTrailingCommas(text);
		if (repaired !== text) {
			try {
				return { text, start, value: JSON.parse(repaired) };
			} catch {}
		}
		return {
			text,
			start,
			value: undefined,
			error: error instanceof Error ? error.message : String(error),
		};
	}
}

const STRING_OR_TRAILING_COMMA = /"(?:[^"\\]|\\.)*"|,(\s*[}\]])/g;

/** Drop `,` directly before a closing bracket (outside strings), the one slip models make most. */
function withoutTrailingCommas(text: string) {
	return text.replace(
		STRING_OR_TRAILING_COMMA,
		(match, closer: string | undefined) => closer ?? match,
	);
}
