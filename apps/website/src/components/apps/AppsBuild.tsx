import { type CSSProperties, useState } from "react";
import CodeBlock from "../pillar/CodeBlock";

type View = "board" | "code" | "flux";
type PinKind = "exec" | "database" | "session" | "rows" | "string";

const VIEWS: readonly { id: View; label: string }[] = [
	{ id: "board", label: "Board" },
	{ id: "code", label: "FlowScript" },
	{ id: "flux", label: "Ask Flux" },
];

interface Pin {
	name: string;
	kind: PinKind;
}

interface BoardNode {
	id: string;
	title: string;
	tone: "event" | "data" | "util" | "log";
	x: number;
	y: number;
	width: number;
	inputs: readonly Pin[];
	outputs: readonly Pin[];
}

const W = 1000;
const H = 600;
const HEADER = 34;
const ROW = 26;

const NODES: readonly BoardNode[] = [
	{
		id: "event",
		title: "Simple Event",
		tone: "event",
		x: 60,
		y: 40,
		width: 190,
		inputs: [],
		outputs: [{ name: "", kind: "exec" }],
	},
	{
		id: "open",
		title: "Open Database",
		tone: "data",
		x: 300,
		y: 40,
		width: 240,
		inputs: [{ name: "", kind: "exec" }],
		outputs: [
			{ name: "", kind: "exec" },
			{ name: "Database", kind: "database" },
		],
	},
	{
		id: "session",
		title: "Create DataFusion Session",
		tone: "data",
		x: 600,
		y: 40,
		width: 320,
		inputs: [{ name: "", kind: "exec" }],
		outputs: [
			{ name: "", kind: "exec" },
			{ name: "Session", kind: "session" },
		],
	},
	{
		id: "register",
		title: "Register Lance Table",
		tone: "data",
		x: 60,
		y: 300,
		width: 218,
		inputs: [
			{ name: "", kind: "exec" },
			{ name: "Session", kind: "session" },
			{ name: "Database", kind: "database" },
		],
		outputs: [{ name: "", kind: "exec" }],
	},
	{
		id: "query",
		title: "SQL Query",
		tone: "data",
		x: 340,
		y: 300,
		width: 200,
		inputs: [
			{ name: "", kind: "exec" },
			{ name: "Session", kind: "session" },
		],
		outputs: [
			{ name: "", kind: "exec" },
			{ name: "Rows", kind: "rows" },
		],
	},
	{
		id: "string",
		title: "To String",
		tone: "util",
		x: 580,
		y: 420,
		width: 180,
		inputs: [{ name: "Value", kind: "rows" }],
		outputs: [{ name: "String", kind: "string" }],
	},
	{
		id: "print",
		title: "Print Info",
		tone: "log",
		x: 790,
		y: 300,
		width: 190,
		inputs: [
			{ name: "", kind: "exec" },
			{ name: "Message", kind: "string" },
		],
		outputs: [{ name: "", kind: "exec" }],
	},
];

/** From node, output pin, to node, input pin, and the gutter lane of a wire that runs backwards. */
const WIRES: readonly [string, number, string, number, number?][] = [
	["event", 0, "open", 0],
	["open", 0, "session", 0],
	["session", 0, "register", 0, 0],
	["register", 0, "query", 0],
	["query", 0, "print", 0],
	["open", 1, "register", 2, 2],
	["session", 1, "register", 1, 1],
	["session", 1, "query", 1, 3],
	["query", 1, "string", 0],
	["string", 0, "print", 1],
];

/** Wires that run back to an earlier column travel through the gap between the rows. */
const GUTTER = 196;
const LANE = 14;
const CORNER = 12;

const PIN_COLOR: Readonly<Record<PinKind, string>> = {
	exec: "#f2f2f5",
	database: "#4fd1c5",
	session: "#a78bfa",
	rows: "#f6ad55",
	string: "#f472b6",
};

const byId = new Map(NODES.map((node) => [node.id, node]));

function pinY(node: BoardNode, index: number) {
	return node.y + HEADER + 14 + index * ROW;
}

function wirePath(
	from: BoardNode,
	fromIndex: number,
	to: BoardNode,
	toIndex: number,
	lane?: number,
) {
	const x1 = from.x + from.width;
	const y1 = pinY(from, fromIndex);
	const x2 = to.x;
	const y2 = pinY(to, toIndex);
	if (lane === undefined) {
		const reach = Math.max(40, Math.abs(x2 - x1) * 0.5);
		return `M${x1} ${y1} C${x1 + reach} ${y1} ${x2 - reach} ${y2} ${x2} ${y2}`;
	}
	const out = x1 + 22 + lane * 6;
	const back = x2 - 22 - lane * 6;
	const gutter = GUTTER + lane * LANE;
	const r = CORNER;
	return [
		`M${x1} ${y1}`,
		`H${out - r} Q${out} ${y1} ${out} ${y1 + r}`,
		`V${gutter - r} Q${out} ${gutter} ${out - r} ${gutter}`,
		`H${back + r} Q${back} ${gutter} ${back} ${gutter + r}`,
		`V${y2 - r} Q${back} ${y2} ${back + r} ${y2}`,
		`H${x2}`,
	].join(" ");
}

