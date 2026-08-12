import { mount } from "svelte";
import Widget from "./Widget.svelte";

const root = document.getElementById("root");
if (root) {
	mount(Widget, { target: root });
}
