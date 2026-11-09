import { useState } from "react";
import CodeBlock from "../pillar/CodeBlock";

interface ObjectType {
	id: string;
	label: string;
	x: number;
	y: number;
	table: string;
	source: string;
	properties: readonly string[];
	action: string;
	cypher: string;
}

const OBJECTS: readonly ObjectType[] = [
	{
		id: "customer",
		label: "Customer",
		x: 110,
		y: 80,
		table: "customers",
		source: "PostgreSQL",
		properties: ["name", "segment", "country", "lifetime_value"],
		action: "Change segment",
		cypher:
			"MATCH (c:Customer)-[:PLACES]->(o:Order)\nRETURN c.name, count(o) AS orders\nORDER BY orders DESC LIMIT 5",
	},
	{
		id: "order",
		label: "Order",
		x: 300,
		y: 170,
		table: "orders",
		source: "PostgreSQL",
		properties: ["number", "total", "status", "created_at"],
		action: "Refund order",
		cypher:
			"MATCH (o:Order)-[:SHIPS_IN]->(s:Shipment)\nWHERE s.delivered_at > s.promised_at\nRETURN o.number, s.carrier",
	},
	{
		id: "product",
		label: "Product",
		x: 490,
		y: 80,
		table: "products",
		source: "Data Studio",
		properties: ["sku", "name", "price", "stock"],
		action: "Reorder stock",
		cypher:
			"MATCH (o:Order)-[:CONTAINS]->(p:Product)\nRETURN p.sku, count(o) AS sold\nORDER BY sold DESC LIMIT 10",
	},
	{
		id: "shipment",
		label: "Shipment",
		x: 300,
		y: 320,
		table: "shipments",
		source: "Parquet on S3",
		properties: ["carrier", "promised_at", "delivered_at"],
		action: "Notify customer",
		cypher:
			"MATCH (s:Shipment)<-[:SHIPS_IN]-(o:Order)<-[:PLACES]-(c:Customer)\nWHERE s.delivered_at IS NULL\nRETURN c.name, o.number",
	},
	{
		id: "ticket",
		label: "Ticket",
		x: 110,
		y: 290,
		table: "tickets",
		source: "Data Studio",
		properties: ["subject", "priority", "status"],
		action: "Escalate ticket",
		cypher:
			"MATCH (c:Customer)-[:OPENS]->(t:Ticket)-[:ABOUT]->(o:Order)\nWHERE t.priority = 'high'\nRETURN c.name, o.number",
	},
	{
		id: "supplier",
		label: "Supplier",
		x: 490,
		y: 290,
		table: "suppliers",
		source: "MySQL",
		properties: ["name", "rating", "country"],
		action: "Request quote",
		cypher:
			"MATCH (p:Product)-[:SUPPLIED_BY]->(s:Supplier)\nWHERE p.stock < 20\nRETURN s.name, collect(p.sku)",
	},
];

const LINKS: readonly { from: string; to: string; label: string }[] = [
	{ from: "customer", to: "order", label: "places" },
	{ from: "order", to: "product", label: "contains" },
	{ from: "order", to: "shipment", label: "ships in" },
	{ from: "customer", to: "ticket", label: "opens" },
	{ from: "ticket", to: "order", label: "about" },
	{ from: "product", to: "supplier", label: "supplied by" },
];

const byId = new Map(OBJECTS.map((object) => [object.id, object]));

export default function DataOntology() {
	const [selectedId, setSelectedId] = useState("order");
	const selected = byId.get(selectedId) ?? OBJECTS[0];
	const related = LINKS.filter(
		(link) => link.from === selected.id || link.to === selected.id,
	);

	return (
		<div className="dt-onto">
			<div className="dt-onto-graph">
				<svg viewBox="0 0 600 400" aria-hidden="true">
					{LINKS.map((link) => {
						const from = byId.get(link.from);
						const to = byId.get(link.to);
						if (!from || !to) return null;
						const active = link.from === selected.id || link.to === selected.id;
						return (
							<g key={`${link.from}-${link.to}`} data-active={active}>
								<line x1={from.x} y1={from.y} x2={to.x} y2={to.y} />
								<text
									x={(from.x + to.x) / 2}
									y={(from.y + to.y) / 2 - 6}
									textAnchor="middle"
								>
									{link.label}
								</text>
							</g>
						);
					})}
				</svg>
				{OBJECTS.map((object) => (
					<button
						key={object.id}
						type="button"
						className="dt-onto-node"
						aria-pressed={object.id === selected.id}
						style={{
							left: `${(object.x / 600) * 100}%`,
							top: `${(object.y / 400) * 100}%`,
						}}
						onClick={() => setSelectedId(object.id)}
					>
						{object.label}
					</button>
				))}
			</div>
			<aside className="dt-onto-panel" aria-live="polite">
				<span className="dt-onto-kind">Object type</span>
				<h3>{selected.label}</h3>
				<p className="dt-onto-source">
					Backed by <code>{selected.table}</code> in {selected.source}
				</p>
				<ul className="dt-onto-props">
					{selected.properties.map((property) => (
						<li key={property}>{property}</li>
					))}
				</ul>
				<p className="dt-onto-links">
					{related.map((link) => (
						<span key={`${link.from}-${link.to}`}>
							{byId.get(link.from)?.label} <em>{link.label}</em>{" "}
							{byId.get(link.to)?.label}
						</span>
					))}
				</p>
				<div className="dt-onto-action">
					<b>{selected.action}</b>
					<span>Governed action · validated inputs · pinned flow version</span>
				</div>
				<CodeBlock
					code={selected.cypher}
					language="cypher"
					title="query.cypher"
					className="pl-code--compact"
				/>
			</aside>
		</div>
	);
}
