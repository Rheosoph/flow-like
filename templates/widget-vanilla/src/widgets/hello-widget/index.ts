import { mountFlowWidget } from "@flow-like/widget-sdk";
import widget from "./widget.config";

const bridge = mountFlowWidget(widget);
const { $props, emit, onQuery } = bridge;

let count = $props.get().count;
let lastHostCount = count;

const root = document.getElementById("root");
if (root) {
	const main = document.createElement("main");
	const title = document.createElement("h1");
	const button = document.createElement("button");
	button.type = "button";

	const renderCount = () => {
		button.textContent = `Count: ${count}`;
	};

	$props.subscribe((props) => {
		title.textContent = props.title;
		if (props.count !== lastHostCount) {
			lastHostCount = props.count;
			count = props.count;
			renderCount();
		}
	});

	button.addEventListener("click", () => {
		count += 1;
		renderCount();
		emit("increased", { value: count });
	});

	onQuery("getCount", () => count);

	renderCount();
	main.append(title, button);
	root.append(main);
}
