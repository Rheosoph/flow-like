import { mountFlowWidget } from "@flow-like/widget-sdk";
import { useWidgetProps } from "@flow-like/widget-sdk/react";
import { type CSSProperties, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import {
	type WeatherState,
	createWeatherLoader,
	describeWeatherState,
} from "./weather";
import widget from "./widget.config";

const bridge = mountFlowWidget(widget);

const styles: Record<string, CSSProperties> = {
	main: {
		display: "grid",
		gap: "0.75rem",
		justifyItems: "start",
		padding: "1rem",
	},
	title: { margin: 0, fontSize: "1.25rem" },
	reading: { margin: 0, color: "var(--muted-foreground)" },
	button: {
		background: "var(--primary)",
		color: "var(--primary-foreground)",
		border: "1px solid var(--border)",
		borderRadius: "var(--radius)",
		padding: "0.5rem 1rem",
		font: "inherit",
		cursor: "pointer",
	},
};

function WeatherWidget() {
	const props = useWidgetProps(bridge);
	const [weather, setWeather] = useState<WeatherState>({ status: "loading" });
	const [loader] = useState(() =>
		createWeatherLoader({
			onState: setWeather,
			onLoaded: (reading) => bridge.emit("loaded", reading),
		}),
	);

	useEffect(() => {
		loader.load(props.latitude, props.longitude);
	}, [loader, props.latitude, props.longitude]);

	useEffect(() => () => loader.dispose(), [loader]);

	return (
		<main style={styles.main}>
			<h1 style={styles.title}>{props.label}</h1>
			<p style={styles.reading}>{describeWeatherState(weather)}</p>
			<button
				type="button"
				style={styles.button}
				onClick={() => loader.load(props.latitude, props.longitude)}
			>
				Refresh
			</button>
		</main>
	);
}

const root = document.getElementById("root");
if (root) {
	createRoot(root).render(<WeatherWidget />);
}