function PinMark({ kind }: { kind: PinKind }) {
	return kind === "exec" ? (
		<i className="ap-pin ap-pin--exec" />
	) : (
		<i className="ap-pin" style={{ background: PIN_COLOR[kind] }} />
	);
}

function Board() {
	return (
		<div
			className="ap-board"
			role="img"
			aria-label="The flow as a board of seven connected nodes"
		>
			<svg
				viewBox={`0 0 ${W} ${H}`}
				preserveAspectRatio="none"
				aria-hidden="true"
			>
				{WIRES.map(([fromId, fromIndex, toId, toIndex, lane]) => {
					const from = byId.get(fromId);
					const to = byId.get(toId);
					if (!from || !to) return null;
					const kind = from.outputs[fromIndex].kind;
					return (
						<path
							key={`${fromId}-${fromIndex}-${toId}-${toIndex}`}
							d={wirePath(from, fromIndex, to, toIndex, lane)}
							stroke={PIN_COLOR[kind]}
							strokeOpacity={kind === "exec" ? 0.85 : 0.75}
							strokeWidth={kind === "exec" ? 3 : 2.2}
							fill="none"
						/>
					);
				})}
			</svg>
			{NODES.map((node) => {
				const rows = Math.max(node.inputs.length, node.outputs.length);
				return (
					<div
						key={node.id}
						className="ap-board-node"
						data-tone={node.tone}
						style={
							{
								"--x": `${(node.x / W) * 100}%`,
								"--y": `${(node.y / H) * 100}%`,
								"--w": `${(node.width / W) * 100}%`,
								"--h": `${((HEADER + 16 + rows * ROW) / H) * 100}%`,
							} as CSSProperties
						}
					>
						<b>{node.title}</b>
						<div className="ap-board-pins">
							<span>
								{node.inputs.map((pin, index) => (
									<em key={`in-${pin.kind}-${index}`}>
										<PinMark kind={pin.kind} />
										{pin.name}
									</em>
								))}
							</span>
							<span className="ap-board-out">
								{node.outputs.map((pin, index) => (
									<em key={`out-${pin.kind}-${index}`}>
										{pin.name}
										<PinMark kind={pin.kind} />
									</em>
								))}
							</span>
						</div>
					</div>
				);
			})}
		</div>
	);
}

const CODE = `use db::*

eventsSimple weeklyRevenue() {
    const sales = open({ name: "sales" })
    const session = df::createSession({ sessionName: "weekly" })
    session.registerLance({ database: sales, tableName: "orders" })
    const { rows } = session.sqlQuery({
        query: "SELECT region, SUM(total) AS revenue FROM orders GROUP BY region",
    })
    log::info(json::stringify({ value: rows, pretty: true }), { toast: false })
}`;

function Flux() {
	return (
		<div className="ap-flux-chat">
			<p className="ap-mock-you">
				Every Monday, total the revenue per region from the orders table and log
				it.
			</p>
			<div className="ap-flux-reply">
				<b>Flux</b>
				<p>
					Built <strong>Weekly revenue</strong>: it opens the sales database,
					sums orders by region with SQL and logs the result. I added a Cron
					event for Mondays at 08:00.
				</p>
				<ul>
					<li>7 nodes added</li>
					<li>1 event added</li>
					<li>Ready for review</li>
				</ul>
			</div>
		</div>
	);
}

export default function AppsBuild() {
	const [view, setView] = useState<View>("board");
	return (
		<div className="ap-build">
			<fieldset className="ap-build-tabs" aria-label="View">
				{VIEWS.map((item) => (
					<button
						key={item.id}
						type="button"
						aria-pressed={item.id === view}
						onClick={() => setView(item.id)}
					>
						{item.label}
					</button>
				))}
			</fieldset>
			<div className="ap-build-stage" key={view}>
				{view === "board" ? (
					<Board />
				) : view === "code" ? (
					<CodeBlock
						code={CODE}
						language="flowscript"
						title="weekly-revenue.flow"
						badge="FlowScript"
					/>
				) : (
					<Flux />
				)}
			</div>
		</div>
	);
}
