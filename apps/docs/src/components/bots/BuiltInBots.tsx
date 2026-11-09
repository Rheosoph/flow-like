import { BotAvatar } from "@flow-like/flow-like-ui/components/bots/bot-avatar";
import { BUILT_IN_BOTS } from "./bot-docs-data";
import { CapabilityChip } from "./ui";

const NOTES: Readonly<Record<(typeof BUILT_IN_BOTS)[number]["key"], string>> = {
	general:
		"Starts with Internet and Memory. Switch its abilities, model and look like any other bot's.",
	flux: "FlowPilot with every tool it had. Its abilities are fixed. Builds run on your desktop when it's online.",
};

export default function BuiltInBots() {
	return (
		<div className="bd not-content bd-grid" aria-label="Built-in bots">
			{BUILT_IN_BOTS.map((bot) => (
				<article key={bot.key} className="bd-card bd-builtin">
					<BotAvatar
						appearance={{ ...bot.appearance, backdrop: "none" }}
						size={84}
						pose="happy"
						animated={false}
						decorative
					/>
					<div>
						<h3 className="bd-title">{bot.name}</h3>
						<p className="bd-sub">{bot.description}</p>
						<div className="bd-chips bd-builtin-chips">
							{bot.capabilities.map((id) => (
								<CapabilityChip key={id} id={id} tone="accent" />
							))}
						</div>
						<p className="bd-note">{NOTES[bot.key]}</p>
					</div>
				</article>
			))}
		</div>
	);
}
