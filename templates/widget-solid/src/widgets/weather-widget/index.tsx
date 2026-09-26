import { mountFlowWidget } from "@flow-like/widget-sdk";
import { useStore } from "@nanostores/solid";
import { type JSX, createEffect, createSignal, on, onCleanup } from "solid-js";
import { render } from "solid-js/web";
import {
	type WeatherState,
	createWeatherLoader,
	describeWeatherState,
} from "./weather";
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
	reading: { margin: "0", color: "var(--muted-foreground)" },
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

function WeatherWidget() {
	const props = useStore(bridge.$props);
	const [weather, setWeather] = createSignal<WeatherState>({
		status: "loading",
	});
	const loader = createWeatherLoader({
		onState: (next) => setWeather(next),
		onLoaded: (reading) => bridge.emit("loaded", reading),
	});

	createEffect(
		on([() => props().latitude, () => props().longitude], ([lat, lon]) =>
			loader.load(lat, lon),
		),
	);

	onCleanup(() => loader.dispose());

	const refresh = () => loader.load(props().latitude, props().longitude);

	return (
		<main style={styles.main}>
			<h1 style={styles.title}>{props().label}</h1>
			<p style={styles.reading}>{describeWeatherState(weather())}</p>
			<button type="button" style={styles.button} onClick={refresh}>
				Refresh
			</button>
		</main>
	);
}

const root = document.getElementById("root");
if (root) {
	render(() => <WeatherWidget />, root);
}
