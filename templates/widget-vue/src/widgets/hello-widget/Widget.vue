<script setup lang="ts">
import { mountFlowWidget } from "@flow-like/widget-sdk";
import { useStore } from "@nanostores/vue";
import { ref, watch } from "vue";
import widget from "./widget.config";

const bridge = mountFlowWidget(widget);
const props = useStore(bridge.$props);
const count = ref(bridge.$props.get().count);

watch(
	() => props.value.count,
	(next) => {
		count.value = next;
	},
);

bridge.onQuery("getCount", () => count.value);

function increase() {
	count.value += 1;
	bridge.emit("increased", { value: count.value });
}
</script>

<template>
	<main>
		<h1>{{ props.title }}</h1>
		<button type="button" @click="increase">Count: {{ count }}</button>
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
