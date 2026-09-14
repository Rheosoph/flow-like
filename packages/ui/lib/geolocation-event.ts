import { geometryMarker, normalizeGeometryValue } from "./geometry";

export interface GeolocationEventConfig {
	sink_type: "geolocation";
	latitude: number;
	longitude: number;
	radius: number;
	trigger_on: "Enter" | "Exit" | "Both";
	background: boolean;
}

export function validateGeolocationEvent(
	config: Record<string, unknown>,
): string | undefined {
	if (
		typeof config.latitude !== "number" ||
		!Number.isFinite(config.latitude) ||
		config.latitude < -90 ||
		config.latitude > 90 ||
		typeof config.longitude !== "number" ||
		!Number.isFinite(config.longitude) ||
		config.longitude < -180 ||
		config.longitude > 180
	)
		return "Choose a valid region center with longitude from -180 to 180 and latitude from -90 to 90.";
	if (
		typeof config.radius !== "number" ||
		!Number.isFinite(config.radius) ||
		config.radius < 100 ||
		config.radius > 100_000
	)
		return "Region radius must be between 100 and 100000 meters.";
	if (!["Enter", "Exit", "Both"].includes(String(config.trigger_on ?? "Both")))
		return "Choose whether this Event fires on entering, leaving, or both transitions.";
	if (config.background !== undefined && typeof config.background !== "boolean")
		return "Background monitoring must be explicitly enabled or disabled.";
}

export function geofenceCenter(
	config: Record<string, unknown>,
): GeoJSON.Point | null {
	if (
		typeof config.longitude !== "number" ||
		typeof config.latitude !== "number"
	)
		return null;
	try {
		return normalizeGeometryValue(
			{ type: "Point", coordinates: [config.longitude, config.latitude] },
			{ schema: geometryMarker("Point"), allowUnset: false },
		) as GeoJSON.Point;
	} catch {
		return null;
	}
}

export function geofenceCenterFields(
	value: unknown,
): Pick<GeolocationEventConfig, "latitude" | "longitude"> {
	const point = normalizeGeometryValue(value, {
		schema: geometryMarker("Point"),
		allowUnset: false,
	}) as GeoJSON.Point;
	if (point.coordinates.length !== 2)
		throw new Error("A region center needs longitude and latitude only.");
	return { longitude: point.coordinates[0], latitude: point.coordinates[1] };
}
