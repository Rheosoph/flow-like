import { mountFlowWidget } from "@flow-like/widget-sdk";
import { useWidgetProps } from "@flow-like/widget-sdk/react";
import { type CSSProperties, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import widget, { type FilterValue } from "./widget.config";

const bridge = mountFlowWidget(widget);

const styles: Record<string, CSSProperties> = {
	main: { display: "grid", gap: "0.75rem", padding: "1rem" },
	title: { margin: 0, fontSize: "1.125rem", fontWeight: 600 },
	row: { display: "flex", gap: "0.75rem", flexWrap: "wrap" },
	field: { display: "grid", gap: "0.25rem", flex: "1 1 8rem" },
	labelText: { fontSize: "0.75rem", color: "var(--muted-foreground)" },
	input: {
		background: "var(--background)",
		color: "var(--foreground)",
		border: "1px solid var(--input)",
		borderRadius: "var(--radius)",
		padding: "0.4rem 0.6rem",
		font: "inherit",
		width: "100%",
		boxSizing: "border-box",
	},
	chips: { display: "flex", gap: "0.4rem", flexWrap: "wrap" },
	chip: {
		border: "1px solid var(--border)",
		borderRadius: "999px",
		padding: "0.3rem 0.7rem",
		font: "inherit",
		fontSize: "0.8125rem",
		cursor: "pointer",
	},
	actions: { display: "flex", gap: "0.5rem", alignItems: "center" },
	primary: {
		background: "var(--primary)",
		color: "var(--primary-foreground)",
		border: "1px solid var(--primary)",
		borderRadius: "var(--radius)",
		padding: "0.45rem 0.9rem",
		font: "inherit",
		cursor: "pointer",
	},
	ghost: {
		background: "transparent",
		color: "var(--foreground)",
		border: "1px solid var(--border)",
		borderRadius: "var(--radius)",
		padding: "0.45rem 0.9rem",
		font: "inherit",
		cursor: "pointer",
	},
	hint: { fontSize: "0.75rem", color: "var(--muted-foreground)" },
};

function toNumber(raw: string): number {
	const parsed = Number.parseFloat(raw);
	return Number.isFinite(parsed) && parsed >= 0 ? parsed : 0;
}

function FilterPanel() {
	const props = useWidgetProps(bridge);
	const categories = Array.isArray(props.categories) ? props.categories : [];

	const [value, setValue] = useState<FilterValue>({
		min: props.min,
		max: props.max,
		categories: Array.isArray(props.selected) ? props.selected : [],
	});
	const valueRef = useRef(value);
	valueRef.current = value;

	// Re-seed from the flow whenever `Update Widget Inputs` pushes new bounds or
	// a new selection; keyed on content so unrelated prop patches don't reset it.
	const seed = JSON.stringify([props.min, props.max, props.selected]);
	// biome-ignore lint/correctness/useExhaustiveDependencies: seed is the content key for these props
	useEffect(() => {
		setValue({
			min: props.min,
			max: props.max,
			categories: Array.isArray(props.selected) ? props.selected : [],
		});
	}, [seed]);

	useEffect(() => {
		const stop = [
			bridge.onQuery("getValue", () => valueRef.current),
			bridge.onQuery(
				"getCategoryCount",
				() => valueRef.current.categories.length,
			),
		];
		return () => {
			for (const dispose of stop) dispose();
		};
	}, []);

	// Publish on every change so headless runs can still read the filter through
	// the mirrored `{instanceId}/values` payload.
	useEffect(() => {
		bridge.setValues({ value, ...value });
	}, [value]);

	const toggleCategory = (category: string) => {
		setValue((current) => ({
			...current,
			categories: current.categories.includes(category)
				? current.categories.filter((entry) => entry !== category)
				: [...current.categories, category],
		}));
	};

	const reset = () => {
		setValue({ min: props.min, max: props.max, categories: [] });
		bridge.emit("resetRequested");
	};

	return (
		<main style={styles.main}>
			<h1 style={styles.title}>{props.label}</h1>

			<div style={styles.row}>
				<label style={styles.field}>
					<span style={styles.labelText}>Min revenue</span>
					<input
						style={styles.input}
						type="number"
						min={0}
						value={value.min}
						onChange={(event) =>
							setValue((current) => ({
								...current,
								min: toNumber(event.target.value),
							}))
						}
					/>
				</label>
				<label style={styles.field}>
					<span style={styles.labelText}>Max revenue (0 = no cap)</span>
					<input
						style={styles.input}
						type="number"
						min={0}
						value={value.max}
						onChange={(event) =>
							setValue((current) => ({
								...current,
								max: toNumber(event.target.value),
							}))
						}
					/>
				</label>
			</div>

			{categories.length > 0 && (
				<div style={styles.chips}>
					{categories.map((category) => {
						const active = value.categories.includes(category);
						return (
							<button
								key={category}
								type="button"
								onClick={() => toggleCategory(category)}
								style={{
									...styles.chip,
									background: active ? "var(--primary)" : "transparent",
									color: active
										? "var(--primary-foreground)"
										: "var(--foreground)",
									borderColor: active ? "var(--primary)" : "var(--border)",
								}}
							>
								{category}
							</button>
						);
					})}
				</div>
			)}

			<div style={styles.actions}>
				<button
					type="button"
					style={styles.primary}
					onClick={() => bridge.emit("applied", value)}
				>
					Apply filter
				</button>
				<button type="button" style={styles.ghost} onClick={reset}>
					Reset
				</button>
				<span style={styles.hint}>
					{value.categories.length === 0
						? "All categories"
						: `${value.categories.length} of ${categories.length} categories`}
				</span>
			</div>
		</main>
	);
}

const root = document.getElementById("root");
if (root) {
	createRoot(root).render(<FilterPanel />);
}
