import { mountFlowWidget } from "@flow-like/widget-sdk";
import { navigationMarker, readDocumentInfo } from "../../lib/location";
import { reachedReport, whenConnected } from "../../lib/reached";
import { renderBanner } from "../../lib/view";
import widget from "./widget.config";

const bridge = mountFlowWidget(widget);
const root = document.getElementById("root");

if (root) {
	const main = document.createElement("main");
	const label = document.createElement("p");
	label.className = "label";
	bridge.$props.subscribe((props) => {
		label.textContent = props.label;
	});
	main.append(label);
	root.append(main);

	const marker = navigationMarker();
	if (marker !== null) {
		const info = readDocumentInfo();
		renderBanner(
			main,
			`NAVIGATION REACHED: ${marker} loaded ${info.label}. This check failed.`,
		);
		void whenConnected(bridge).then(() =>
			bridge.emit("result", reachedReport(widget.id, marker, info)),
		);
	}
}
