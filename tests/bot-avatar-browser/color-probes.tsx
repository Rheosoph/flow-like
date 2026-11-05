import {
	type BotAppearance,
	BotAvatar,
} from "../../packages/ui/components/bots";

const examples: readonly [string, string, BotAppearance][] = [
	[
		"solid",
		"Essentials",
		{
			kind: "mochi",
			colors: { body: { kind: "swatch", colorId: "color-coral" } },
		},
	],
	[
		"holo",
		"Aurora",
		{
			kind: "mochi",
			colors: { body: { kind: "swatch", colorId: "holo-aurora" } },
		},
	],
	[
		"pattern",
		"Starry night",
		{
			kind: "mochi",
			colors: { body: { kind: "swatch", colorId: "pattern-stars" } },
		},
	],
	[
		"flux-holo",
		"Flux · Aurora",
		{
			kind: "flux",
			colors: { body: { kind: "swatch", colorId: "holo-aurora" } },
		},
	],
	[
		"flux-accent",
		"Flux · patterned accent",
		{
			kind: "flux",
			colors: { accent: { kind: "swatch", colorId: "pattern-stars" } },
		},
	],
	[
		"lava-holo",
		"Lava · Aurora",
		{
			kind: "lava",
			colors: { body: { kind: "swatch", colorId: "holo-aurora" } },
		},
	],
	[
		"magma-holo",
		"Magma · Aurora",
		{
			kind: "magma",
			colors: { body: { kind: "swatch", colorId: "holo-aurora" } },
		},
	],
	[
		"slate-holo",
		"Slate · Aurora",
		{
			kind: "slate",
			colors: { body: { kind: "swatch", colorId: "holo-aurora" } },
		},
	],
	[
		"outfit",
		"Patterned outfit",
		{
			kind: "mochi",
			equipment: { neck: "hoodie", head: "cap" },
			colors: {
				clothing: { kind: "swatch", colorId: "pattern-stars" },
				background: { kind: "swatch", colorId: "pastel-mint" },
			},
		},
	],
	[
		"hex",
		"Custom hex",
		{
			kind: "mochi",
			colors: { body: { kind: "hex", value: "#31A896", packId: "color-lab" } },
		},
	],
	[
		"lcd",
		"LCD override",
		{
			kind: "mochi",
			effect: "lcd",
			colors: { body: { kind: "swatch", colorId: "holo-aurora" } },
		},
	],
];

export function ColorProbes({ paused = false }: { paused?: boolean }) {
	return (
		<main className="review-grid">
			{examples.map(([id, label, appearance]) => (
				<figure key={id} data-paint-probe={id}>
					<BotAvatar
						appearance={appearance}
						pose="center"
						size={220}
						animated={!paused}
					/>
					<figcaption>{label}</figcaption>
				</figure>
			))}
		</main>
	);
}
