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
		// Use a simple line-by-line approach for flat/nested YAML
		const lines = input.split("\n");
		const result: Record<string, unknown> = {};
		const stack: { indent: number; obj: Record<string, unknown> }[] = [
			{ indent: -1, obj: result },
		];

		for (const rawLine of lines) {
			const line = rawLine.replace(/\r$/, "");
			if (line.trim() === "" || line.trim().startsWith("#")) continue;

			const indent = line.length - line.trimStart().length;
			const trimmedLine = line.trim();

			// Pop stack to find parent at correct indent
			while (stack.length > 1 && stack[stack.length - 1].indent >= indent) {
				stack.pop();
			}
			const parent = stack[stack.length - 1].obj;

			// Array item
			if (trimmedLine.startsWith("- ")) {
				const value = trimmedLine.slice(2).trim();
				// Find which key this array belongs to
				const lastKey = Object.keys(parent).pop();
				if (lastKey && Array.isArray(parent[lastKey])) {
					if (value.includes(":")) {
						const obj: Record<string, unknown> = {};
						const [k, ...rest] = value.split(":");
						obj[k.trim()] = parseYamlValue(rest.join(":").trim());
						(parent[lastKey] as unknown[]).push(obj);
						stack.push({ indent, obj });
					} else {
						(parent[lastKey] as unknown[]).push(parseYamlValue(value));
					}
				}
				continue;
			}

			// Key: value pair
			const colonIndex = trimmedLine.indexOf(":");
			if (colonIndex > 0) {
				const key = trimmedLine.slice(0, colonIndex).trim();
				const rawValue = trimmedLine.slice(colonIndex + 1).trim();

				if (rawValue === "" || rawValue === "|" || rawValue === ">") {
					// Nested object or block scalar - create empty object for now
					const child: Record<string, unknown> = {};
					parent[key] = child;
					stack.push({ indent, obj: child });
				} else if (rawValue === "[]") {
					parent[key] = [];
				} else {
					parent[key] = parseYamlValue(rawValue);
				}
			}
		}

		return Object.keys(result).length > 0 ? result : null;
	} catch {
		return null;
	}
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
