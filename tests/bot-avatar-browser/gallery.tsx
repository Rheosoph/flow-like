import { useState } from "react";
import {
	BOT_EXPRESSIONS,
	type BotAppearance,
	BotAvatar,
	type BotExpression,
	defaultBotCatalog,
} from "../../packages/ui/components/bots";

const outfits: Record<
	string,
	{ label: string; equipment: BotAppearance["equipment"] }
> = {
	classic: {
		label: "Classic",
		equipment: {
			head: "crown",
			eyes: "glasses",
			neck: "hoodie",
			hand: "laptop",
		},
	},
	police: {
		label: "Police",
		equipment: { head: "police-cap", neck: "police-uniform" },
	},
	fire: {
		label: "Fire crew",
		equipment: { head: "fire-helmet", neck: "fire-coat" },
	},
	field: {
		label: "Field crew",
		equipment: { head: "field-cap", neck: "field-jacket" },
	},
	rescue: { label: "Rescue", equipment: { neck: "rescue-vest" } },
	office: { label: "Office", equipment: { neck: "office-blazer" } },
	reception: {
		label: "Reception",
		equipment: { head: "reception-headset", neck: "reception-vest" },
	},
	lanyard: { label: "Lanyard", equipment: { neck: "office-lanyard" } },
	doctor: { label: "Doctor", equipment: { neck: "doctor-coat" } },
	nurse: { label: "Nurse", equipment: { neck: "nurse-tunic" } },
	scrubs: {
		label: "Scrubs",
		equipment: { head: "scrub-cap", neck: "scrub-top" },
	},
};

export function BotGallery({ parameters }: { parameters: URLSearchParams }) {
	const requestedExpression = parameters.get("expression") as BotExpression;
	const [expression, setExpression] = useState<BotExpression>(
		BOT_EXPRESSIONS.includes(requestedExpression)
			? requestedExpression
			: "neutral",
	);
	const [playing, setPlaying] = useState(!parameters.has("paused"));
	const [sleeping, setSleeping] = useState(parameters.has("sleep"));
	const [equipped, setEquipped] = useState(
		parameters.has("outfit") || parameters.has("uniform"),
	);
	const requestedUniform = parameters.get("uniform") ?? "classic";
	const [uniform, setUniform] = useState(
		Object.hasOwn(outfits, requestedUniform) ? requestedUniform : "classic",
	);
	const [size, setSize] = useState(parameters.get("size") === "32" ? 32 : 120);
	const [packId, setPackId] = useState(parameters.get("pack") ?? "all");
	const packs = Object.values(defaultBotCatalog.packs).filter(
		(pack) => packId === "all" || pack.id === packId,
	);
	return (
		<main>
			<header>
				<span className="eyebrow">FLOW-LIKE / COMPANIONS</span>
				<h1>Meet the crew in motion</h1>
				<p>
					Compare expressions, outfits, and small avatars across the whole crew.
				</p>
				<a href="/">Customize a bot</a>
				{" · "}
				<a href="/?wardrobe">Browse the wardrobe</a>
			</header>
			<fieldset className="gallery-controls" aria-label="Gallery controls">
				<label>
					Pack
					<select
						value={packId}
						onChange={(event) => setPackId(event.target.value)}
					>
						<option value="all">All packs</option>
						{Object.values(defaultBotCatalog.packs).map((pack) => (
							<option key={pack.id} value={pack.id}>
								{pack.label}
							</option>
						))}
					</select>
				</label>
				<label>
					Expression
					<select
						value={expression}
						onChange={(event) =>
							setExpression(event.target.value as BotExpression)
						}
					>
						{BOT_EXPRESSIONS.map((value) => (
							<option key={value}>{value}</option>
						))}
					</select>
				</label>
				<label>
					Size
					<select
						value={size}
						onChange={(event) => setSize(Number(event.target.value))}
					>
						<option value="120">120 px</option>
						<option value="64">64 px</option>
						<option value="32">32 px</option>
					</select>
				</label>
				<label>
					<input
						type="checkbox"
						checked={playing}
						onChange={(event) => setPlaying(event.target.checked)}
					/>{" "}
					Animate
				</label>
				<label>
					<input
						type="checkbox"
						checked={sleeping}
						onChange={(event) => setSleeping(event.target.checked)}
					/>{" "}
					Sleep
				</label>
				<label>
					<input
						type="checkbox"
						checked={equipped}
						onChange={(event) => setEquipped(event.target.checked)}
					/>{" "}
					Outfit
				</label>
				<label>
					Uniform
					<select
						aria-label="Uniform"
						value={uniform}
						onChange={(event) => {
							setUniform(event.target.value);
							setEquipped(true);
						}}
					>
						{Object.entries(outfits).map(([id, outfit]) => (
							<option key={id} value={id}>
								{outfit.label}
							</option>
						))}
					</select>
				</label>
			</fieldset>
			<div className="crew-sheet" data-gallery>
				{packs.map((pack) => (
					<section key={pack.id} data-gallery-pack={pack.id}>
						<h2>{pack.label}</h2>
						<p>{pack.description}</p>
						<div className="crew-row">
							{pack.shapeIds.map((kind) => (
								<figure key={kind}>
									<BotAvatar
										appearance={{
											kind,
											equipment: equipped
												? outfits[uniform].equipment
												: undefined,
										}}
										pose={sleeping ? "sleep" : "center"}
										expression={expression}
										animated={playing}
										size={size}
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
		</main>
	);
}
