import { defineWidget } from "@flow-like/widget-sdk";

interface Inputs {
	/** Place name shown above the reading @default "Berlin" */
	label: string;
	/** Latitude in degrees @minimum -90 @maximum 90 @default 52.52 */
	latitude: number;
	/** Longitude in degrees @minimum -180 @maximum 180 @default 13.41 */
	longitude: number;
}

interface Events {
	/** Fired after every successful reading */
	loaded: { temperature: number; unit: string };
}

export default defineWidget<Inputs, Events>({
	id: "weather-widget",
	name: "Weather Widget",
	description:
		"Preact widget that calls an external API: current temperature from Open-Meteo.",
	sizing: { defaultHeight: 200, resizable: true },
	csp: {
		connectSrc: ["https://api.open-meteo.com"],
	},
});
