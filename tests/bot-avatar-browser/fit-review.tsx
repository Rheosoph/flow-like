import { useState } from "react";
import {
	BotAvatar,
	type BotEffect,
	type BotFrame,
	type BotPose,
	defaultBotCatalog,
} from "../../packages/ui/components/bots";
import "./fit-review.css";

const OUTFITS = [
	"sweater",
	"hoodie",
	"scarf",
	"bowtie",
	"office-blazer",
	"doctor-coat",
	"denim-jacket",
	"flow-track-jacket",
] as const;
const OVERVIEW_OUTFITS = ["sweater", "doctor-coat", "denim-jacket"] as const;
const POSES: readonly BotPose[] = ["center", "left", "right", "sleep", "idle"];

export function BotFitReview({ parameters }: { parameters: URLSearchParams }) {
	const requestedKind = parameters.get("kind") ?? "mochi";
	const [kind, setKind] = useState(
		Object.hasOwn(defaultBotCatalog.shapes, requestedKind)
			? requestedKind
			: "mochi",
	);
	const [overview, setOverview] = useState(parameters.has("overview"));
	const [packId, setPackId] = useState(parameters.get("pack") ?? "all");
	const requestedPose = parameters.get("pose") as BotPose;
	const [pose, setPose] = useState<BotPose>(
		POSES.includes(requestedPose) ? requestedPose : "center",
	);
	const [effect, setEffect] = useState<BotEffect>(
		parameters.get("effect") === "pixel" ? "pixel" : "none",
	);
	const [frame, setFrame] = useState<BotFrame>(
		parameters.get("frame") === "square" ? "square" : "circle",
	);
	const [playing, setPlaying] = useState(false);
	const shapes = Object.values(defaultBotCatalog.shapes);
	const currentIndex = shapes.findIndex((shape) => shape.id === kind);
	const currentShape = defaultBotCatalog.shapes[kind];
	const requestedOutfit = parameters.get("outfit");
	const detailedOutfits: readonly string[] =
		requestedOutfit &&
		Object.hasOwn(defaultBotCatalog.equipment, requestedOutfit) &&
		defaultBotCatalog.equipment[requestedOutfit].slot === "neck"
			? [...new Set([...OUTFITS, requestedOutfit])]
			: OUTFITS;

	function characterRow(character: string, compact: boolean) {
		const shape = defaultBotCatalog.shapes[character];
		return (
			<article
				key={character}
				data-fit-character={character}
				className={compact ? "fit-review-row" : "fit-review-detail"}
			>
				<div className="fit-review-name">
					<strong>{shape.label}</strong>
					<code>{character}</code>
					{compact && (
						<button
							type="button"
							onClick={() => {
								setKind(character);
								setOverview(false);
							}}
						>
							Inspect
						</button>
					)}
				</div>
				<div className="fit-review-outfits">
					{["none", ...(compact ? OVERVIEW_OUTFITS : detailedOutfits)].map(
						(outfit) => (
							<figure key={outfit} data-fit-equipment={outfit}>
								<BotAvatar
									appearance={{
										kind: character,
										frame,
										effect,
										equipment: outfit === "none" ? {} : { neck: outfit },
									}}
									pose={pose}
									size={compact ? 120 : 200}
									animated={playing}
								/>
								<figcaption>
									{outfit === "none"
										? "No outfit"
										: defaultBotCatalog.equipment[outfit].label}
								</figcaption>
							</figure>
						),
					)}
				</div>
			</article>
		);
	}

	return (
		<main className="fit-review" data-fit-review>
			<header>
				<span className="eyebrow">FLOW-LIKE / FIT REVIEW</span>
				<h1>Clothing and cutouts</h1>
				<p>
					Compare the bare silhouette with each garment, one character at a
					time.
				</p>
			</header>
			<fieldset className="gallery-controls" aria-label="Fit review controls">
				<label>
					Character
					<select
						aria-label="Fit character"
						value={kind}
						onChange={(event) => {
							setKind(event.target.value);
							setOverview(false);
						}}
					>
						{shapes.map((shape) => (
							<option key={shape.id} value={shape.id}>
								{shape.label} ({shape.id})
							</option>
						))}
					</select>
				</label>
				<label>
					Collection
					<select
						aria-label="Fit collection"
						value={packId}
						onChange={(event) => setPackId(event.target.value)}
					>
						<option value="all">All collections</option>
						{Object.values(defaultBotCatalog.packs).map((pack) => (
							<option key={pack.id} value={pack.id}>
								{pack.label}
							</option>
						))}
					</select>
				</label>
				<label>
					Pose
					<select
						aria-label="Fit pose"
						value={pose}
						onChange={(event) => setPose(event.target.value as BotPose)}
					>
						{POSES.map((value) => (
							<option key={value}>{value}</option>
						))}
					</select>
				</label>
				<label>
					Frame
					<select
						aria-label="Fit frame"
						value={frame}
						onChange={(event) => setFrame(event.target.value as BotFrame)}
					>
						{["circle", "squircle", "square"].map((value) => (
							<option key={value}>{value}</option>
						))}
					</select>
				</label>
				<label>
					Effect
					<select
						aria-label="Fit effect"
						value={effect}
						onChange={(event) => setEffect(event.target.value as BotEffect)}
					>
						<option value="none">None</option>
						<option value="pixel">Pixel</option>
					</select>
				</label>
				<label>
					<input
						type="checkbox"
						checked={overview}
						onChange={(event) => setOverview(event.target.checked)}
					/>
					Overview
				</label>
				<label>
					<input
						type="checkbox"
						checked={playing}
						onChange={(event) => setPlaying(event.target.checked)}
					/>
					Animate
				</label>
			</fieldset>
			{overview ? (
				<div className="fit-review-packs">
					{Object.values(defaultBotCatalog.packs)
						.filter((pack) => packId === "all" || pack.id === packId)
						.map((pack) => (
							<section key={pack.id} data-fit-pack={pack.id}>
								<h2>{pack.label}</h2>
								{pack.shapeIds.map((character) =>
									characterRow(character, true),
								)}
							</section>
						))}
				</div>
			) : (
				<>
					<nav
						className="fit-review-navigation"
						aria-label="Character review navigation"
					>
						<button
							type="button"
							disabled={currentIndex === 0}
							onClick={() => setKind(shapes[currentIndex - 1].id)}
						>
							Previous character
						</button>
						<span>
							{currentShape.label} · {currentIndex + 1} / {shapes.length}
						</span>
						<button
							type="button"
							disabled={currentIndex === shapes.length - 1}
							onClick={() => setKind(shapes[currentIndex + 1].id)}
						>
							Next character
						</button>
					</nav>
					{characterRow(kind, false)}
				</>
			)}
		</main>
	);
}
