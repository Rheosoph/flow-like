import { useState } from "react";
import {
	BotAvatar,
	type BotEffect,
	type BotMood,
} from "../../packages/ui/components/bots";
import {
	BOT_EFFECTS,
	BOT_EFFECT_GROUPS,
} from "../../packages/ui/components/bots/effects";
import "./effects.css";

const samples: Record<BotEffect, { kind: string; mood: BotMood }> = {
	pixel: { kind: "mochi", mood: "likes-you" },
	lcd: { kind: "pip", mood: "cheerful" },
	crt: { kind: "cupi", mood: "surprised" },
	glitch: { kind: "hexa", mood: "oops" },
	thermal: { kind: "flux", mood: "wink" },
	riso: { kind: "sprout", mood: "cheerful" },
	raster: { kind: "pebble", mood: "thinking" },
	comic: { kind: "flux", mood: "likes-you" },
	watercolor: { kind: "mochi", mood: "love" },
	blueprint: { kind: "bubble", mood: "surprised" },
	sticker: { kind: "flux", mood: "wink" },
	vinyl: { kind: "hexa", mood: "sad" },
	neon: { kind: "sprout", mood: "likes-you" },
	none: { kind: "mochi", mood: "surprised" },
	offline: { kind: "hexa", mood: "sleeping" },
};

export function BotEffects({ parameters }: { parameters: URLSearchParams }) {
	const [playing, setPlaying] = useState(!parameters.has("paused"));
	const [effect, setEffect] = useState<BotEffect>("none");
	return (
		<main className="effects-preview" data-effects-preview>
			<header className="effects-header">
				<div>
					<span className="eyebrow">EFFECTS / 15 LOOKS</span>
					<h1>Looks on top</h1>
					<p>
						Give the whole character a new finish. Expressions, thought bubbles,
						and favorite accessories come along.
					</p>
					<button
						type="button"
						aria-pressed={playing}
						data-toggle-effects-motion
						onClick={() => setPlaying((current) => !current)}
					>
						{playing ? "Pause motion" : "Play motion"}
					</button>
				</div>
				<aside className="effects-pixel-scale" aria-label="Pixel sizes">
					<h2>Pixel, at every size</h2>
					<p>
						The same 24 × 24 grid, from a tiny companion to a full portrait.
					</p>
					<div>
						{[96, 64, 44, 28].map((size) => (
							<figure key={size} data-pixel-size={size}>
								<BotAvatar
									appearance={{ kind: "mochi", effect: "pixel" }}
									size={size}
									animated={false}
									pose="center"
								/>
								<figcaption>{size} px</figcaption>
							</figure>
						))}
					</div>
				</aside>
			</header>
			<div className="effects-collections">
				{BOT_EFFECT_GROUPS.map((group) => (
					<section
						key={group.id}
						className={`effects-collection effects-collection-${group.id}`}
						aria-label={group.label}
					>
						<div className="effects-section-heading">
							<h2>{group.label}</h2>
							<p>{group.description}</p>
						</div>
						<div className="effects-grid">
							{group.effects.map((look, index) => (
								<figure key={look.id} data-effect-card={look.id}>
									<div className="effects-card-art">
										<BotAvatar
											appearance={{
												kind: samples[look.id].kind,
												effect: look.id,
												equipment:
													look.id === "raster"
														? { eyes: "glasses" }
														: undefined,
											}}
											mood={samples[look.id].mood}
											size={128}
											animated={playing}
										/>
										<div className="effects-card-examples">
											<BotAvatar
												appearance={{
													kind: index % 2 ? "bambu" : "flux",
													effect: look.id,
													equipment: { head: "crown" },
												}}
												expression="happy"
												pose="center"
												size={52}
												animated={playing}
											/>
											<BotAvatar
												appearance={{
													kind: index % 2 ? "mochi" : "cupi",
													effect: look.id,
													emote: "idea",
												}}
												expression="curious"
												pose="center"
												size={52}
												animated={playing}
											/>
										</div>
									</div>
									<figcaption>
										<strong>{look.label}</strong>
										<span>{look.description}</span>
									</figcaption>
								</figure>
							))}
						</div>
					</section>
				))}
			</div>
			<section className="effects-try" aria-label="Try a look across the crew">
				<div>
					<h2>Try it across the crew</h2>
					<label>
						Effect
						<select
							aria-label="Crew effect"
							value={effect}
							onChange={(event) => setEffect(event.target.value as BotEffect)}
						>
							{BOT_EFFECT_GROUPS.map((group) => (
								<optgroup key={group.id} label={group.label}>
									{group.effects.map((look) => (
										<option key={look.id} value={look.id}>
											{look.label}
										</option>
									))}
								</optgroup>
							))}
						</select>
					</label>
				</div>
				<div className="effects-crew" data-effect-crew>
					{["mochi", "bambu", "cupi", "hexa", "orbi", "drip"].map((kind) => (
						<BotAvatar
							key={kind}
							appearance={{
								kind,
								effect,
								emote: "heart",
								equipment: { head: "crown", neck: "scarf", hand: "laptop" },
							}}
							expression="wink"
							pose="track"
							gx={0.5}
							gy={-0.2}
							size={112}
							animated={playing}
						/>
					))}
				</div>
				<p>{BOT_EFFECTS.find((look) => look.id === effect)?.description}</p>
			</section>
		</main>
	);
}
