import { useState } from "react";
import {
	BOT_LOCATIONS,
	type BotCapability,
	type BotLocation,
	capabilityLabel,
	editorLocationMessage,
	isBotCapabilityAvailable,
	locationDescription,
	locationLabel,
} from "./bot-docs-data";
import { CapabilityChip, Segment } from "./ui";
import "./host-planner.css";

interface HostProfile {
	readonly answers: string;
	readonly models: string;
	readonly tools: string;
	readonly extra: string;
}

const PROFILES: Readonly<Record<BotLocation, HostProfile>> = {
	server: {
		answers:
			"Around the clock. A worker on the server picks up each turn within seconds, whether or not any of your apps is open.",
		models:
			"Hosted models your plan includes, or a provider you connected with your own key. Models that run on your own hardware can't be used here, so give it a cloud fallback.",
		tools:
			"No screen, browser or shell. Files, documents, code and images run on the platform's executor.",
		extra: "Only cloud bots count toward your plan's bot limit.",
	},
	desktop: {
		answers:
			"While the Flow-Like desktop app is running on your computer. The mobile app answers too while it is open. Turns that arrive while the app is closed wait up to an hour.",
		models:
			"Every model in your profile, including local models on your own hardware.",
		tools:
			"Everything, including the computer's screen, a real browser and the command line. Phones have no shell, and iPhones can't run code yet.",
		extra:
			"Bots you create while signed out live only on this computer until you move them to your account.",
	},
	device: {
		answers:
			"While the device agent runs on a managed device you registered, such as a server or a mini PC. The agent fetches turns from your hub over HTTPS.",
		models: "Hosted models through Flow-Like's model service.",
		tools:
			"Everything, including a browser and a desktop on a virtual display. Its screen is end-to-end encrypted and opens only in the desktop app.",
		extra: "Only that one device can pick up the bot's turns.",
	},
};

const TRY_CAPABILITIES: readonly BotCapability[] = [
	"internet",
	"browser_use",
	"computer_use",
	"command_line",
	"encryption",
];

function listLabels(ids: readonly BotCapability[]): string {
	return ids.map(capabilityLabel).join(", ");
}

export default function HostPlanner() {
	const [location, setLocation] = useState<BotLocation>("desktop");
	const [selected, setSelected] = useState<readonly BotCapability[]>([
		"internet",
		"browser_use",
		"command_line",
	]);
	const [parked, setParked] = useState<readonly BotCapability[]>([]);
	const [message, setMessage] = useState<string | null>(null);

	const move = (next: BotLocation) => {
		if (next === location) return;
		const dropped = selected.filter(
			(id) => !isBotCapabilityAvailable(id, next),
		);
		const restored = parked.filter((id) => isBotCapabilityAvailable(id, next));
		setSelected([
			...selected.filter((id) => !dropped.includes(id)),
			...restored,
		]);
		setParked([...parked.filter((id) => !restored.includes(id)), ...dropped]);
		setMessage(
			dropped.length > 0
				? editorLocationMessage("turnedOff", listLabels(dropped))
				: restored.length > 0
					? editorLocationMessage("turnedBackOn", listLabels(restored))
					: null,
		);
		setLocation(next);
	};

	const toggle = (id: BotCapability, on: boolean) => {
		setSelected(
			on ? [...selected, id] : selected.filter((item) => item !== id),
		);
		setParked(parked.filter((item) => item !== id));
		setMessage(null);
	};

	const profile = PROFILES[location];

	return (
		<section className="bd not-content" aria-label="Where a bot runs">
			<div className="bd-head">
				<div>
					<span className="bd-eyebrow">Host planner</span>
					<p className="bd-title">A bot lives on one host</p>
					<p className="bd-sub">{locationDescription(location)}.</p>
				</div>
				<Segment
					label="Where the bot runs"
					value={location}
					options={BOT_LOCATIONS.map((value) => ({
						value,
						label: locationLabel(value),
					}))}
					onChange={move}
				/>
			</div>
			<div className="bd-split">
				<dl className="bd-host-facts">
					<div>
						<dt>Answers</dt>
						<dd>{profile.answers}</dd>
					</div>
					<div>
						<dt>Models</dt>
						<dd>{profile.models}</dd>
					</div>
					<div>
						<dt>Tools</dt>
						<dd>{profile.tools}</dd>
					</div>
					<div>
						<dt>Good to know</dt>
						<dd>{profile.extra}</dd>
					</div>
				</dl>
				<div className="bd-card">
					<span className="bd-label">
						Try it: switch abilities, then move the bot
					</span>
					<div className="bd-host-toggles">
						{TRY_CAPABILITIES.map((id) => {
							const allowed = isBotCapabilityAvailable(id, location);
							return (
								<label
									key={id}
									className="bd-toggle bd-host-toggle"
									data-disabled={!allowed}
								>
									<input
										type="checkbox"
										checked={selected.includes(id)}
										disabled={!allowed}
										onChange={(event) => toggle(id, event.target.checked)}
									/>
									<CapabilityChip id={id} off={!allowed} />
								</label>
							);
						})}
					</div>
					{message ? (
						<output className="bd-host-message">{message}</output>
					) : location === "server" ? (
						<p className="bd-note">
							Needs the desktop app or a device. Change where it runs to switch
							these on.
						</p>
					) : null}
				</div>
			</div>
		</section>
	);
}
