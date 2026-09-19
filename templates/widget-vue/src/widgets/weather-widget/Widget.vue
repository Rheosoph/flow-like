<script setup lang="ts">
import { mountFlowWidget } from "@flow-like/widget-sdk";
import { useStore } from "@nanostores/vue";
import { computed, onBeforeUnmount, ref, watch } from "vue";
import {
	type WeatherState,
	createWeatherLoader,
	describeWeatherState,
} from "./weather";
import widget from "./widget.config";

const bridge = mountFlowWidget(widget);
const props = useStore(bridge.$props);
const weather = ref<WeatherState>({ status: "loading" });
const reading = computed(() => describeWeatherState(weather.value));

const loader = createWeatherLoader({
	onState: (next) => {
		weather.value = next;
	},
	onLoaded: (reading) => bridge.emit("loaded", reading),
});

watch(
	[() => props.value.latitude, () => props.value.longitude],
	([latitude, longitude]) => loader.load(latitude, longitude),
	{ immediate: true },
);

onBeforeUnmount(() => loader.dispose());

function refresh() {
	loader.load(props.value.latitude, props.value.longitude);
}
</script>

<template>
	<main>
		<h1>{{ props.label }}</h1>
		<p>{{ reading }}</p>
		<button type="button" @click="refresh">Refresh</button>
	</main>
</template>

<style scoped>
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
