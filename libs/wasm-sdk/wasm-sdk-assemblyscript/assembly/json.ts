import { ExecutionInput } from "./types";

export function escapeJsonString(s: string): string {
	let result = "";
	for (let i = 0; i < s.length; i++) {
		const c = s.charCodeAt(i);
		if (c == 34) result += '\\"';
		else if (c == 92) result += "\\\\";
		else if (c == 10) result += "\\n";
		else if (c == 13) result += "\\r";
		else if (c == 9) result += "\\t";
		else if (c < 32) {
			const hex = c.toString(16);
			result += "\\u" + "0000".substr(0, 4 - hex.length) + hex;
		} else result += String.fromCharCode(c);
	}
	return result;
}

export function jsonString(s: string): string {
	return '"' + escapeJsonString(s) + '"';
}

export function isWhitespace(c: i32): bool {
	return c == 32 || c == 9 || c == 10 || c == 13;
}

export function extractStringValue(json: string, key: string): string {
	const keyStr = '"' + key + '"';
	const idx = json.indexOf(keyStr);
	if (idx < 0) return "";
	let i = idx + keyStr.length;
	while (
		i < json.length &&
		(isWhitespace(json.charCodeAt(i)) || json.charCodeAt(i) == 58)
	)
		i++;
	if (json.charCodeAt(i) != 34) return "";
	i++;
	const start = i;
	while (i < json.length && json.charCodeAt(i) != 34) i++;
	return json.substring(start, i);
}

export function extractBoolValue(json: string, key: string): bool {
	const keyStr = '"' + key + '"';
	const idx = json.indexOf(keyStr);
	if (idx < 0) return false;
	let i = idx + keyStr.length;
	while (
		i < json.length &&
		(isWhitespace(json.charCodeAt(i)) || json.charCodeAt(i) == 58)
	)
		i++;
	return json.substr(i, 4) == "true";
}

export function extractIntValue(json: string, key: string): i32 {
	const keyStr = '"' + key + '"';
	const idx = json.indexOf(keyStr);
	if (idx < 0) return 0;
	let i = idx + keyStr.length;
	while (
		i < json.length &&
		(isWhitespace(json.charCodeAt(i)) || json.charCodeAt(i) == 58)
	)
		i++;
	let num = 0;
	let neg = false;
	if (json.charCodeAt(i) == 45) {
		neg = true;
		i++;
	}
	while (i < json.length) {
		const c = json.charCodeAt(i);
		if (c < 48 || c > 57) break;
		num = num * 10 + (c - 48);
		i++;
	}
	return neg ? -num : num;
}

export function parseInputsObject(
	json: string,
	map: Map<string, string>,
): void {
	let i = 1;
	while (i < json.length - 1) {
		while (i < json.length && isWhitespace(json.charCodeAt(i))) i++;
		if (i >= json.length - 1 || json.charCodeAt(i) == 125) break;
		if (json.charCodeAt(i) != 34) {
			i++;
			continue;
		}
		const keyStart = i + 1;
		i++;
		while (i < json.length && json.charCodeAt(i) != 34) i++;
		const key = json.substring(keyStart, i);
		i++;

		while (
			i < json.length &&
			(isWhitespace(json.charCodeAt(i)) || json.charCodeAt(i) == 58)
		)
			i++;

		const valueStart = i;
		if (json.charCodeAt(i) == 34) {
			i++;
			while (i < json.length) {
				if (json.charCodeAt(i) == 34 && json.charCodeAt(i - 1) != 92) break;
				i++;
			}
			i++;
		} else if (json.charCodeAt(i) == 123) {
			let depth = 1;
			i++;
			while (depth > 0 && i < json.length) {
				if (json.charCodeAt(i) == 123) depth++;
				else if (json.charCodeAt(i) == 125) depth--;
				i++;
			}
		} else if (json.charCodeAt(i) == 91) {
			let depth = 1;
			i++;
			while (depth > 0 && i < json.length) {
				if (json.charCodeAt(i) == 91) depth++;
				else if (json.charCodeAt(i) == 93) depth--;
				i++;
			}
		} else {
			while (
				i < json.length &&
				!isWhitespace(json.charCodeAt(i)) &&
				json.charCodeAt(i) != 44 &&
				json.charCodeAt(i) != 125
			)
				i++;
		}
		const value = json.substring(valueStart, i);
		map.set(key, value);

		while (
			i < json.length &&
			(isWhitespace(json.charCodeAt(i)) || json.charCodeAt(i) == 44)
		)
			i++;
	}
}

