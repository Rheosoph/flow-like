import { describe, expect, test } from "bun:test";
import { resolveInlineStyle, resolveStyle } from "./StyleResolver";
import { applyStyleUpdate, normalizeStyleUpdate } from "./style-updates";
import type { Style } from "./types";

describe("A2UI style resolution", () => {
	test("renders the canonical frontend gradient contract", () => {
		const style: Style = {
			background: {
				gradient: {
					type: "linear",
					angle: 45,
					stops: [
						{ color: "#ff0000", position: 0 },
						{ color: "#0000ff", position: 100 },
					],
				},
			},
		};

		expect(resolveInlineStyle(style).background).toBe(
			"linear-gradient(45deg, #ff0000 0%, #0000ff 100%)",
		);
	});

	test("renders legacy Rust gradient fields and fractional stops", () => {
		const style: Style = {
			background: {
				gradient: {
					gradientType: "conic",
					direction: "from 30deg",
					stops: [
						{ color: "red", position: 0 },
						{ color: "blue", position: 1 },
					],
				},
			},
		};

		expect(resolveInlineStyle(style).background).toBe(
			"conic-gradient(from 30deg, red 0%, blue 100%)",
		);
	});

	test("renders canonical and legacy spacing and size values", () => {
		const canonical: Style = {
			margin: { top: "8px", right: "16px", bottom: "4px", left: "2px" },
			padding: { top: "1rem" },
			width: "100%",
			minHeight: "240px",
		};
		const legacy: Style = {
			margin: { value: "8px 16px" },
			padding: { value: "1rem" },
			width: { value: "100%" },
		};

		expect(resolveInlineStyle(canonical)).toMatchObject({
			marginTop: "8px",
			marginRight: "16px",
			marginBottom: "4px",
			marginLeft: "2px",
			paddingTop: "1rem",
			width: "100%",
			minHeight: "240px",
		});
		expect(resolveInlineStyle(legacy)).toMatchObject({
			margin: "8px 16px",
			padding: "1rem",
			width: "100%",
		});
	});

	test("renders canonical position, shadow, transform, and responsive styles", () => {
		const style: Style = {
			position: { type: "absolute", top: "1rem", left: "2rem" },
			shadow: {
				x: "0",
				y: "2px",
				blur: "4px",
				spread: "0",
				color: "rgba(0,0,0,0.1)",
				inset: true,
				textShadow: "0 1px 2px #0004",
			},
			transform: { skew: "10deg, 5deg", transformOrigin: "top left" },
			responsiveOverrides: {
				md: {
					className: "p-4",
					hidden: true,
					gap: "1rem",
					gridCols: 3,
					padding: {
						top: "8px",
						right: "16px",
						bottom: "8px",
						left: "16px",
					},
				},
			},
		};

		expect(resolveInlineStyle(style)).toMatchObject({
			position: "absolute",
			top: "1rem",
			left: "2rem",
			boxShadow: "inset 0 2px 4px 0 rgba(0,0,0,0.1)",
			textShadow: "0 1px 2px #0004",
			transform: "skew(10deg, 5deg)",
			transformOrigin: "top left",
			"--a2ui-md-display": "none",
			"--a2ui-md-gap": "1rem",
			"--a2ui-md-grid-cols": "repeat(3, minmax(0, 1fr))",
			"--a2ui-md-padding": "8px 16px 8px 16px",
		});
		expect(resolveStyle(style)).toContain("md:p-4");
		expect(resolveStyle(style)).toContain("a2ui-md-display");
		expect(resolveStyle(style)).toContain("a2ui-md-grid-cols");
	});

	test("renders the legacy Rust position, shadow, and responsive names", () => {
		const style: Style = {
			position: { positionType: "fixed", right: "0" },
			shadow: {
				boxShadows: ["0 2px 4px #0003", "0 8px 20px #0002"],
			},
			responsive: {
				lg: { width: { value: "50%" }, margin: { value: "1rem auto" } },
			},
		};

		expect(resolveInlineStyle(style)).toMatchObject({
			position: "fixed",
			right: "0",
			boxShadow: "0 2px 4px #0003, 0 8px 20px #0002",
			"--a2ui-lg-width": "50%",
			"--a2ui-lg-margin": "1rem auto",
		});
	});

	test("normalizes legacy Rust fields back to the frontend contract", () => {
		const normalized = normalizeStyleUpdate({
			background: {
				gradient: {
					gradientType: "linear",
					direction: "to right",
					stops: [
						{ color: "red", position: 0 },
						{ color: "blue", position: 1 },
					],
				},
			},
			position: { positionType: "absolute", top: "0" },
			margin: { value: "8px 16px" },
			width: { value: "100%" },
			responsive: {
				md: { width: { value: "50%" }, padding: { value: "4px 8px" } },
			},
		});

		expect(normalized).toMatchObject({
			background: {
				gradient: {
					type: "linear",
					direction: "to right",
					stops: [
						{ color: "red", position: 0 },
						{ color: "blue", position: 100 },
					],
				},
			},
			position: { type: "absolute", top: "0" },
			margin: { top: "8px", right: "16px", bottom: "8px", left: "16px" },
			width: "100%",
			responsiveOverrides: {
				md: {
					width: "50%",
					padding: {
						top: "4px",
						right: "8px",
						bottom: "4px",
						left: "8px",
					},
				},
			},
		});
		expect(normalized).not.toHaveProperty("responsive");
	});

	test("normalizes existing legacy style when applying a partial update", () => {
		const result = applyStyleUpdate(
			{
				position: { positionType: "sticky", top: "0" },
				width: { value: "20rem" },
				responsive: { sm: { hidden: true } },
			},
			{ class_name: "rounded-lg" },
		);

		expect(result).toMatchObject({
			className: "rounded-lg",
			position: { type: "sticky", top: "0" },
			width: "20rem",
			responsiveOverrides: { sm: { hidden: true } },
		});
		expect(result).not.toHaveProperty("responsive");
	});

	test("uses a bound background image fallback until data is resolved", () => {
		const style: Style = {
			background: {
				image: {
					url: { path: "/theme/hero", defaultValue: "/fallback.jpg" },
					size: "contain",
				},
			},
		};

		expect(resolveInlineStyle(style)).toMatchObject({
			backgroundImage: "url(/fallback.jpg)",
			backgroundSize: "contain",
		});
	});
});
