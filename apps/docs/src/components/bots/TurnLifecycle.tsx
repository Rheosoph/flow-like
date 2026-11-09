import { useEffect, useState } from "react";
import {
	BOT_LOCATIONS,
	type BotLocation,
	locationLabel,
} from "./bot-docs-data";
import { Segment } from "./ui";
import "./turn-lifecycle.css";

type Trigger = "direct" | "room" | "schedule" | "webhook" | "email";

const TRIGGERS: readonly { value: Trigger; label: string }[] = [
	{ value: "direct", label: "1:1 message" },
	{ value: "room", label: "Room" },
	{ value: "schedule", label: "Schedule" },
	{ value: "webhook", label: "Webhook" },
	{ value: "email", label: "Email" },
];

const WAKE: Readonly<Record<Trigger, string>> = {
	direct:
		"You write to the bot in your 1:1 chat. Every message there wakes it.",
	room: "Someone writes in a room. An @mention or a reply wakes the bot; otherwise it reads along and decides, unless it may only answer when mentioned.",
	schedule:
		"A task you or the bot scheduled comes due. It lands in your 1:1 chat as “Scheduled: …”. A run missed while the host was away fires once.",
	webhook:
		"Something calls the bot's webhook URL with its secret or a signature. The payload lands in your 1:1 chat as “Webhook: …”.",
	email:
		"Mail arrives at the bot's flow-like.me address. Automated and failed-authentication mail is dropped; the rest lands in your 1:1 chat as “Email: …”.",
};

const PICKUP: Readonly<Record<BotLocation, string>> = {
	server: "A worker on the server claims the turn within seconds.",
	desktop:
		"Your running desktop app claims it; it checks in about every 30 seconds. While the app is closed, the turn waits.",
	device:
		"The device agent on your managed device claims it. Only that device can.",
};

const WORK: Readonly<Record<BotLocation, string>> = {
	server:
		"The bot's model reads its instructions, memory and the conversation, then uses the tools its abilities switch on. No screen or shell here.",
	desktop:
		"The bot's model, local ones included, reads its instructions, memory and the conversation, then uses its tools, the computer's screen, browser and shell among them.",
	device:
		"The bot's model reads its instructions, memory and the conversation, then uses its tools, including the device's browser and shell.",
};

const BACKGROUND: ReadonlySet<Trigger> = new Set([
	"schedule",
	"webhook",
	"email",
]);

function stages(trigger: Trigger, location: BotLocation) {
	const background = BACKGROUND.has(trigger);
	return [
		{ title: "Wakes", body: WAKE[trigger] },
		{
			title: "Queues",
			body: "The message and the turn are saved together. One turn runs per bot and conversation; newer messages fold into a turn that hasn't started. A turn no host picks up within an hour expires.",
		},
		{ title: "Gets picked up", body: PICKUP[location] },
		{ title: "Works", body: WORK[location] },
		{
			title: "Asks you",
			body: background
				? "Nobody is watching, so for a tool that needs approval the bot first checks whether your own messages or its instructions clearly cover the call. If not, the turn pauses and you get a push with an approval card."
				: "A tool that needs approval shows a card in the chat: Allow once, Always allow or Deny.",
		},
		{
			title: "Answers",
			body:
				trigger === "room"
					? "The reply appears in the room, visible to everyone in it."
					: trigger === "webhook"
						? "The reply appears in your 1:1 chat. A caller that asked to wait up to 30 seconds gets it in the HTTP response, too."
						: background
							? "The result appears in your 1:1 chat with the bot. Tasks marked to report only meaningful changes stay quiet when nothing changed."
							: "The reply appears in your 1:1 chat, on every device you're signed in on.",
		},
	];
}

export default function TurnLifecycle() {
	const [trigger, setTrigger] = useState<Trigger>("direct");
	const [location, setLocation] = useState<BotLocation>("server");
	const [active, setActive] = useState(0);
	const [playing, setPlaying] = useState(false);
	const list = stages(trigger, location);

	useEffect(() => {
		if (!playing) return;
		const timer = window.setInterval(() => {
			setActive((current) => {
				if (current >= list.length - 1) {
					setPlaying(false);
					return current;
				}
				return current + 1;
			});
		}, 1800);
		return () => window.clearInterval(timer);
	}, [playing, list.length]);

	const play = () => {
		setActive(0);
		setPlaying(true);
	};

	return (
		<section className="bd not-content" aria-label="How a bot turn runs">
			<div className="bd-head">
				<div>
					<span className="bd-eyebrow">Anatomy of a turn</span>
					<p className="bd-title">From a nudge to an answer</p>
					<p className="bd-sub">Choose what wakes the bot and where it runs.</p>
				</div>
				<button
					type="button"
					className="bd-button bd-button--primary"
					onClick={play}
				>
					{playing ? "Playing…" : "Play"}
				</button>
			</div>
			<div className="bd-turn-controls">
				<Segment
					label="What wakes the bot"
					value={trigger}
					options={TRIGGERS}
					onChange={setTrigger}
				/>
				<Segment
					label="Where the bot runs"
					value={location}
					options={BOT_LOCATIONS.map((value) => ({
						value,
						label: locationLabel(value),
					}))}
					onChange={setLocation}
				/>
			</div>
			<ol className="bd-turn">
				{list.map((stage, index) => (
					<li
						key={stage.title}
						data-active={index === active}
						data-done={index < active}
					>
						<button
							type="button"
							onClick={() => {
								setPlaying(false);
								setActive(index);
							}}
						>
							<span className="bd-turn-dot" aria-hidden="true">
								{index + 1}
							</span>
							<span className="bd-turn-title">{stage.title}</span>
						</button>
						<p>{stage.body}</p>
					</li>
				))}
			</ol>
		</section>
	);
}
