import type { BotShapeDefinition } from "../types";
import { crewShape, ovalPath } from "./helpers";

export const basicsShapes: readonly BotShapeDefinition[] = [
	crewShape({
		id: "pip",
		label: "Pip",
		series: "basics",
		description: "A soft green circle with dot eyes.",
		palette: {
			body: "#83b88f",
			background: "#e1eee3",
			accent: "#b9d5a3",
			clothing: "#5f5a85",
		},
		silhouette: ovalPath(50, 76, 40, 39),
		anchors: { head: { x: 50, y: 38, width: 48 } },
	}),
	crewShape({
		id: "mochi",
		label: "Mochi",
		series: "basics",
		description: "A lavender squircle with wide round eyes.",
		palette: {
			body: "#a795e1",
			background: "#eee7ff",
			cheek: "#dbb4df",
			accent: "#e7bb80",
			clothing: "#586879",
		},
		eyeStyle: "round",
		cheeks: true,
		silhouette: "M14 66Q14 31 50 31Q86 31 86 66V84Q86 103 50 103Q14 103 14 84Z",
		anchors: {
			eyes: { left: [38, 62], right: [63, 62], radius: 8 },
			mouth: [50, 77],
			head: { x: 50, y: 32, width: 48 },
		},
	}),
	crewShape({
		id: "hexa",
		label: "Hexa",
		series: "basics",
		description: "A golden hexagon with relaxed half-closed eyes.",
		palette: {
			body: "#e9ba3c",
			background: "#fbf1ce",
			accent: "#f6db89",
			clothing: "#5d6481",
		},
		eyeStyle: "sleepy",
		silhouette:
			"M47 26Q50 24 53 26L87 47Q89 48 89 51V85L50 107L11 85V51Q11 48 14 46Z",
		anchors: {
			eyes: { left: [39, 62], right: [63, 62], radius: 8 },
			mouth: [51, 76],
			head: { x: 50, y: 29, width: 40 },
		},
	}),
	crewShape({
		id: "pebble",
		label: "Pebble",
		series: "basics",
		description: "A smooth rose-colored pebble with a quiet smile.",
		palette: {
			body: "#be7fa1",
			background: "#f3e1ec",
			accent: "#e9bd80",
			clothing: "#435c74",
		},
		silhouette:
			"M16 78C13 54 29 35 48 34C70 31 84 49 84 74L86 97Q52 114 16 98Z",
		anchors: {
			eyes: { left: [37, 66], right: [62, 66], radius: 3.1 },
			mouth: [50, 82],
			head: { x: 50, y: 35, width: 44 },
			neck: { y: 89, bottom: 105 },
		},
	}),
	crewShape({
		id: "nimbus",
		label: "Nimbus",
		series: "basics",
		description: "A blue-gray cloud with a curious expression.",
		palette: {
			body: "#93a4bf",
			background: "#e6e9f1",
			accent: "#f4cc7a",
			clothing: "#506483",
		},
		eyeStyle: "round",
		mouthStyle: "o",
		bodyMotion: "float",
		silhouette:
			"M18 94C7 88 9 65 28 60C29 34 62 29 72 59C93 59 98 82 86 95Q53 112 18 94Z",
		anchors: {
			eyes: { left: [39, 71], right: [62, 71], radius: 8 },
			mouth: [51, 85],
			head: { x: 50, y: 40, width: 38 },
			neck: { y: 88, bottom: 105 },
		},
	}),
];
