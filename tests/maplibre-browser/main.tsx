import type * as GeoJSON from "geojson";
import * as maplibre from "maplibre-gl";
import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import GeometryEditorMap from "../../packages/ui/components/ui/geometry-editor-map";
import {
	MapClusterLayer,
	MapMarker,
	MapRoute,
	Map as MapView,
	MarkerContent,
	MarkerPopup,
	useMap,
} from "../../packages/ui/components/ui/map";
import type { GeometryDraft } from "../../packages/ui/lib/geometry-draft";

const maps = new Set<maplibre.Map>();
const errors: string[] = [];
const fire = maplibre.Map.prototype.fire;
maplibre.Map.prototype.fire = function (...args) {
	const type = typeof args[0] === "string" ? args[0] : args[0].type;
	if (type === "remove") maps.delete(this);
	else if (!maps.has(this)) {
		maps.add(this);
		this.on("error", (event) => errors.push(event.error.message));
	}
	return Reflect.apply(fire, this, args);
};

const style: maplibre.StyleSpecification = {
	version: 8,
	glyphs: `${location.origin}/glyphs/{fontstack}/{range}.pbf`,
	sources: {},
	layers: [
		{
			id: "background",
			type: "background",
			paint: { "background-color": "#e0e8f0" },
		},
	],
};
const points: GeoJSON.FeatureCollection<GeoJSON.Point> = {
	type: "FeatureCollection",
	features: [0, 0.01, 0.02].map((x) => ({
		type: "Feature",
		properties: { name: "point" },
		geometry: { type: "Point", coordinates: [x, 0] },
	})),
};
let mainMap: maplibre.Map | null = null;
function Capture() {
	const { map } = useMap();
	useEffect(() => {
		mainMap = map;
	}, [map]);
	return null;
}
let moveCount = 0;
let addedPosition = false;
function Fixture() {
	const [theme, setTheme] = useState<"light" | "dark">("light");
	const [updated, setUpdated] = useState(false);
	const [draft, setDraft] = useState<GeometryDraft>({
		kind: "Polygon",
		shapes: [
			[
				[
					[0, 0],
					[1, 0],
					[1, 1],
				],
			],
		],
	});
	return (
		<>
			<button
				type="button"
				id="theme"
				onClick={() => setTheme(theme === "light" ? "dark" : "light")}
			>
				Theme
			</button>
			<button type="button" id="update" onClick={() => setUpdated(true)}>
				Update route
			</button>
			<MapView
				theme={theme}
				center={[0, 0]}
				zoom={5}
				onViewportChange={() => moveCount++}
				styles={{
					light: style,
					dark: {
						...style,
						layers: [
							{
								id: "background",
								type: "background",
								paint: { "background-color": "#182030" },
							},
						],
					},
				}}
			>
				<Capture />
				<MapMarker longitude={-1} latitude={0}>
					<MarkerContent>
						<button type="button" id="marker">
							Marker
						</button>
					</MarkerContent>
					<MarkerPopup>Safe popup</MarkerPopup>
				</MapMarker>
				<MapRoute
					id="test"
					coordinates={
						updated
							? [
									[-2, -1],
									[2, 1],
								]
							: [
									[-1, -1],
									[1, 1],
								]
					}
				/>
				<MapClusterLayer data={points} />
			</MapView>
			<GeometryEditorMap
				draft={draft}
				active={{ shape: 0, ring: 0 }}
				onAddPosition={(position) => {
					addedPosition = true;
					setDraft({ ...draft, shapes: [[[...draft.shapes[0][0], position]]] });
				}}
				onMovePosition={() => {}}
				onSelectRing={() => {}}
			/>
		</>
	);
}
const container = document.getElementById("app");
if (!container) throw new Error("Missing fixture container");
const root = createRoot(container);
root.render(<Fixture />);
const mapTest = {
	get ready() {
		return mainMap !== null;
	},
	get map() {
		if (!mainMap) throw new Error("Map is not mounted");
		return mainMap;
	},
	maps,
	errors,
	style,
	get moveCount() {
		return moveCount;
	},
	get addedPosition() {
		return addedPosition;
	},
	maplibre,
	unmount: () => root.unmount(),
};
export type MapTest = typeof mapTest;
Object.assign(window, { mapTest });
