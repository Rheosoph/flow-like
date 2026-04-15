import type { DifyWorkflow, ImportFormat, N8nWorkflow } from "./types";

export function detectFormat(input: string): {
	format: ImportFormat;
	parsed: N8nWorkflow | DifyWorkflow | null;
	error?: string;
} {
	const trimmed = input.trim();

	// Try JSON first
	if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
		try {
			const parsed = JSON.parse(trimmed);
			return classifyParsed(parsed);
		} catch {
			return { format: "unknown", parsed: null, error: "Invalid JSON" };
		}
	}

	// Try YAML (Dify exports as YAML)
	try {
		const parsed = parseSimpleYaml(trimmed);
		if (parsed && typeof parsed === "object") {
			return classifyParsed(parsed);
		}
	} catch {
		// Not valid YAML either
	}

	return {
		format: "unknown",
		parsed: null,
		error: "Could not parse as JSON or YAML",
	};
}

function classifyParsed(obj: Record<string, unknown>): {
	format: ImportFormat;
	parsed: N8nWorkflow | DifyWorkflow | null;
} {
	// n8n: has "nodes" array and "connections" object
	if (
		Array.isArray(obj.nodes) &&
		obj.connections &&
		typeof obj.connections === "object"
	) {
		const firstNode = obj.nodes[0];
		if (
			firstNode &&
			typeof firstNode === "object" &&
			"type" in (firstNode as Record<string, unknown>)
		) {
			const nodeType = (firstNode as Record<string, unknown>).type;
			if (typeof nodeType === "string" && nodeType.startsWith("n8n-nodes")) {
				return { format: "n8n", parsed: obj as unknown as N8nWorkflow };
			}
		}
		// Still likely n8n if it has connections keyed by node names
		if (obj.connections && !obj.workflow) {
			return { format: "n8n", parsed: obj as unknown as N8nWorkflow };
		}
	}

	// Dify: has "app" object with "mode" and "workflow" with "graph"
	if (
		obj.app &&
		typeof obj.app === "object" &&
		obj.workflow &&
		typeof obj.workflow === "object"
	) {
		const app = obj.app as Record<string, unknown>;
		const workflow = obj.workflow as Record<string, unknown>;
		if (
			(app.mode === "workflow" || app.mode === "advanced-chat") &&
			workflow.graph &&
			typeof workflow.graph === "object"
		) {
			return { format: "dify", parsed: obj as unknown as DifyWorkflow };
		}
	}

	// Dify YAML: may have "kind: app" at top level
	if (obj.kind === "app" && obj.app && obj.workflow) {
		return { format: "dify", parsed: obj as unknown as DifyWorkflow };
	}

	// Fallback heuristics
	if (Array.isArray(obj.nodes) && obj.connections) {
		return { format: "n8n", parsed: obj as unknown as N8nWorkflow };
	}

	return { format: "unknown", parsed: null };
}

/**
 * Minimal YAML parser supporting the subset needed for Dify DSL.
 * Handles nested objects, arrays, and scalar values.
 * For production use, consider a proper YAML library.
 */
function parseSimpleYaml(input: string): Record<string, unknown> | null {
	try {
		const lines = input.split("\n");
		const startIndex = nextMeaningfulLine(lines, 0);
		if (startIndex >= lines.length) return null;

		const [parsed] = parseYamlBlock(
			lines,
			startIndex,
			getIndent(lines[startIndex]),
		);
		if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
			return null;
		}

		return Object.keys(parsed).length > 0 ? parsed : null;
	} catch {
		return null;
	}
}

function parseYamlBlock(
	lines: string[],
	startIndex: number,
	indent: number,
): [unknown, number] {
	if (startIndex >= lines.length) return [{}, startIndex];
	return lines[startIndex].trim().startsWith("- ")
		? parseYamlArray(lines, startIndex, indent)
		: parseYamlObject(lines, startIndex, indent);
}

function parseYamlObject(
	lines: string[],
	startIndex: number,
	indent: number,
): [Record<string, unknown>, number] {
	const result: Record<string, unknown> = {};
	let index = startIndex;

	while (index < lines.length) {
		index = nextMeaningfulLine(lines, index);
		if (index >= lines.length) break;

		const line = lines[index];
		const lineIndent = getIndent(line);
		if (lineIndent < indent) break;
		if (lineIndent !== indent || line.trim().startsWith("- ")) break;

		const trimmedLine = line.trim();
		const colonIndex = trimmedLine.indexOf(":");
		if (colonIndex <= 0) {
			index += 1;
			continue;
		}

		const key = trimmedLine.slice(0, colonIndex).trim();
		const rawValue = trimmedLine.slice(colonIndex + 1).trim();

		if (rawValue === "[]") {
			result[key] = [];
			index += 1;
			continue;
		}

		if (rawValue === "|" || rawValue === ">") {
			const [blockValue, nextIndex] = parseYamlBlockScalar(
				lines,
				index + 1,
				lineIndent,
				rawValue,
			);
			result[key] = blockValue;
			index = nextIndex;
			continue;
		}

		if (rawValue === "") {
			const nextIndex = nextMeaningfulLine(lines, index + 1);
			if (nextIndex < lines.length && getIndent(lines[nextIndex]) > lineIndent) {
				const [child, afterChild] = parseYamlBlock(
					lines,
					nextIndex,
					getIndent(lines[nextIndex]),
				);
				result[key] = child;
				index = afterChild;
				continue;
			}

			result[key] = {};
			index += 1;
			continue;
		}

		result[key] = parseYamlValue(rawValue);
		index += 1;
	}

	return [result, index];
}

