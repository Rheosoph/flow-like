<script lang="ts">
import { mountFlowWidget } from "@flow-like/widget-sdk";
import { onDestroy } from "svelte";
import {
	type WeatherState,
	createWeatherLoader,
	describeWeatherState,
} from "./weather";
import widget from "./widget.config";

const bridge = mountFlowWidget(widget);
const widgetProps = bridge.$props;

let weather = $state<WeatherState>({ status: "loading" });

const loader = createWeatherLoader({
	onState: (next) => {
		weather = next;
	},
	onLoaded: (reading) => bridge.emit("loaded", reading),
});

const reading = $derived(describeWeatherState(weather));
const latitude = $derived($widgetProps.latitude);
const longitude = $derived($widgetProps.longitude);

$effect(() => {
	loader.load(latitude, longitude);
});

onDestroy(() => loader.dispose());

function refresh() {
	loader.load(latitude, longitude);
}
</script>

<main>
	<h1>{$widgetProps.label}</h1>
	<p>{reading}</p>
	<button type="button" onclick={refresh}>Refresh</button>
</main>

<style>
	main {
		display: grid;
		gap: 0.75rem;
		justify-items: start;
		padding: 1rem;
	}

	h1 {
		margin: 0;
		font-size: 1.25rem;
	}

	p {
		margin: 0;
		color: var(--muted-foreground);
	}

	button {
		background: var(--primary);
		color: var(--primary-foreground);
		border: 1px solid var(--border);
		border-radius: var(--radius);
		padding: 0.5rem 1rem;
		font: inherit;
		cursor: pointer;
	}
</style>
