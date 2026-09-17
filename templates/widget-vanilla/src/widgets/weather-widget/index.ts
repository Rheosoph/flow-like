import { mountFlowWidget } from "@flow-like/widget-sdk";
import { createWeatherLoader, describeWeatherState } from "./weather";
import widget from "./widget.config";

const bridge = mountFlowWidget(widget);
const { $props, emit } = bridge;

const root = document.getElementById("root");
if (root) {
	const main = document.createElement("main");
	const title = document.createElement("h1");
	const reading = document.createElement("p");
	const refresh = document.createElement("button");
	refresh.type = "button";
	refresh.textContent = "Refresh";

	const loader = createWeatherLoader({
		onState: (state) => {
			reading.textContent = describeWeatherState(state);
		},
		onLoaded: (result) => emit("loaded", result),
	});

	let lastLocation = "";
	$props.subscribe((props) => {
		title.textContent = props.label;
		const location = `${props.latitude},${props.longitude}`;
		if (location !== lastLocation) {
			lastLocation = location;
			loader.load(props.latitude, props.longitude);
		}
	});

	refresh.addEventListener("click", () => {
		const { latitude, longitude } = $props.get();
		loader.load(latitude, longitude);
	});

	main.append(title, reading, refresh);
	root.append(main);
}