function parseYamlArray(
	lines: string[],
	startIndex: number,
	indent: number,
): [unknown[], number] {
	const result: unknown[] = [];
	let index = startIndex;

	while (index < lines.length) {
		index = nextMeaningfulLine(lines, index);
		if (index >= lines.length) break;

		const line = lines[index];
		const lineIndent = getIndent(line);
		const trimmedLine = line.trim();
		if (lineIndent < indent || lineIndent !== indent || !trimmedLine.startsWith("- ")) {
			break;
		}

		const value = trimmedLine.slice(2).trim();
		if (value === "") {
			const nextIndex = nextMeaningfulLine(lines, index + 1);
			if (nextIndex < lines.length && getIndent(lines[nextIndex]) > lineIndent) {
				const [child, afterChild] = parseYamlBlock(
					lines,
					nextIndex,
					getIndent(lines[nextIndex]),
				);
				result.push(child);
				index = afterChild;
				continue;
			}

			result.push(null);
			index += 1;
			continue;
		}

		if (value === "|" || value === ">") {
			const [blockValue, nextIndex] = parseYamlBlockScalar(
				lines,
				index + 1,
				lineIndent,
				value,
			);
			result.push(blockValue);
			index = nextIndex;
			continue;
		}

		if (value.includes(":")) {
			const [child, afterChild] = parseYamlArrayObjectItem(
				lines,
				index,
				indent,
			);
			result.push(child);
			index = afterChild;
			continue;
		}

		result.push(parseYamlValue(value));
		index += 1;
	}

	return [result, index];
}

function parseYamlArrayObjectItem(
	lines: string[],
	startIndex: number,
	indent: number,
): [Record<string, unknown>, number] {
	const line = lines[startIndex].trim().slice(2).trim();
	const colonIndex = line.indexOf(":");
	const key = line.slice(0, colonIndex).trim();
	const rawValue = line.slice(colonIndex + 1).trim();
	const result: Record<string, unknown> = {};
	let index = startIndex + 1;

	if (rawValue === "|" || rawValue === ">") {
		const [blockValue, nextIndex] = parseYamlBlockScalar(
			lines,
			index,
			indent,
			rawValue,
		);
		result[key] = blockValue;
		index = nextIndex;
	} else if (rawValue === "") {
		const nextIndex = nextMeaningfulLine(lines, index);
		if (nextIndex < lines.length && getIndent(lines[nextIndex]) > indent) {
			const [child, afterChild] = parseYamlBlock(
				lines,
				nextIndex,
				getIndent(lines[nextIndex]),
			);
			result[key] = child;
			index = afterChild;
		} else {
			result[key] = {};
		}
	} else {
		result[key] = rawValue === "[]" ? [] : parseYamlValue(rawValue);
	}

	const nextIndex = nextMeaningfulLine(lines, index);
	if (nextIndex < lines.length && getIndent(lines[nextIndex]) > indent) {
		const [rest, afterRest] = parseYamlObject(
			lines,
			nextIndex,
			getIndent(lines[nextIndex]),
		);
		Object.assign(result, rest);
		index = afterRest;
	}

	return [result, index];
}

function parseYamlBlockScalar(
	lines: string[],
	startIndex: number,
	parentIndent: number,
	style: "|" | ">",
): [string, number] {
	const values: string[] = [];
	let index = startIndex;
	let contentIndent: number | null = null;

	while (index < lines.length) {
		const rawLine = lines[index].replace(/\r$/, "");
		const trimmedLine = rawLine.trim();
		const lineIndent = getIndent(rawLine);
		const minimumIndent = contentIndent ?? parentIndent + 1;

		if (trimmedLine !== "" && lineIndent < minimumIndent) break;
		if (trimmedLine === "") {
			values.push("");
			index += 1;
			continue;
		}

		contentIndent ??= lineIndent;
		values.push(rawLine.slice(contentIndent));
		index += 1;
	}

	const text = values.join("\n");
	return [style === ">" ? text.replace(/([^\n])\n(?=[^\n])/g, "$1 ") : text, index];
}

function nextMeaningfulLine(lines: string[], startIndex: number): number {
	let index = startIndex;
	while (index < lines.length) {
		const line = lines[index].replace(/\r$/, "");
		if (line.trim() !== "" && !line.trim().startsWith("#")) break;
		index += 1;
	}
	return index;
}

function getIndent(line: string): number {
	return line.length - line.trimStart().length;
}

function parseYamlValue(value: string): unknown {
	if (value === "true") return true;
	if (value === "false") return false;
	if (value === "null" || value === "~") return null;
	if (/^-?\d+$/.test(value)) return Number.parseInt(value, 10);
	if (/^-?\d+\.\d+$/.test(value)) return Number.parseFloat(value);
	// Remove surrounding quotes
	if (
		(value.startsWith('"') && value.endsWith('"')) ||
		(value.startsWith("'") && value.endsWith("'"))
	) {
		return value.slice(1, -1);
	}
	return value;
}
