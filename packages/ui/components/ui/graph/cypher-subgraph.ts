import type {
	GraphOverlay,
	SubgraphEdge,
	SubgraphNode,
	SubgraphResult,
} from "../../../state/backend-state/graph-state";

/**
 * Rebuilds drawable nodes and edges from tabular Cypher results.
 *
 * The graph engine expands a returned node variable into `{var}.{column}`
 * columns, so the structure survives the trip — it just arrives flattened.
 * Each variable's column set is matched against the overlay's mappings: a
 * group carrying a node mapping's id column becomes that node, a group
 * carrying an edge mapping's source and target columns becomes that edge.
 */
export function subgraphFromCypherRows(
	rows: readonly unknown[],
	overlay: GraphOverlay,
): SubgraphResult | null {
	if (rows.length === 0) return null;

	const varColumns = new Map<string, Set<string>>();
	for (const row of rows) {
		if (typeof row !== "object" || row === null) continue;
		for (const key of Object.keys(row)) {
			const dot = key.indexOf(".");
			if (dot <= 0 || dot === key.length - 1) continue;
			const variable = key.slice(0, dot);
			const column = key.slice(dot + 1);
			const bucket = varColumns.get(variable);
			if (bucket) bucket.add(column);
			else varColumns.set(variable, new Set([column]));
		}
	}
	if (varColumns.size === 0) return null;

	interface NodeBinding {
		kind: "node";
		variable: string;
		label: string;
		idColumn: string;
		displayColumn?: string;
	}
	interface EdgeBinding {
		kind: "edge";
		variable: string;
		label: string;
		srcColumn: string;
		dstColumn: string;
		srcLabel: string;
		dstLabel: string;
	}

	const bindings: (NodeBinding | EdgeBinding)[] = [];
	for (const [variable, columns] of varColumns) {
		// An edge group is unmistakable: it carries both endpoint columns.
		const edgeMapping = overlay.edges.find(
			(mapping) =>
				columns.has(mapping.src_column) && columns.has(mapping.dst_column),
		);
		if (edgeMapping) {
			bindings.push({
				kind: "edge",
				variable,
				label: edgeMapping.label,
				srcColumn: edgeMapping.src_column,
				dstColumn: edgeMapping.dst_column,
				srcLabel: edgeMapping.src_label,
				dstLabel: edgeMapping.dst_label,
			});
			continue;
		}

		// Node groups are scored by how much of the column set the mapping
		// explains, so two labels sharing an id column name still resolve.
		let best: { mapping: NodeBinding; score: number } | null = null;
		for (const mapping of overlay.nodes) {
			if (!columns.has(mapping.id_column)) continue;
			const known = new Set<string>([
				mapping.id_column,
				...(mapping.display_column ? [mapping.display_column] : []),
				...mapping.property_columns.map((column) => column.name),
			]);
			let overlap = 0;
			for (const column of columns) {
				if (known.has(column)) overlap += 1;
			}
			const score = overlap / columns.size;
			if (!best || score > best.score) {
				best = {
					mapping: {
						kind: "node",
						variable,
						label: mapping.label,
						idColumn: mapping.id_column,
						displayColumn: mapping.display_column,
					},
					score,
				};
			}
		}
		if (best) bindings.push(best.mapping);
	}
	if (bindings.length === 0) return null;

	const nodeById = new Map<string, SubgraphNode>();
	const edgeById = new Map<string, SubgraphEdge>();

	const ensureStubNode = (label: string, rawId: unknown) => {
		if (rawId === null || rawId === undefined || label.length === 0)
			return null;
		const id = `${label}:${String(rawId)}`;
		if (!nodeById.has(id)) {
			nodeById.set(id, {
				id,
				label,
				caption: String(rawId),
				props: {},
			});
		}
		return id;
	};

	for (const row of rows) {
		if (typeof row !== "object" || row === null) continue;
		const record = row as Record<string, unknown>;

		for (const binding of bindings) {
			const value = (column: string) => record[`${binding.variable}.${column}`];

			if (binding.kind === "node") {
				const rawId = value(binding.idColumn);
				if (rawId === null || rawId === undefined) continue;
				const id = `${binding.label}:${String(rawId)}`;

				const props: Record<string, unknown> = {};
				const prefix = `${binding.variable}.`;
				for (const [key, cell] of Object.entries(record)) {
					if (key.startsWith(prefix)) props[key.slice(prefix.length)] = cell;
				}

				const displayValue = binding.displayColumn
					? props[binding.displayColumn]
					: undefined;
				const caption =
					displayValue !== null && displayValue !== undefined
						? String(displayValue)
						: String(rawId);

				// A row that carries more columns for a node we already saw wins over
				// the stub an edge endpoint created.
				const existing = nodeById.get(id);
				if (!existing || Object.keys(existing.props).length === 0) {
					nodeById.set(id, { id, label: binding.label, caption, props });
				}
				continue;
			}

			const srcRaw = value(binding.srcColumn);
			const dstRaw = value(binding.dstColumn);
			const source = ensureStubNode(binding.srcLabel, srcRaw);
			const target = ensureStubNode(binding.dstLabel, dstRaw);
			if (!source || !target) continue;

			const id = `${binding.label}:${source}->${target}`;
			if (edgeById.has(id)) continue;

			const props: Record<string, unknown> = {};
			const prefix = `${binding.variable}.`;
			for (const [key, cell] of Object.entries(record)) {
				if (key.startsWith(prefix)) props[key.slice(prefix.length)] = cell;
			}
			edgeById.set(id, {
				id,
				source,
				target,
				label: binding.label,
				props,
			});
		}
	}

	if (nodeById.size === 0 && edgeById.size === 0) return null;
	return {
		nodes: [...nodeById.values()],
		edges: [...edgeById.values()],
		truncated: false,
	};
}
