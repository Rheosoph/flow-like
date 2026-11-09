import { type CSSProperties, useState } from "react";
import type { IconType } from "react-icons";
import {
	LuDatabase,
	LuFileSpreadsheet,
	LuLayers,
	LuTable2,
} from "react-icons/lu";
import CodeBlock from "../pillar/CodeBlock";
import {
	type DataClassification,
	LevelBadge,
	SensitivityBadges,
	combineDataSensitivity,
} from "./sensitivity";

interface Column {
	name: string;
	type: string;
	level: DataClassification;
	pii?: boolean;
}

interface Table {
	id: string;
	schema: string;
	source: string;
	icon: IconType;
	columns: readonly Column[];
}

const TABLES: Readonly<Record<string, Table>> = {
	orders: {
		id: "orders",
		schema: "shop",
		source: "PostgreSQL",
		icon: LuDatabase,
		columns: [
			{ name: "id", type: "bigint", level: "internal" },
			{ name: "customer_id", type: "bigint", level: "internal" },
			{ name: "region_id", type: "int", level: "public" },
			{ name: "total", type: "numeric", level: "confidential" },
			{ name: "created_at", type: "timestamp", level: "internal" },
		],
	},
	regions: {
		id: "regions",
		schema: "app",
		source: "Data Studio",
		icon: LuTable2,
		columns: [
			{ name: "id", type: "int32", level: "public" },
			{ name: "name", type: "utf8", level: "public" },
		],
	},
	customers: {
		id: "customers",
		schema: "shop",
		source: "PostgreSQL",
		icon: LuDatabase,
		columns: [
			{ name: "id", type: "bigint", level: "internal" },
			{ name: "name", type: "text", level: "confidential", pii: true },
			{ name: "email", type: "text", level: "confidential", pii: true },
		],
	},
	shipments: {
		id: "shipments",
		schema: "lake",
		source: "Parquet on S3",
		icon: LuLayers,
		columns: [
			{ name: "order_id", type: "int64", level: "internal" },
			{ name: "carrier", type: "string", level: "internal" },
			{ name: "promised_at", type: "timestamp", level: "internal" },
			{ name: "delivered_at", type: "timestamp", level: "internal" },
		],
	},
	tickets: {
		id: "tickets",
		schema: "app",
		source: "Data Studio",
		icon: LuTable2,
		columns: [
			{ name: "order_id", type: "int64", level: "internal" },
			{ name: "subject", type: "utf8", level: "confidential" },
			{ name: "created_at", type: "timestamp", level: "internal" },
		],
	},
	order_items: {
		id: "order_items",
		schema: "sales",
		source: "DuckDB",
		icon: LuDatabase,
		columns: [
			{ name: "id", type: "bigint", level: "internal" },
			{ name: "sku", type: "varchar", level: "public" },
			{ name: "quantity", type: "int", level: "internal" },
		],
	},
	returns: {
		id: "returns",
		schema: "files",
		source: "returns.csv",
		icon: LuFileSpreadsheet,
		columns: [
			{ name: "id", type: "int64", level: "internal" },
			{ name: "item_id", type: "int64", level: "internal" },
			{ name: "reason", type: "utf8", level: "internal" },
		],
	},
};

interface Question {
	id: string;
	label: string;
	ask: string;
	tables: readonly string[];
	/** `table.column` pairs the query reads. */
	uses: readonly string[];
	sql: string;
	columns: readonly string[];
	rows: readonly (readonly string[])[];
	values: readonly number[];
	ms: number;
}

const QUESTIONS: readonly Question[] = [
	{
		id: "revenue",
		label: "Revenue by region",
		ask: "What did each region earn this quarter?",
		tables: ["orders", "regions"],
		uses: [
			"orders.total",
			"orders.region_id",
			"orders.created_at",
			"regions.id",
			"regions.name",
		],
		sql: `-- Revenue per region this quarter
SELECT r.name AS region,
       SUM(o.total) AS revenue
FROM shop.orders o
JOIN app.regions r ON r.id = o.region_id
WHERE o.created_at >= DATE '2026-10-01'
GROUP BY r.name
ORDER BY revenue DESC;`,
		columns: ["region", "revenue"],
		rows: [
			["Americas", "€538,410"],
			["EMEA", "€412,300"],
			["APAC", "€201,950"],
		],
		values: [538410, 412300, 201950],
		ms: 38,
	},
	{
		id: "late",
		label: "Late deliveries → tickets",
		ask: "Which customers opened a support ticket after a late delivery?",
		tables: ["orders", "shipments", "tickets", "customers"],
		uses: [
			"orders.id",
			"orders.customer_id",
			"shipments.order_id",
			"shipments.promised_at",
			"shipments.delivered_at",
			"tickets.order_id",
			"tickets.created_at",
			"customers.id",
			"customers.name",
		],
		sql: `SELECT c.name AS customer,
       COUNT(*) AS tickets
FROM shop.orders o
JOIN lake.shipments s ON s.order_id = o.id
JOIN app.tickets t ON t.order_id = o.id
JOIN shop.customers c ON c.id = o.customer_id
WHERE s.delivered_at > s.promised_at
  AND t.created_at > s.delivered_at
GROUP BY c.name
ORDER BY tickets DESC
LIMIT 3;`,
		columns: ["customer", "tickets"],
		rows: [
			["Contoso GmbH", "7"],
			["Fabrikam AB", "4"],
			["Northwind Ltd", "3"],
		],
		values: [7, 4, 3],
		ms: 112,
	},
	{
		id: "returns",
		label: "Return rate by product",
		ask: "Which products get returned most, relative to how often they sell?",
		tables: ["order_items", "returns"],
		uses: [
			"order_items.id",
			"order_items.sku",
			"returns.id",
			"returns.item_id",
		],
		sql: `SELECT i.sku,
       COUNT(r.id) * 1.0 / COUNT(i.id) AS return_rate
FROM sales.order_items i
LEFT JOIN files.returns r ON r.item_id = i.id
GROUP BY i.sku
HAVING COUNT(i.id) > 100
ORDER BY return_rate DESC
LIMIT 3;`,
		columns: ["sku", "return_rate"],
		rows: [
			["TRAIL-42", "11.8%"],
			["CITY-07", "7.2%"],
			["KIDS-15", "5.9%"],
		],
		values: [11.8, 7.2, 5.9],
		ms: 21,
	},
];

