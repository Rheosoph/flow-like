<script lang="ts">
import { mountFlowWidget } from "@flow-like/widget-sdk";
import widget from "./widget.config";

const bridge = mountFlowWidget(widget);
const widgetProps = bridge.$props;

let count = $state(widgetProps.get().count);
let lastHostCount = widgetProps.get().count;

$effect(() => {
	const next = $widgetProps.count;
	if (next !== lastHostCount) {
		lastHostCount = next;
		count = next;
	}
});

bridge.onQuery("getCount", () => count);

function increase() {
	count += 1;
	bridge.emit("increased", { value: count });
}
</script>

<main>
	<h1>{$widgetProps.title}</h1>
	<button type="button" onclick={increase}>Count: {count}</button>
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
