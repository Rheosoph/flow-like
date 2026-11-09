import { BotAvatar } from "@flow-like/flow-like-ui/components/bots/bot-avatar";
import { useState } from "react";
import {
	BOT_PRESETS,
	CLASSIFICATION_LABELS,
	locationLabel,
	presetCopyOf,
} from "./bot-docs-data";
import { CapabilityChip } from "./ui";
import "./preset-gallery.css";

export default function PresetGallery() {
	const [selectedId, setSelectedId] = useState(BOT_PRESETS[0].id);
	const preset =
		BOT_PRESETS.find((item) => item.id === selectedId) ?? BOT_PRESETS[0];
	const copy = presetCopyOf(preset.id);
	const level = preset.classification.classification;

	return (
		<section className="bd not-content" aria-label="Bot presets">
			<div className="bd-head">
				<div>
					<span className="bd-eyebrow">Presets</span>
					<p className="bd-title">Start from a job, not a blank form</p>
					<p className="bd-sub">
						Describe the bot in your own words and the closest preset fills in
						its look, abilities, host and data label. Pick one to see what it
						sets.
					</p>
				</div>
			</div>
			<div className="bd-preset">
				<fieldset className="bd-preset-list" aria-label="Presets">
					{BOT_PRESETS.map((item) => (
						<button
							key={item.id}
							type="button"
							aria-pressed={item.id === preset.id}
							className="bd-preset-option"
							onClick={() => setSelectedId(item.id)}
						>
							<BotAvatar
								appearance={item.appearance}
								size={40}
								animated={false}
								decorative
							/>
							<span>{presetCopyOf(item.id).name}</span>
						</button>
					))}
				</fieldset>
				<article className="bd-preset-detail" aria-live="polite">
					<div className="bd-preset-hero">
						<BotAvatar
							appearance={preset.appearance}
							size={112}
							pose="happy"
							animated={false}
							decorative
						/>
						<div>
							<h3 className="bd-title">{copy.name}</h3>
							<p className="bd-sub">{copy.description}</p>
							<div className="bd-chips bd-preset-meta">
								<span className="bd-chip">
									Runs on: {locationLabel(preset.location)}
								</span>
								{level ? (
									<span className="bd-chip">
										Handles: {CLASSIFICATION_LABELS[level]}
									</span>
								) : null}
								{preset.classification.pii ? (
									<span className="bd-chip bd-chip--warn">Personal data</span>
								) : null}
							</div>
						</div>
					</div>
					<div className="bd-field">
						<span className="bd-label">Abilities it switches on</span>
						<div className="bd-chips">
							{preset.capabilities.map((id) => (
								<CapabilityChip key={id} id={id} tone="accent" />
							))}
						</div>
					</div>
					<div className="bd-field">
						<span className="bd-label">Instructions it starts with</span>
						<blockquote className="bd-preset-quote">
							{copy.instructions}
						</blockquote>
					</div>
				</article>
			</div>
			<p className="bd-note">
				Everything a preset sets stays editable. Start from scratch to pick
				every ability yourself.
			</p>
		</section>
	);
}
