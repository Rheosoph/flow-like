import { BotAvatar } from "@flow-like/flow-like-ui/components/bots/bot-avatar";
import { useState } from "react";
import { Segment, Toggle, type Tone, Verdict } from "./ui";
import "./wake-checker.css";

type Place = "direct" | "room";
type Sender = "owner" | "person" | "bot";

interface Outcome {
	readonly tone: Tone;
	readonly title: string;
	readonly detail: string;
}

const SENDER_OPTIONS: Readonly<
	Record<Place, readonly { value: Sender; label: string }[]>
> = {
	direct: [
		{ value: "owner", label: "You" },
		{ value: "person", label: "Another person" },
	],
	room: [
		{ value: "owner", label: "You" },
		{ value: "person", label: "Another person" },
		{ value: "bot", label: "Another bot" },
	],
};

const PERSON_CAP =
	" Messages from people other than you count toward a cap of 20 turns per person per day.";

function wakeOutcome(
	place: Place,
	sender: Sender,
	mentioned: boolean,
	reply: boolean,
	mentionedOnly: boolean,
): Outcome {
	const cap = sender === "person" ? PERSON_CAP : "";
	if (place === "direct") {
		return {
			tone: "good",
			title: "It answers",
			detail: `Every message in a 1:1 chat wakes the bot.${cap}`,
		};
	}
	if (sender === "bot") {
		return mentioned || reply
			? {
					tone: "good",
					title: "It answers",
					detail:
						"A bot's message wakes another bot only when it mentions or replies to it. Chains stop after 3 hops, and turns that bots start for each other count toward a daily cap.",
				}
			: {
					tone: "bad",
					title: "It stays quiet",
					detail:
						"Bots never read along with other bots. The message is stored and shown, nothing more.",
				};
	}
	if (mentioned || reply) {
		return {
			tone: "good",
			title: "It answers",
			detail: `An @mention or a reply to one of its messages always wakes the bot.${cap}`,
		};
	}
	if (mentionedOnly) {
		return {
			tone: "bad",
			title: "It stays quiet",
			detail:
				"Its room limit says “Answer only when mentioned”, so the message is stored and shown without waking it.",
		};
	}
	return {
		tone: "warn",
		title: "It reads along and decides",
		detail: `The bot gets the message and its decision model decides whether answering helps. It may well stay silent.${cap}`,
	};
}

export default function WakeChecker() {
	const [place, setPlace] = useState<Place>("room");
	const [sender, setSender] = useState<Sender>("person");
	const [mentioned, setMentioned] = useState(false);
	const [reply, setReply] = useState(false);
	const [mentionedOnly, setMentionedOnly] = useState(false);

	const changePlace = (next: Place) => {
		setPlace(next);
		if (!SENDER_OPTIONS[next].some((option) => option.value === sender))
			setSender("owner");
	};

	const outcome = wakeOutcome(place, sender, mentioned, reply, mentionedOnly);
	const inRoom = place === "room";
	const senderName =
		sender === "owner" ? "You" : sender === "person" ? "Mara" : "Scout";

	return (
		<section className="bd not-content" aria-label="Will my bot answer?">
			<div className="bd-head">
				<div>
					<span className="bd-eyebrow">Wake checker</span>
					<p className="bd-title">Will my bot answer?</p>
					<p className="bd-sub">
						Set up a message and see whether it wakes your bot, Byte.
					</p>
				</div>
			</div>
			<div className="bd-split">
				<div>
					<div className="bd-field">
						<span className="bd-label">Where</span>
						<Segment
							label="Where the message is sent"
							value={place}
							options={[
								{ value: "direct", label: "1:1 chat with the bot" },
								{ value: "room", label: "Room" },
							]}
							onChange={changePlace}
						/>
					</div>
					<div className="bd-field">
						<span className="bd-label">Who writes</span>
						<Segment
							label="Who writes the message"
							value={sender}
							options={SENDER_OPTIONS[place]}
							onChange={setSender}
						/>
					</div>
					{inRoom ? (
						<div className="bd-field">
							<Toggle
								label="Mentions @Byte"
								checked={mentioned}
								onChange={setMentioned}
							/>
							<Toggle
								label="Replies to one of Byte's messages"
								checked={reply}
								onChange={setReply}
							/>
							{sender !== "bot" ? (
								<Toggle
									label="Room limit: Answer only when mentioned"
									hint="Set by the bot's owner for this room."
									checked={mentionedOnly}
									onChange={setMentionedOnly}
								/>
							) : null}
						</div>
					) : null}
				</div>
				<div className="bd-wake-stage">
					<div className="bd-wake-message">
						<span className="bd-wake-sender">{senderName}</span>
						<p>
							{inRoom && reply ? (
								<span className="bd-wake-quote">
									↪ Byte: “Revenue is up 12%.”
								</span>
							) : null}
							{inRoom && mentioned ? <mark>@Byte</mark> : null}
							{inRoom && mentioned ? " " : ""}
							can you check last week's numbers?
						</p>
					</div>
					<div className="bd-wake-bot">
						<BotAvatar
							appearance={{ kind: "chip" }}
							size={56}
							pose={
								outcome.tone === "good"
									? "happy"
									: outcome.tone === "bad"
										? "sleep"
										: "track"
							}
							animated={false}
							decorative
						/>
						<Verdict tone={outcome.tone} title={outcome.title}>
							{outcome.detail}
						</Verdict>
					</div>
				</div>
			</div>
		</section>
	);
}
