import type { WidgetContract } from "./contract";
import type { FlwStandaloneGlobal } from "./mount";

declare global {
	// Injected into the built HTML by @flow-like/widget-bundler; undefined in
	// unbundled dev.
	var __FLW_CONTRACT__: WidgetContract | undefined;
	// Devtools escape hatch registered by the SDK in standalone mode.
	var __flw: FlwStandaloneGlobal | undefined;
}
