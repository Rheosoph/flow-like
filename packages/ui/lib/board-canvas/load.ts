export interface BoardCanvasHandle {
	set_scene(scene: string): void;
	set_palette(palette: string): void;
	set_positions(positions: string): void;
	set_viewport(x: number, y: number, zoom: number): void;
	set_overlay(overlay: string): void;
	set_lod(enabled: boolean): void;
	set_layer(layer: "all" | "edges" | "nodes"): void;
	set_fonts(regular: Uint8Array, medium: Uint8Array): void;
	set_icon?(
		key: string,
		width: number,
		height: number,
		rgba: Uint8Array,
		tint: boolean,
	): void;
	/** `{ nodes: Record<id, NodeStatus>, motion }`, see `collectNodeStatus`. */
	set_status?(status: string): void;
	/** Each run's last activity text and tone, keyed by run id. */
	set_activity?(activity: string): void;
	pulse?(): void;
	stop_pulse?(): void;
	stats(): unknown;
	destroy(): void;
}

interface BoardCanvasModule {
	default(): Promise<unknown>;
	start(
		canvas: HTMLCanvasElement,
		scene: string,
		palette: string,
	): Promise<BoardCanvasHandle>;
}

let loading: Promise<BoardCanvasModule> | undefined;
export function loadBoardCanvas(): Promise<BoardCanvasModule> {
	if (!loading) {
		const modulePath = "/board-canvas/board_canvas.js";
		loading = import(/* webpackIgnore: true */ /* @vite-ignore */ modulePath)
			.then(async (module: BoardCanvasModule) => {
				await module.default();
				return module;
			})
			.catch((cause: unknown) => {
				loading = undefined;
				throw new Error(
					"The egui canvas could not be loaded. Run the board-canvas build and reload the app.",
					{ cause },
				);
			});
	}
	return loading;
}

let fonts: Promise<[Uint8Array, Uint8Array]> | undefined;
export function loadBoardCanvasFonts(): Promise<[Uint8Array, Uint8Array]> {
	fonts ??= Promise.all(
		["Inter-Regular.ttf", "Inter-Medium.ttf"].map(async (file) => {
			const response = await fetch(`/board-canvas/${file}`);
			if (!response.ok) throw new Error(`Cannot load canvas font ${file}`);
			return new Uint8Array(await response.arrayBuffer());
		}),
	)
		.then(([regular, medium]) => [regular, medium] as [Uint8Array, Uint8Array])
		.catch((error) => {
			fonts = undefined;
			throw error;
		});
	return fonts;
}
