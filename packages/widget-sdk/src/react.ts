import { useStore } from "@nanostores/react";
import type {
	WidgetEventsShape,
	WidgetInputsShape,
	WidgetQueriesShape,
} from "./define";
import type { BridgeMode, WidgetBridge } from "./mount";
import type { ThemeState } from "./protocol";

export function useWidgetProps<
	I extends WidgetInputsShape,
	E extends WidgetEventsShape,
	Q extends WidgetQueriesShape,
>(bridge: WidgetBridge<I, E, Q>): I {
	return useStore(bridge.$props);
}

export function useWidgetTheme<
	I extends WidgetInputsShape,
	E extends WidgetEventsShape,
	Q extends WidgetQueriesShape,
>(bridge: WidgetBridge<I, E, Q>): ThemeState {
	return useStore(bridge.$theme);
}

export function useWidgetMode<
	I extends WidgetInputsShape,
	E extends WidgetEventsShape,
	Q extends WidgetQueriesShape,
>(bridge: WidgetBridge<I, E, Q>): BridgeMode {
	return useStore(bridge.$mode);
}

export { useStore };
