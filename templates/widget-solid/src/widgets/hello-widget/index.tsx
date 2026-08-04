import { mountFlowWidget } from "@flow-like/widget-sdk";
import { useStore } from "@nanostores/solid";
import { type JSX, createEffect, createMemo, createSignal, on } from "solid-js";
import { render } from "solid-js/web";
import widget from "./widget.config";

const bridge = mountFlowWidget(widget);

const styles: Record<string, JSX.CSSProperties> = {
	main: {
		display: "grid",
		gap: "0.75rem",
		"justify-items": "start",
		padding: "1rem",
	},
	title: { margin: "0", "font-size": "1.25rem" },
	button: {
		background: "var(--primary)",
		color: "var(--primary-foreground)",
		border: "1px solid var(--border)",
		"border-radius": "var(--radius)",
		padding: "0.5rem 1rem",
		font: "inherit",
		cursor: "pointer",
	},
};

function HelloWidget() {
	const props = useStore(bridge.$props);
	const [count, setCount] = createSignal(bridge.$props.get().count);
	const hostCount = createMemo(() => props().count);

	createEffect(on(hostCount, (next) => setCount(next), { defer: true }));

	bridge.onQuery("getCount", () => count());

	const increase = () => {
		const value = count() + 1;
		setCount(value);
		bridge.emit("increased", { value });
	};

	return (
		<main style={styles.main}>
			<h1 style={styles.title}>{props().title}</h1>
			<button type="button" style={styles.button} onClick={increase}>
				Count: {count()}
			</button>
		</main>
	);
}

const root = document.getElementById("root");
if (root) {
	render(() => <HelloWidget />, root);
}
