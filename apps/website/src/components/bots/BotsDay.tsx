import { BotAvatar } from "@flow-like/flow-like-ui/components/bots/bot-avatar";
import { useEffect, useState } from "react";
import { CREW, type CrewId } from "./bot-site-data";

export interface BotsDayEvent {
	time: string;
	kind: string;
	bot: CrewId;
	trigger: string;
	reply: string;
	/** Shown in a room instead of the 1:1 chat. */
	room?: string;
	approval?: string;
	chart?: boolean;
	quiet?: boolean;
}

export interface BotsDayCopy {
	pause: string;
	play: string;
	chatWith: string;
	inRoom: string;
	deny: string;
	once: string;
	always: string;
}

const STEP_MS = 4200;
const CHART = [38, 44, 41, 72, 55, 49, 52];

function prefersReducedMotion(): boolean {
	return (
		typeof window !== "undefined" &&
		window.matchMedia("(prefers-reduced-motion: reduce)").matches
	);
}

export default function BotsDay({
	events,
	copy,
}: {
	events: readonly BotsDayEvent[];
	copy: BotsDayCopy;
}) {
	const [active, setActive] = useState(0);
	const [playing, setPlaying] = useState(false);
	const [held, setHeld] = useState(false);

	useEffect(() => {
		setPlaying(!prefersReducedMotion());
	}, []);

	useEffect(() => {
		if (!playing || held) return;
		const timer = window.setTimeout(
			() => setActive((active + 1) % events.length),
			STEP_MS,
		);
		return () => window.clearTimeout(timer);
	}, [playing, held, active, events.length]);

	const event = events[active];
	const bot = CREW[event.bot];
	const where = event.room
		? copy.inRoom.replace("{name}", event.room)
		: copy.chatWith.replace("{name}", bot.name);

	return (
		<div
			className="bp-day"
			onPointerEnter={() => setHeld(true)}
			onPointerLeave={() => setHeld(false)}
			onFocus={() => setHeld(true)}
			onBlur={() => setHeld(false)}
		>
			<ol className="bp-day-rail">
				{events.map((item, index) => (
					<li key={item.time}>
						<button
							type="button"
							aria-pressed={index === active}
							onClick={() => setActive(index)}
						>
							<span className="bp-day-time">{item.time}</span>
							<BotAvatar
								appearance={{ ...CREW[item.bot].appearance, backdrop: "none" }}
								size={30}
								animated={false}
								decorative
							/>
							<span className="bp-day-kind">{item.kind}</span>
							{index === active && playing && !held ? (
								<span
									className="bp-day-progress"
									style={{ animationDuration: `${STEP_MS}ms` }}
									aria-hidden="true"
								/>
							) : null}
						</button>
					</li>
				))}
			</ol>
			<article className="bp-day-chat" aria-live="polite" key={active}>
				<header>
					<span>{where}</span>
					<button
						type="button"
						className="bp-day-toggle"
						onClick={() => setPlaying(!playing)}
					>
						{playing ? copy.pause : copy.play}
					</button>
				</header>
				<div className="bp-day-body">
					{event.room ? (
						<p className="bp-day-human">{event.trigger}</p>
					) : (
						<p className="bp-day-pill">{event.trigger}</p>
					)}
					<div className="bp-day-reply" data-quiet={event.quiet === true}>
						<BotAvatar
							appearance={{ ...bot.appearance, backdrop: "none" }}
							size={44}
							pose={event.quiet ? "sleep" : "happy"}
							animated={false}
							decorative
						/>
						<div>
							<strong>{bot.name}</strong>
							<p>{event.reply}</p>
							{event.chart ? (
								<div className="bp-day-chart" aria-hidden="true">
									{CHART.map((value, index) => (
										<i
											key={`${index}-${value}`}
											style={{ height: `${value}%` }}
											data-peak={index === 3}
										/>
									))}
								</div>
							) : null}
							{event.approval ? (
								<div className="bp-day-approval">
									<span>{event.approval}</span>
									<div>
										<b>{copy.deny}</b>
										<b>{copy.once}</b>
										<b data-primary="true">{copy.always}</b>
									</div>
								</div>
							) : null}
						</div>
					</div>
				</div>
			</article>
		</div>
	);
}
