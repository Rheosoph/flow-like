import type {
	WasmNodeDefinition,
	WasmPinDefinition,
} from "@tm9657/flow-like-ui/lib/schema/developer";

export type LintSeverity = "error" | "warning" | "info";

export interface LintIssue {
	severity: LintSeverity;
	message: string;
	nodeName: string;
	nodeIndex: number;
	pinName?: string;
}

function lintNode(node: WasmNodeDefinition, nodeIndex: number): LintIssue[] {
	const issues: LintIssue[] = [];
	const ctx = { nodeName: node.friendly_name || node.name, nodeIndex };

	const inputPins = node.pins.filter((p) => p.pin_type === "Input");
	const outputPins = node.pins.filter((p) => p.pin_type === "Output");
	const inputDataPins = inputPins.filter((p) => p.data_type !== "Execution");
	const outputDataPins = outputPins.filter((p) => p.data_type !== "Execution");
	const inputExecPins = inputPins.filter((p) => p.data_type === "Execution");
	const outputExecPins = outputPins.filter((p) => p.data_type === "Execution");

	// ERROR: Same input pin name as output pin name
	const inputNames = new Set(inputPins.map((p) => p.name));
	for (const pin of outputPins) {
		if (inputNames.has(pin.name)) {
			issues.push({
				...ctx,
				severity: "error",
				message: `Input and output pin share the same name "${pin.name}" — get/set operations will collide. Use different names (e.g. "input_${pin.name}" / "output_${pin.name}").`,
				pinName: pin.name,
			});
		}
	}

	// ERROR: Impure node missing execution pins on one side
	const isImpure = inputExecPins.length > 0 || outputExecPins.length > 0;
	if (isImpure && inputExecPins.length === 0) {
		issues.push({
			...ctx,
			severity: "error",
			message: "Impure node has output execution pin(s) but no input execution pin.",
		});
	}
	if (isImpure && outputExecPins.length === 0) {
		issues.push({
			...ctx,
			severity: "error",
			message: "Impure node has input execution pin(s) but no output execution pin.",
		});
	}

	// ERROR: Pure node with execution pins shouldn't happen, but for completeness
	// (already covered by impure checks above)

	// WARNING: Generic data type used
	for (const pin of node.pins) {
		if (pin.data_type === "Generic") {
			issues.push({
				...ctx,
				severity: "warning",
				message: `Pin "${pin.friendly_name}" (${pin.pin_type.toLowerCase()}) uses the Generic data type — consider using a specific type for better type safety.`,
				pinName: pin.name,
			});
		}
	}

	// WARNING: No description on node
	if (!node.description?.trim()) {
		issues.push({
			...ctx,
			severity: "warning",
			message: "Node has no description. Add one so users understand what it does.",
		});
	}

	// WARNING: No scores defined
	if (!node.scores) {
		issues.push({
			...ctx,
			severity: "warning",
			message: "No scores defined (privacy, security, performance, etc.). Add scores to rate the node.",
		});
	}

	// WARNING: No icon set
	if (!node.icon?.trim()) {
		issues.push({
			...ctx,
			severity: "info",
			message: "No icon set. Consider adding an icon for better visual identification.",
		});
	}

	// WARNING: No description on pins
	for (const pin of node.pins) {
		if (pin.data_type === "Execution") continue;
		if (!pin.description?.trim()) {
			issues.push({
				...ctx,
				severity: "warning",
				message: `Pin "${pin.friendly_name}" (${pin.pin_type.toLowerCase()}) has no description.`,
				pinName: pin.name,
			});
		}
	}

	// WARNING: No default value on input data pins
	for (const pin of inputDataPins) {
		if (pin.default_value === undefined || pin.default_value === null) {
			issues.push({
				...ctx,
				severity: "info",
				message: `Input pin "${pin.friendly_name}" has no default value. Consider setting one for better UX.`,
				pinName: pin.name,
			});
		}
	}

	// WARNING: Struct pin without schema
	for (const pin of node.pins) {
		if (pin.data_type === "Struct" && !pin.schema?.trim()) {
			issues.push({
				...ctx,
				severity: "warning",
				message: `Struct pin "${pin.friendly_name}" (${pin.pin_type.toLowerCase()}) has no schema. Set a JSON schema for typed interactions.`,
				pinName: pin.name,
			});
		}
	}

	// WARNING: Empty category
	if (!node.category?.trim()) {
		issues.push({
			...ctx,
			severity: "warning",
			message: "Node has no category. Assign a category path (e.g. \"Utils/String\").",
		});
	}

	// ERROR: Schema root type is array/set — pin schemas must describe a single element
	for (const pin of node.pins) {
		if (!pin.schema?.trim()) continue;
		try {
			const schema = JSON.parse(pin.schema);
			const rootType = schema.type;
			if (rootType === "array") {
				issues.push({
					...ctx,
					severity: "error",
					message: `Pin "${pin.friendly_name}" (${pin.pin_type.toLowerCase()}) has a root-level array schema. Pin schemas must describe a single element — use value_type (Array/HashSet) for collections.`,
					pinName: pin.name,
				});
			}
		} catch {
			// Invalid JSON schema — not our problem here
		}
	}

	return issues;
}

export function lintNodes(nodes: WasmNodeDefinition[]): LintIssue[] {
	return nodes.flatMap((node, i) => lintNode(node, i));
}

export function countBySeverity(issues: LintIssue[]) {
	let errors = 0;
	let warnings = 0;
	let infos = 0;
	for (const issue of issues) {
		if (issue.severity === "error") errors++;
		else if (issue.severity === "warning") warnings++;
		else infos++;
	}
	return { errors, warnings, infos };
}
