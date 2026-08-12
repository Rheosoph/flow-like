import { mountFlowWidget } from "@flow-like/widget-sdk";
import { useWidgetProps } from "@flow-like/widget-sdk/react";
import {
	type CSSProperties,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { createRoot } from "react-dom/client";
import widget, { type SalesRow } from "./widget.config";

const bridge = mountFlowWidget(widget);

const CHART_WIDTH = 640;
const CHART_HEIGHT = 200;

const styles: Record<string, CSSProperties> = {
	main: { display: "grid", gap: "0.75rem", padding: "1rem" },
	header: {
		display: "flex",
		alignItems: "baseline",
		justifyContent: "space-between",
		gap: "1rem",
		flexWrap: "wrap",
	},
	title: { margin: 0, fontSize: "1.125rem", fontWeight: 600 },
	total: { color: "var(--muted-foreground)", fontSize: "0.875rem" },
	chart: { width: "100%", height: CHART_HEIGHT, overflow: "visible" },
	footer: {
		display: "flex",
		alignItems: "center",
		justifyContent: "space-between",
		gap: "1rem",
		fontSize: "0.875rem",
		color: "var(--muted-foreground)",
	},
	button: {
		background: "transparent",
		color: "var(--foreground)",
		border: "1px solid var(--border)",
		borderRadius: "var(--radius)",
		padding: "0.35rem 0.75rem",
		font: "inherit",
		fontSize: "0.8125rem",
		cursor: "pointer",
	},
	empty: {
		display: "grid",
		placeItems: "center",
		height: CHART_HEIGHT,
		border: "1px dashed var(--border)",
		borderRadius: "var(--radius)",
		color: "var(--muted-foreground)",
	},
};

function formatValue(value: number, currency: string): string {
	return `${Math.round(value).toLocaleString()} ${currency}`.trim();
}

interface Geometry {
	x: number;
	y: number;
	width: number;
	height: number;
	centerX: number;
}

function useGeometry(rows: SalesRow[]): { max: number; bars: Geometry[] } {
	return useMemo(() => {
		const max = rows.reduce((peak, row) => Math.max(peak, row.value), 0) || 1;
		const slot = rows.length > 0 ? CHART_WIDTH / rows.length : CHART_WIDTH;
		const width = Math.max(slot * 0.6, 4);
		const bars = rows.map((row, index) => {
			const height = Math.max((row.value / max) * (CHART_HEIGHT - 24), 2);
			const x = index * slot + (slot - width) / 2;
			return {
				x,
				y: CHART_HEIGHT - height,
				width,
				height,
				centerX: x + width / 2,
			};
		});
		return { max, bars };
	}, [rows]);
}

function SalesChart() {
	const props = useWidgetProps(bridge);
	const rows = Array.isArray(props.rows) ? props.rows : [];
	const [selectedIndex, setSelectedIndex] = useState(-1);
	const stateRef = useRef({ rows, selectedIndex });
	stateRef.current = { rows, selectedIndex };

	const { bars } = useGeometry(rows);
	const total = useMemo(
		() => rows.reduce((sum, row) => sum + row.value, 0),
		[rows],
	);

	// A flow-driven `props:update` can shrink or replace the series — never keep
	// pointing at a bucket that no longer exists.
	useEffect(() => {
		setSelectedIndex((current) => (current < rows.length ? current : -1));
	}, [rows.length]);

	useEffect(() => {
		const stop = [
			bridge.onQuery("getSelection", () => {
				const { rows: live, selectedIndex: index } = stateRef.current;
				const row = index >= 0 ? live[index] : undefined;
				return row
					? { label: row.label, value: row.value, index, selected: true }
					: { label: "", value: 0, index: -1, selected: false };
			}),
			bridge.onQuery("getSeries", ({ top }) =>
				[...stateRef.current.rows]
					.sort((a, b) => b.value - a.value)
					.slice(0, Math.max(top, 0)),
			),
			bridge.onQuery("getTotal", () =>
				stateRef.current.rows.reduce((sum, row) => sum + row.value, 0),
			),
		];
		return () => {
			for (const dispose of stop) dispose();
		};
	}, []);

	// Mirror state so `Query Widget` still answers value-style reads when the
	// surface is not live (headless runs replay this from the elements payload).
	useEffect(() => {
		const row = selectedIndex >= 0 ? rows[selectedIndex] : undefined;
		bridge.setValues({
			total,
			count: rows.length,
			selection: row
				? {
						label: row.label,
						value: row.value,
						index: selectedIndex,
						selected: true,
					}
				: { label: "", value: 0, index: -1, selected: false },
		});
	}, [rows, selectedIndex, total]);

	const select = (index: number) => {
		const row = rows[index];
		if (!row) return;
		setSelectedIndex(index);
		bridge.emit("pointSelected", { label: row.label, value: row.value, index });
	};

	const highlighted = (row: SalesRow, index: number) =>
		index === selectedIndex ||
		(props.highlight !== "" && row.label === props.highlight);

	return (
		<main style={styles.main}>
			<header style={styles.header}>
				<h1 style={styles.title}>{props.title}</h1>
				<span style={styles.total}>
					{rows.length} buckets · {formatValue(total, props.currency)}
				</span>
			</header>

			{rows.length === 0 ? (
				<div style={styles.empty}>Waiting for rows from the flow…</div>
			) : (
				<svg
					style={styles.chart}
					viewBox={`0 0 ${CHART_WIDTH} ${CHART_HEIGHT + 20}`}
					preserveAspectRatio="none"
					role="img"
					aria-label={props.title}
				>
					<title>{props.title}</title>
					{props.variant === "line" && bars.length > 1 && (
						<polyline
							points={bars.map((bar) => `${bar.centerX},${bar.y}`).join(" ")}
							fill="none"
							stroke="var(--primary)"
							strokeWidth={2}
						/>
					)}
					{bars.map((bar, index) => {
						const row = rows[index];
						if (!row) return null;
						const active = highlighted(row, index);
						return (
							<g
								key={row.label}
								// biome-ignore lint/a11y/useSemanticElements: SVG has no semantic button element
								role="button"
								tabIndex={0}
								aria-label={`${row.label}: ${formatValue(row.value, props.currency)}`}
								aria-pressed={index === selectedIndex}
								onClick={() => select(index)}
								onKeyDown={(event) => {
									if (event.key === "Enter" || event.key === " ") {
										event.preventDefault();
										select(index);
									}
								}}
								style={{ cursor: "pointer" }}
							>
								{props.variant === "line" ? (
									<circle
										cx={bar.centerX}
										cy={bar.y}
										r={active ? 6 : 4}
										fill={active ? "var(--primary)" : "var(--muted-foreground)"}
									/>
								) : (
									<rect
										x={bar.x}
										y={bar.y}
										width={bar.width}
										height={bar.height}
										rx={4}
										fill={active ? "var(--primary)" : "var(--muted)"}
										stroke={active ? "var(--primary)" : "var(--border)"}
									/>
								)}
							</g>
						);
					})}
					{bars.map((bar, index) => {
						const row = rows[index];
						if (!row) return null;
						return (
							<text
								key={`${row.label}-label`}
								x={bar.centerX}
								y={CHART_HEIGHT + 14}
								textAnchor="middle"
								fontSize={11}
								fill="var(--muted-foreground)"
							>
								{row.label}
							</text>
						);
					})}
				</svg>
			)}

			<footer style={styles.footer}>
				<span>
					{selectedIndex >= 0 && rows[selectedIndex]
						? `Selected ${rows[selectedIndex].label} · ${formatValue(rows[selectedIndex].value, props.currency)}`
						: "Click a bucket to select it"}
				</span>
				<button
					type="button"
					style={styles.button}
					onClick={() => bridge.emit("refreshRequested")}
				>
					Refresh
				</button>
			</footer>
		</main>
	);
}

const root = document.getElementById("root");
if (root) {
	createRoot(root).render(<SalesChart />);
}
