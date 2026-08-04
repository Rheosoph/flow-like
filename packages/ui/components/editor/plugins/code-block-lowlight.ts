import { all, createLowlight } from "lowlight";
import { DIRECTIVE_TYPES } from "./remark-directives";

/**
 * Fence languages routed to custom renderers (charts, admonitions, embeds).
 * They carry no grammar, and Plate re-runs its decorator on every render, so an
 * unregistered language floods the console with one warning per pass.
 */
const CUSTOM_FENCE_LANGUAGES = [
	"nivo",
	"plotly",
	"embed",
	"map",
	...DIRECTIVE_TYPES.map((type) => `directive-${type}`),
];

/** Mirrors highlight.js' own `plaintext` grammar, which `all` does not ship. */
const plaintext = () => ({
	name: "Plain text",
	disableAutodetect: true,
	contains: [],
});

export function createEditorLowlight() {
	const lowlight = createLowlight(all);
	lowlight.register("plaintext", plaintext);
	lowlight.registerAlias("plaintext", CUSTOM_FENCE_LANGUAGES);
	return lowlight;
}
