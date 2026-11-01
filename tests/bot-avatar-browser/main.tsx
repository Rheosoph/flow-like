import { useState } from "react";
import { createRoot } from "react-dom/client";
import {
	type BotAppearance,
	BotAvatar,
	BotCustomizer,
	canSaveBotAppearance,
	createBotCatalog,
	defaultBotCatalog,
} from "../../packages/ui/components/bots";
import "./preview.css";
import { ColorProbes } from "./color-probes";
import { BotEffects } from "./effects";
import { BotFitReview } from "./fit-review";
import { BotGallery } from "./gallery";
import { BotMoods } from "./moods";
import { BotWardrobe } from "./wardrobe";

const parameters = new URLSearchParams(location.search);
const probeCatalog = createBotCatalog({
	shapes: [
		{
			...defaultBotCatalog.shapes.mochi,
			id: "pupil-probe",
			palette: {
				body: "#ffffff",
				background: "#ffffff",
				sclera: "#ffffff",
				ink: "#000000",
				cheek: "#ffffff",
				clothing: "#ffffff",
				accent: "#ffffff",
			},
			anchors: {
				...defaultBotCatalog.shapes.mochi.anchors,
				eyes: { left: [35, 50], right: [65, 50], radius: 3 },
			},
			mouthStyle: "none",
			cheeks: false,
			renderDetails: undefined,
		},
	],
});

function App() {
	const initialKind = parameters.get("kind") ?? "flux";
	const [value, setValue] = useState<BotAppearance>({
		kind: Object.hasOwn(defaultBotCatalog.shapes, initialKind)
			? initialKind
			: "flux",
	});
	const [requestedPack, setRequestedPack] = useState("");
	const [saved, setSaved] = useState<BotAppearance | null>(null);
	const owned =
		parameters.get("owned") === "all"
			? [
					...Object.keys(defaultBotCatalog.packs),
					...Object.keys(defaultBotCatalog.colorPacks),
				]
			: parameters.has("owned")
				? (parameters.get("owned") ?? "").split(",").filter(Boolean)
				: undefined;
	const sheet = parameters.has("crew");
	if (parameters.has("paints"))
		return <ColorProbes paused={parameters.has("paused")} />;
	if (parameters.has("gallery")) return <BotGallery parameters={parameters} />;
	if (parameters.has("effects")) return <BotEffects parameters={parameters} />;
	if (parameters.has("fit")) return <BotFitReview parameters={parameters} />;
	if (parameters.has("moods")) return <BotMoods parameters={parameters} />;
	if (parameters.has("wardrobe"))
		return <BotWardrobe parameters={parameters} />;
	if (parameters.has("review"))
		return (
			<main className="review-grid">
				{[
					"orbi",
					"pebble",
					"ember",
					"magma",
					"piyo",
					"floppy",
					"chip",
					"bit",
					"puri",
					"slate",
				].map((kind) => (
					<figure key={kind} data-review={kind}>
						<BotAvatar
							appearance={{
								kind,
								frame:
									parameters.get("frame") === "square" ? "square" : "circle",
							}}
							pose={parameters.has("sleep") ? "sleep" : "center"}
							animated={!parameters.has("paused")}
							size={220}
						/>
						<figcaption>{defaultBotCatalog.shapes[kind].label}</figcaption>
					</figure>
				))}
			</main>
		);
	if (parameters.has("probes"))
		return (
			<div className="probes">
				<div data-probe="pupils">
					<BotAvatar
						catalog={probeCatalog}
						appearance={{ kind: "pupil-probe", frame: "square" }}
						pose="track"
						gx={1}
						gy={1}
						animated={false}
						size={300}
					/>
				</div>
				<div data-probe="gold">
					<BotAvatar
						appearance={{ kind: "astro", variant: "gold", frame: "square" }}
						animated={false}
						size={300}
					/>
				</div>
				<div data-probe="hami">
					<BotAvatar
						appearance={{ kind: "hami", frame: "square" }}
						animated={false}
						size={300}
					/>
				</div>
				<div data-probe="lava">
					<BotAvatar
						appearance={{ kind: "lava", frame: "square" }}
						pose="center"
						size={300}
					/>
				</div>
			</div>
		);
	return (
		<main>
			<header>
				<span className="eyebrow">FLOW-LIKE / COMPANIONS</span>
				<h1>{sheet ? "Meet the crew" : "Find a crew. Make it yours."}</h1>
				<p>
					{Object.keys(defaultBotCatalog.packs).length} character packs, with a
					shared wardrobe for every bot.
				</p>
				<a href="/?gallery">Explore the animated gallery</a>
				{" · "}
				<a href="/?wardrobe">
					Browse all {Object.keys(defaultBotCatalog.equipment).length} outfit
					items
				</a>
			</header>
			{sheet ? (
				<div className="crew-sheet">
					{Object.values(defaultBotCatalog.packs)
						.filter(
							(pack) =>
								!parameters.has("pack") ||
								parameters.getAll("pack").includes(pack.id),
						)
						.map((pack) => (
							<section key={pack.id}>
								<h2>{pack.label}</h2>
								<p>{pack.description}</p>
								<div className="crew-row">
									{pack.shapeIds.map((kind) => (
										<figure key={kind}>
											<BotAvatar
												size={120}
												appearance={{ kind }}
												animated={false}
											/>
											<figcaption>
												{defaultBotCatalog.shapes[kind].label}
											</figcaption>
										</figure>
									))}
								</div>
							</section>
						))}
				</div>
			) : (
				<BotCustomizer
					value={value}
					onChange={setValue}
					onSave={(next) => {
						if (!canSaveBotAppearance(next, defaultBotCatalog, owned))
							throw new Error("Unlock the selected packs before saving.");
						setSaved(structuredClone(next));
					}}
					previewSize={220}
					ownedPackIds={owned}
					onRequestPack={(pack) => setRequestedPack(pack.id)}
					onRequestColorPack={(pack) => setRequestedPack(pack.id)}
				/>
			)}
			<output data-requested-pack aria-live="polite">
				{requestedPack ? `Selected pack: ${requestedPack}` : ""}
			</output>
			<output data-appearance hidden>
				{JSON.stringify(value)}
			</output>
			<output data-saved-appearance hidden>
				{JSON.stringify(saved)}
			</output>
			<output data-save-status aria-live="polite">
				{saved
					? `Saved ${defaultBotCatalog.shapes[saved.kind].label} for this preview.`
					: ""}
			</output>
		</main>
	);
}

const root = document.getElementById("root");
if (!root) throw new Error("Missing preview root");
createRoot(root).render(<App />);
