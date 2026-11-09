import type {
	GraphOverlay,
	RemoteOntologyImport,
} from "../../packages/ui/state/backend-state/graph-state";
import type {
	ExecuteSqlPayload,
	ExecuteSqlResult,
	QueryColumn,
	SavedQuery,
} from "../../packages/ui/state/backend-state/query-state";

export const appId = "fixture-knowledge";
const timestamp = "2026-11-06T09:30:00Z";
const document = `# Collar joining: cold forming

## Process overview

The workpiece is joined by pressing a collar into a prepared opening. The collar transfers the load through its contact surface.

### Setup checklist

- Inspect the fixture and align the workpiece.
- Confirm the material thickness and insertion depth.
- Measure the pressing force before releasing the part.

| Parameter | Value | Unit |
| --- | --- | --- |
| Pressing force | 12.5 | kN |
| Insertion depth | 4.2 | mm |

**Inspection note:** check the complete circumference for cracks. A visible crack requires a new sample and a review of the forming setup.

Read the [process reference](https://example.com/process-reference) for the inspection procedure.

\`\`\`json
{ "operation": "press", "quality_check": true }
\`\`\`

## Additional observations

${"The operator records the measured force and checks the material surface after each trial. Keep the fixture clean and record any changes to the setup. ".repeat(15)}`;

export const columns: QueryColumn[] = [
	{ name: "id", type_name: "Utf8", position: 0 },
	{ name: "topic", type_name: "Utf8", position: 1 },
	{ name: "process", type_name: "Utf8", position: 2 },
	{ name: "content", type_name: "Utf8", position: 3 },
	{ name: "notes", type_name: "Utf8", position: 4 },
	{ name: "metadata", type_name: "Struct", position: 5 },
	{ name: "approved", type_name: "Boolean", position: 6 },
	{ name: "confidence", type_name: "Float64", position: 7 },
	{
		name: "updated_at",
		type_name: "Timestamp(Millisecond, None)",
		position: 8,
	},
	{ name: "attachment", type_name: "Utf8", position: 9 },
];

const topics = [
	"Collar joining, cold forming",
	"Press fitting",
	"Laser welding",
	"Adhesive bonding",
	"Riveting",
	"Friction welding",
	"Mechanical fastening",
	"Brazing",
];
export const rows = topics.map((topic, index) => ({
	id: `BS_00${34 + index}_2010`,
	topic,
	process: ["Forming", "Pressing", "Welding", "Bonding"][index % 4],
	content:
		index === 0
			? document
			: `## ${topic}\n\nProcess record ${index + 1}. **Inspection complete.**\n\n- Sample reviewed\n- Measurements recorded`,
	notes:
		index === 1
			? ""
			: index === 2
				? null
				: "Sample inspected after the first production run. The material, setup parameters, and measured results are documented in the attached report. ".repeat(
						4,
					),
	metadata: {
		material: index % 2 ? "Aluminium" : "Steel",
		revision: index + 1,
		tags: ["manufacturing", "verified"],
		settings: { force_kN: 12.5 + index, temperature_C: 20 },
		reviewer: null,
	},
	approved: index % 3 !== 2,
	confidence: 0.98 - index * 0.025,
	updated_at: 1793957400000 - index * 86400000,
	attachment: `apps/${appId}/upload/inspection-${index + 1}.txt`,
}));

export const savedQueries: SavedQuery[] = [
	{
		id: "query-documents",
		name: "Process documents",
		description: "Inspect full content and quality records",
		sql: 'SELECT *\nFROM "data"\nORDER BY updated_at DESC',
		kind: "query",
	},
	{
		id: "query-one",
		name: "Find a document",
		description: "A single record with long Markdown content",
		sql: "SELECT * FROM \"data\" WHERE id = 'BS_0034_2010'",
		kind: "query",
	},
	{
		id: "query-parameter",
		name: "Documents by process",
		description: "Filter with a named parameter",
		sql: 'SELECT * FROM "data" WHERE process = $process',
		kind: "query",
		param_schema: {
			properties: {
				process: {
					type: "string",
					title: "Process",
					enum: ["Forming", "Pressing", "Welding", "Bonding"],
				},
			},
			required: ["process"],
		},
	},
	{
		id: "query-empty",
		name: "Empty result",
		description: "Review the no-results state",
		sql: 'SELECT * FROM "data" WHERE 1 = 0',
		kind: "query",
	},
	{
		id: "query-error",
		name: "Query error",
		description: "Review an invalid table error",
		sql: 'SELECT * FROM "missing_table"',
		kind: "query",
	},
	{
		id: "view-approved",
		name: "approved_documents",
		description: "Approved process records",
		sql: 'SELECT * FROM "data" WHERE approved = true',
		kind: "view",
	},
].map((query) => ({
	...query,
	app_id: appId,
	surface: "native",
	default_limit: 1000,
	viz_config: { view: "table" },
	created_at: timestamp,
	updated_at: timestamp,
})) as SavedQuery[];

export const ontology: GraphOverlay = {
	id: "manufacturing",
	name: "Manufacturing",
	description: "Processes and their documents",
	nodes: [
		{
			label: "Document",
			table: "data",
			id_column: "id",
			display_column: "topic",
			property_columns: columns.map((column) => ({
				name: column.name,
				data_type: column.type_name,
			})),
			style: {
				color: "#3b82f6",
				icon: "file-text",
				size: { mode: "fixed", value: 12 },
			},
		},
	],
	edges: [],
	object_views: [],
	actions: [],
	exposed: true,
	bindings_enabled: true,
	default_limit: 1000,
	created_at: timestamp,
	updated_at: timestamp,
};
export const remoteImport: RemoteOntologyImport = {
	id: "supplier-documents",
	target_app_id: "supplier-app",
	remote_ontology_id: "supplier-manufacturing",
	contract: { ...ontology, name: "Supplier documents" },
	source_updated_at: timestamp,
	bindings_enabled: true,
	installed_at: timestamp,
	updated_at: timestamp,
};

export function execute(
	payload: Pick<ExecuteSqlPayload, "sql" | "params" | "limit">,
	data = rows,
): ExecuteSqlResult {
	if (/missing_table/i.test(payload.sql))
		throw new Error(
			'Table "missing_table" was not found. Available tables: data, process_metrics.',
		);
	if (/^\s*select\s+1\s*;?\s*$/i.test(payload.sql))
		return {
			columns: [{ name: "Int64(1)", type_name: "Int64", position: 0 }],
			rows: [{ "Int64(1)": 1 }],
			row_count: 1,
			truncated: false,
		};
	let matching = /where\s+1\s*=\s*0/i.test(payload.sql) ? [] : data;
	if (payload.sql.includes("BS_0034_2010")) matching = matching.slice(0, 1);
	if (payload.sql.includes("$process"))
		matching = matching.filter(
			(row) => row.process === payload.params?.process,
		);
	if (/approved\s*=\s*true/i.test(payload.sql))
		matching = matching.filter((row) => row.approved);
	const result = matching.slice(0, payload.limit ?? matching.length);
	return {
		columns,
		rows: result,
		row_count: result.length,
		truncated: result.length < matching.length,
	};
}