export function parseExecutionInputJson(json: string): ExecutionInput {
	const input = new ExecutionInput();

	const inputsStart = json.indexOf('"inputs"');
	if (inputsStart >= 0) {
		const objStart = json.indexOf("{", inputsStart + 8);
		if (objStart >= 0) {
			let depth = 1;
			let objEnd = objStart + 1;
			while (depth > 0 && objEnd < json.length) {
				const c = json.charCodeAt(objEnd);
				if (c == 123) depth++;
				else if (c == 125) depth--;
				objEnd++;
			}
			const inputsStr = json.substring(objStart, objEnd);
			parseInputsObject(inputsStr, input.inputs);
		}
	}

	input.node_id = extractStringValue(json, "node_id");
	input.node_name = extractStringValue(json, "node_name");
	input.run_id = extractStringValue(json, "run_id");
	input.app_id = extractStringValue(json, "app_id");
	input.board_id = extractStringValue(json, "board_id");
	input.user_id = extractStringValue(json, "user_id");
	input.stream_state = extractBoolValue(json, "stream_state");
	input.log_level = u8(extractIntValue(json, "log_level"));

	return input;
}

// ============================================================================
// JSON Array/Map Serialization
// ============================================================================

export function serializeStringArray(values: string[]): string {
	let json = "[";
	for (let i = 0; i < values.length; i++) {
		if (i > 0) json += ",";
		json += jsonString(values[i]);
	}
	return json + "]";
}

export function serializeI64Array(values: i64[]): string {
	let json = "[";
	for (let i = 0; i < values.length; i++) {
		if (i > 0) json += ",";
		json += values[i].toString();
	}
	return json + "]";
}

export function serializeF64Array(values: f64[]): string {
	let json = "[";
	for (let i = 0; i < values.length; i++) {
		if (i > 0) json += ",";
		json += values[i].toString();
	}
	return json + "]";
}

export function serializeBoolArray(values: bool[]): string {
	let json = "[";
	for (let i = 0; i < values.length; i++) {
		if (i > 0) json += ",";
		json += values[i] ? "true" : "false";
	}
	return json + "]";
}

export function serializeStringMap(map: Map<string, string>): string {
	let json = "{";
	const keys = map.keys();
	for (let i = 0; i < keys.length; i++) {
		if (i > 0) json += ",";
		json += jsonString(keys[i]) + ":" + jsonString(map.get(keys[i]));
	}
	return json + "}";
}

// ============================================================================
// JSON Array/Map Parsing
// ============================================================================

export function parseJsonStringArray(json: string): string[] {
	const result: string[] = [];
	const trimmed = json.trim();
	if (trimmed.length < 2 || trimmed.charCodeAt(0) != 91) return result;
	let i = 1;
	while (i < trimmed.length - 1) {
		while (i < trimmed.length && isWhitespace(trimmed.charCodeAt(i))) i++;
		if (i >= trimmed.length - 1 || trimmed.charCodeAt(i) == 93) break;
		if (trimmed.charCodeAt(i) == 44) {
			i++;
			continue;
		}
		if (trimmed.charCodeAt(i) == 34) {
			i++;
			let s = "";
			while (i < trimmed.length) {
				const c = trimmed.charCodeAt(i);
				if (c == 92 && i + 1 < trimmed.length) {
					const nc = trimmed.charCodeAt(i + 1);
					if (nc == 34) {
						s += '"';
						i += 2;
					} else if (nc == 92) {
						s += "\\";
						i += 2;
					} else if (nc == 110) {
						s += "\n";
						i += 2;
					} else if (nc == 114) {
						s += "\r";
						i += 2;
					} else if (nc == 116) {
						s += "\t";
						i += 2;
					} else {
						s += String.fromCharCode(c);
						i++;
					}
				} else if (c == 34) {
					i++;
					break;
				} else {
					s += String.fromCharCode(c);
					i++;
				}
			}
			result.push(s);
		} else {
			i++;
		}
	}
	return result;
}

