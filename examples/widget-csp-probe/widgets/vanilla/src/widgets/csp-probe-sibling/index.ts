import { mountFlowWidget } from "@flow-like/widget-sdk";
import { createSiblingBlobs } from "../../lib/blobs";
import { navigationMarker, readDocumentInfo } from "../../lib/location";
import { reachedReport, whenConnected } from "../../lib/reached";
import { renderBanner } from "../../lib/view";
import widget from "./widget.config";

const bridge = mountFlowWidget(widget);
const blobUrls = createSiblingBlobs();
bridge.onQuery("getBlobUrls", () => blobUrls);

const root = document.getElementById("root");

if (root) {
	const main = document.createElement("main");
	const label = document.createElement("p");
	label.className = "label";
	bridge.$props.subscribe((props) => {
		label.textContent = props.label;
	});
	const blobs = document.createElement("pre");
	blobs.className = "blobs";
	blobs.textContent = JSON.stringify(Object.values(blobUrls), null, 2);
	const hint = document.createElement("p");
	hint.className = "label";
	hint.textContent =
		"Copy this array into the probe's foreignBlobUrls input, or forward the blobUrls event with a flow. The URLs live until this widget unmounts.";
	main.append(label, blobs, hint);
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
	} else {
		void whenConnected(bridge).then(() => {
			if (bridge.$mode.get() === "hosted") bridge.emit("blobUrls", blobUrls);
		});
	}
}