const LINKS: Readonly<Record<string, string>> = Object.fromEntries(
	Object.keys(TABLES).map((id) => [id, id]),
);

function SchemaCard({
	table,
	uses,
	active,
	index,
	onHover,
}: {
	table: Table;
	uses: ReadonlySet<string>;
	active: boolean;
	index: number;
	onHover: (id: string | null) => void;
}) {
	const Icon = table.icon;
	return (
		<article
			className="dt-schema"
			data-active={active}
			style={{ "--tilt": `${index % 2 === 0 ? -1.2 : 1}deg` } as CSSProperties}
			onMouseEnter={() => onHover(table.id)}
			onMouseLeave={() => onHover(null)}
		>
			<header>
				<Icon aria-hidden="true" />
				<b>
					{table.schema}.<span>{table.id}</span>
				</b>
				<small>{table.source}</small>
			</header>
			<ul>
				{table.columns.map((column) => (
					<li
						key={column.name}
						data-used={uses.has(`${table.id}.${column.name}`)}
					>
						<code>{column.name}</code>
						<span className="dt-schema-type">{column.type}</span>
						<span className="dt-schema-label">
							<LevelBadge level={column.level} />
							{column.pii ? (
								<span className="dt-schema-pii" title="Personal data">
									PII
								</span>
							) : null}
						</span>
					</li>
				))}
			</ul>
		</article>
	);
}

export default function DataQuery() {
	const [questionId, setQuestionId] = useState(QUESTIONS[0].id);
	const [mode, setMode] = useState<"sql" | "ask">("sql");
	const [hovered, setHovered] = useState<string | null>(null);
	const question =
		QUESTIONS.find((item) => item.id === questionId) ?? QUESTIONS[0];
	const uses = new Set(question.uses);
	const usedColumns = question.tables.flatMap((id) =>
		TABLES[id].columns.filter((column) => uses.has(`${id}.${column.name}`)),
	);
	const label = combineDataSensitivity(
		...usedColumns.map((column) => ({
			classification: column.level,
			pii: column.pii,
		})),
	);
	const max = Math.max(...question.values);

	return (
		<div className="dt-query">
			<div className="dt-query-bar">
				<fieldset className="dt-tabs" aria-label="Question">
					{QUESTIONS.map((item) => (
						<button
							key={item.id}
							type="button"
							aria-pressed={item.id === question.id}
							onClick={() => setQuestionId(item.id)}
						>
							{item.label}
						</button>
					))}
				</fieldset>
				<fieldset className="dt-tabs" aria-label="How to ask">
					<button
						type="button"
						aria-pressed={mode === "sql"}
						onClick={() => setMode("sql")}
					>
						SQL
					</button>
					<button
						type="button"
						aria-pressed={mode === "ask"}
						onClick={() => setMode("ask")}
					>
						Ask Flux
					</button>
				</fieldset>
			</div>
			<div className="dt-query-stage" key={question.id}>
				<div className="dt-schemas" aria-label="Tables used">
					{question.tables.map((id, index) => (
						<SchemaCard
							key={id}
							table={TABLES[id]}
							uses={uses}
							active={hovered === id}
							index={index}
							onHover={setHovered}
						/>
					))}
				</div>
				<div className="dt-query-main">
					{mode === "sql" ? (
						<CodeBlock
							code={question.sql}
							language="sql"
							title="query.sql"
							badge="DataFusion · read-only"
							links={LINKS}
							activeLink={hovered}
							onLink={setHovered}
						/>
					) : (
						<div className="dt-ask">
							<p className="dt-ask-you">{question.ask}</p>
							<div className="dt-ask-flux">
								<b>Flux</b>
								<p>
									I joined {question.tables.length} tables from{" "}
									{new Set(question.tables.map((id) => TABLES[id].source)).size}{" "}
									sources and ran one read-only query. Here's the answer; the
									SQL is in the step log.
								</p>
							</div>
						</div>
					)}
					<div className="dt-result">
						<header>
							<span>
								{question.rows.length} rows · {question.ms} ms
							</span>
							<span className="dt-result-label">
								Built from
								<SensitivityBadges value={label} />
							</span>
						</header>
						<table>
							<thead>
								<tr>
									{question.columns.map((column) => (
										<th key={column} scope="col">
											{column}
										</th>
									))}
									<th aria-hidden="true" />
								</tr>
							</thead>
							<tbody>
								{question.rows.map((row, rowIndex) => (
									<tr key={row[0]}>
										{row.map((cell, index) => (
											<td key={`${row[0]}-${question.columns[index]}`}>
												{cell}
											</td>
										))}
										<td className="dt-result-bar" aria-hidden="true">
											<i
												style={{
													width: `${(question.values[rowIndex] / max) * 100}%`,
												}}
												data-top={rowIndex === 0}
											/>
										</td>
									</tr>
								))}
							</tbody>
						</table>
					</div>
				</div>
			</div>
		</div>
	);
}