export function parseJsonI64Array(json: string): i64[] {
	const result: i64[] = [];
	const trimmed = json.trim();
	if (trimmed.length < 2 || trimmed.charCodeAt(0) != 91) return result;
	let i = 1;
	while (i < trimmed.length - 1) {
		while (
			i < trimmed.length &&
			(isWhitespace(trimmed.charCodeAt(i)) || trimmed.charCodeAt(i) == 44)
		)
			i++;
		if (i >= trimmed.length - 1 || trimmed.charCodeAt(i) == 93) break;
		const start = i;
		while (
			i < trimmed.length &&
			!isWhitespace(trimmed.charCodeAt(i)) &&
			trimmed.charCodeAt(i) != 44 &&
			trimmed.charCodeAt(i) != 93
		)
			i++;
		result.push(I64.parseInt(trimmed.substring(start, i)));
	}
	return result;
}

export function parseJsonF64Array(json: string): f64[] {
	const result: f64[] = [];
	const trimmed = json.trim();
	if (trimmed.length < 2 || trimmed.charCodeAt(0) != 91) return result;
	let i = 1;
	while (i < trimmed.length - 1) {
		while (
			i < trimmed.length &&
			(isWhitespace(trimmed.charCodeAt(i)) || trimmed.charCodeAt(i) == 44)
		)
			i++;
		if (i >= trimmed.length - 1 || trimmed.charCodeAt(i) == 93) break;
		const start = i;
		while (
			i < trimmed.length &&
			!isWhitespace(trimmed.charCodeAt(i)) &&
			trimmed.charCodeAt(i) != 44 &&
			trimmed.charCodeAt(i) != 93
		)
			i++;
		result.push(F64.parseFloat(trimmed.substring(start, i)));
	}
	return result;
}

export function parseJsonBoolArray(json: string): bool[] {
	const result: bool[] = [];
	const trimmed = json.trim();
	if (trimmed.length < 2 || trimmed.charCodeAt(0) != 91) return result;
	let i = 1;
	while (i < trimmed.length - 1) {
		while (
			i < trimmed.length &&
			(isWhitespace(trimmed.charCodeAt(i)) || trimmed.charCodeAt(i) == 44)
		)
			i++;
		if (i >= trimmed.length - 1 || trimmed.charCodeAt(i) == 93) break;
		const start = i;
		while (
			i < trimmed.length &&
			!isWhitespace(trimmed.charCodeAt(i)) &&
			trimmed.charCodeAt(i) != 44 &&
			trimmed.charCodeAt(i) != 93
		)
			i++;
		result.push(trimmed.substring(start, i) == "true");
	}
	return result;
}

export function parseJsonStringMap(json: string): Map<string, string> {
	const map = new Map<string, string>();
	const trimmed = json.trim();
	if (trimmed.length < 2 || trimmed.charCodeAt(0) != 123) return map;
	parseInputsObject(trimmed, map);
	// Unwrap quoted values
	const keys = map.keys();
	for (let i = 0; i < keys.length; i++) {
		const k = keys[i];
		const v = map.get(k);
		if (v.startsWith('"') && v.endsWith('"')) {
			map.set(k, v.slice(1, v.length - 1));
		}
	}
	return map;
}
