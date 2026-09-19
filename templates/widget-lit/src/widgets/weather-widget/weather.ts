// Every origin this module fetches from must be listed in widget.config.ts
// under `csp.connectSrc`; the host blocks anything else.
const FORECAST_URL = "https://api.open-meteo.com/v1/forecast";

export interface WeatherReading {
	temperature: number;
	unit: string;
}

export type WeatherState =
	| { status: "loading" }
	| { status: "ready"; reading: WeatherReading }
	| { status: "error"; message: string };

interface WeatherLoaderOptions {
	onState: (state: WeatherState) => void;
	onLoaded: (reading: WeatherReading) => void;
}

interface ForecastResponse {
	current?: { temperature_2m?: unknown };
	current_units?: { temperature_2m?: unknown };
}

async function fetchCurrentTemperature(
	latitude: number,
	longitude: number,
	signal: AbortSignal,
): Promise<WeatherReading> {
	const url = new URL(FORECAST_URL);
	url.searchParams.set("latitude", latitude.toString());
	url.searchParams.set("longitude", longitude.toString());
	url.searchParams.set("current", "temperature_2m");

	const response = await fetch(url, { signal });
	if (!response.ok) {
		throw new Error(`Open-Meteo responded with HTTP ${response.status}`);
	}
	const body: ForecastResponse = await response.json();
	const temperature = body.current?.temperature_2m;
	if (typeof temperature !== "number") {
		throw new Error("Open-Meteo returned no temperature");
	}
	const unit = body.current_units?.temperature_2m;
	return { temperature, unit: typeof unit === "string" ? unit : "°C" };
}

function describeError(error: unknown): string {
	// A CSP block and an offline network both surface as a TypeError.
	if (error instanceof TypeError) {
		return "Could not reach api.open-meteo.com. In Flow-Like, allow this widget's network access.";
	}
	return error instanceof Error ? error.message : String(error);
}

export function createWeatherLoader({
	onState,
	onLoaded,
}: WeatherLoaderOptions) {
	let controller: AbortController | undefined;

	return {
		load(latitude: number, longitude: number) {
			controller?.abort();
			const current = new AbortController();
			controller = current;
			onState({ status: "loading" });

			fetchCurrentTemperature(latitude, longitude, current.signal).then(
				(reading) => {
					if (current.signal.aborted) return;
					onState({ status: "ready", reading });
					onLoaded(reading);
				},
				(error: unknown) => {
					if (current.signal.aborted) return;
					onState({ status: "error", message: describeError(error) });
				},
			);
		},
		dispose() {
			controller?.abort();
		},
	};
}

export function describeWeatherState(state: WeatherState): string {
	switch (state.status) {
		case "loading":
			return "Loading…";
		case "ready":
			return `${state.reading.temperature.toFixed(1)} ${state.reading.unit}`;
		case "error":
			return state.message;
	}
}
