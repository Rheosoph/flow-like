import { useState } from "react";
import {
	BOT_AGENT_STATES,
	BOT_MOODS,
	type BotAgentState,
	BotAvatar,
	type BotMood,
} from "../../packages/ui/components/bots";
import "./moods.css";

const moodKinds: Record<BotMood, string> = {
	"likes-you": "sprout",
	cheerful: "pip",
	love: "mochi",
	wink: "bubble",
	surprised: "kuma",
	thinking: "pebble",
	sad: "hexa",
	angry: "cupi",
	oops: "drip",
	sleeping: "ember",
};
const stateKinds: Record<Exclude<BotAgentState, "idle">, string> = {
	working: "mochi",
	idea: "pip",
	done: "ember",
	error: "cupi",
	"needs-you": "bubble",
	offline: "kuma",
};
const activeStates = (Object.keys(BOT_AGENT_STATES) as BotAgentState[]).filter(
	(state): state is Exclude<BotAgentState, "idle"> => state !== "idle",
);
const retro = [
	{ kind: "sprout", mood: "likes-you", effect: "lcd" },
	{ kind: "cupi", agentState: "working", effect: "lcd" },
	{ kind: "hexa", mood: "sad", effect: "lcd" },
	{ kind: "mochi", mood: "love", effect: "pixel" },
	{ kind: "ember", agentState: "done", effect: "pixel" },
	{ kind: "sprout", mood: "angry", effect: "pixel" },
	{ kind: "pip", mood: "cheerful", effect: "pixel" },
	{ kind: "cupi", agentState: "idea", effect: "pixel" },
] as const;

export function BotMoods({ parameters }: { parameters: URLSearchParams }) {
	const [playing, setPlaying] = useState(!parameters.has("paused"));
	const [agentState, setAgentState] = useState<BotAgentState>("idle");
	return (
		<main className="moods-preview" data-moods-preview>
			<header>
				<span className="eyebrow">FLOW-LIKE / COMPANIONS</span>
				<div className="moods-heading">
					<h1>Moods & moments</h1>
					<button
						type="button"
						data-toggle-motion
						aria-pressed={playing}
						onClick={() => setPlaying((current) => !current)}
					>
						{playing ? "Pause motion" : "Play motion"}
					</button>
				</div>
				<p>
					A face and a thought bubble give every companion a feeling. Agent
					states show what is happening right now.
				</p>
			</header>

			<section aria-labelledby="moods-feelings">
				<h2 id="moods-feelings">Feelings</h2>
				<div className="moods-grid">
					{(Object.keys(BOT_MOODS) as BotMood[]).map((mood) => {
						const preset = BOT_MOODS[mood];
						return (
							<figure key={mood} data-mood-card={mood}>
								<BotAvatar
									appearance={{ kind: moodKinds[mood] }}
									mood={mood}
									size={116}
									animated={playing}
								/>
								<figcaption>
									<strong>{preset.label}</strong>
									<span>
										{preset.emote} · {preset.expression}
									</span>
								</figcaption>
							</figure>
						);
					})}
				</div>
			</section>

			<section aria-labelledby="moods-states">
				<h2 id="moods-states">Agent states</h2>
				<div className="moods-grid moods-states">
					{activeStates.map((state) => {
						const preset = BOT_AGENT_STATES[state];
						return (
							<figure key={state} data-state-card={state}>
								<BotAvatar
									appearance={{
										kind: stateKinds[state],
										equipment:
											state === "working" ? { hand: "laptop" } : undefined,
									}}
									agentState={state}
									size={104}
									animated={playing}
								/>
								<figcaption>
									<strong>{preset.label}</strong>
									<span>
										{preset.emote} · {preset.expression}
									</span>
								</figcaption>
							</figure>
						);
					})}
				</div>
			</section>

			<section aria-labelledby="moods-retro">
				<h2 id="moods-retro">A little retro</h2>
				<div className="moods-retro">
					{retro.map(({ kind, effect, ...state }, index) => (
						<figure key={`${kind}-${effect}`} data-retro-card={index}>
							<BotAvatar
								appearance={{ kind, effect, frame: "squircle" }}
								{...state}
								size={92}
								animated={playing}
							/>
							<figcaption>{effect.toUpperCase()}</figcaption>
						</figure>
					))}
				</div>
			</section>

			<section
				className="moods-footer"
				aria-label="Small avatars and state preview"
			>
				<div className="moods-compact">
					<h2>At a glance</h2>
					<div>
						{activeStates.map((state) => (
							<BotAvatar
								key={state}
								appearance={{ kind: stateKinds[state] }}
								agentState={state}
								size={32}
								animated={playing}
							/>
						))}
					</div>
				</div>
				<div className="moods-state-probe" data-state-probe>
					<BotAvatar
						appearance={{ kind: "mochi", effect: "pixel" }}
						mood="love"
						agentState={agentState}
						size={64}
						animated={playing}
					/>
					<label>
						Try a state
						<select
							aria-label="Preview agent state"
							value={agentState}
							onChange={(event) =>
								setAgentState(event.target.value as BotAgentState)
							}
						>
							{Object.entries(BOT_AGENT_STATES).map(([id, state]) => (
								<option key={id} value={id}>
									{state.label}
								</option>
							))}
						</select>
					</label>
				</div>
			</section>
		</main>
	);
}
