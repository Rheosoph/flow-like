import { BotAvatar } from "@flow-like/flow-like-ui/components/bots/bot-avatar";
import { useState } from "react";
import { Segment, type Tone, Verdict } from "./ui";
import "./approval-demo.css";

type ScenarioId = "read" | "write" | "shell" | "mail" | "settings";
type Mode = "watching" | "background";
type Decision = "once" | "always" | "deny";

interface Scenario {
	readonly label: string;
	readonly summary: string;
	readonly tool: string;
	/** Absent when the tool can't be allowed for good. */
	readonly scope?: string;
	readonly covers?: string;
}

const SCENARIOS: Readonly<Record<ScenarioId, Scenario>> = {
	read: {
		label: "Read a feed",
		summary: "read news.example.com/feed.xml",
		tool: "web_request",
		scope: "web:news.example.com:read",
		covers:
			"Every read from news.example.com. Writing to it, or any other site, still asks.",
	},
	write: {
		label: "Post to an API",
		summary: "send a POST to hooks.example.com",
		tool: "web_request",
		scope: "web:hooks.example.com:write",
		covers:
			"Writes to hooks.example.com. Every secret it sends there is approved separately.",
	},
	shell: {
		label: "Run a command",
		summary: "run `git status` on the command line",
		tool: "shell_run",
		scope: "command:git",
		covers: "Every git command. Any other program still asks.",
	},
	mail: {
		label: "Send mail",
		summary: "email lena@acme.com",
		tool: "email_send",
		scope: "email:acme.com",
		covers: "Mail to anyone at acme.com. Other domains still ask.",
	},
	settings: {
		label: "Change its settings",
		summary: "switch on Web requests for itself",
		tool: "bot_update_configuration",
	},
};

function outcome(
	scenario: Scenario,
	decision: Decision,
): { tone: Tone; title: string; detail: string } {
	if (decision === "deny") {
		return {
			tone: "bad",
			title: "Denied",
			detail:
				"The call is refused and the bot is told not to retry it. It can tell you why it needed it or find another way.",
		};
	}
	if (decision === "once" || !scenario.scope) {
		return {
			tone: "good",
			title: "Allowed once",
			detail:
				scenario.tool === "bot_update_configuration"
					? "The new settings apply from the bot's next turn. It asks again next time."
					: "This exact call runs now. The next one asks again.",
		};
	}
	return {
		tone: "good",
		title: "Always allowed",
		detail: `${scenario.covers} Revoke it any time under the bot's Allowed actions.`,
	};
}

export default function ApprovalDemo() {
	const [scenarioId, setScenarioId] = useState<ScenarioId>("read");
	const [mode, setMode] = useState<Mode>("watching");
	const [decision, setDecision] = useState<Decision | null>(null);
	const scenario = SCENARIOS[scenarioId];
	const grantable = scenario.scope !== undefined;
	const result = decision ? outcome(scenario, decision) : null;

	const pick = (id: ScenarioId) => {
		setScenarioId(id);
		setDecision(null);
	};

	return (
		<section className="bd not-content" aria-label="Approval card demo">
			<div className="bd-head">
				<div>
					<span className="bd-eyebrow">Approval demo</span>
					<p className="bd-title">Answer an approval card</p>
					<p className="bd-sub">
						Pick what the bot wants to do, then answer the card.
					</p>
				</div>
				<Segment
					label="Who is watching"
					value={mode}
					options={[
						{ value: "watching", label: "You're in the chat" },
						{ value: "background", label: "Background turn" },
					]}
					onChange={(next) => {
						setMode(next);
						setDecision(null);
					}}
				/>
			</div>
			<div className="bd-chips bd-approval-scenarios">
				{(Object.keys(SCENARIOS) as ScenarioId[]).map((id) => (
					<button
						key={id}
						type="button"
						className="bd-button"
						aria-pressed={id === scenarioId}
						onClick={() => pick(id)}
					>
						{SCENARIOS[id].label}
					</button>
				))}
			</div>
			<div className="bd-split bd-approval-layout">
				<div className="bd-approval-card">
					<div className="bd-approval-top">
						<BotAvatar
							appearance={{ kind: "pip" }}
							size={40}
							animated={false}
							decorative
						/>
						<div>
							<span className="bd-eyebrow">Approval request</span>
							<p className="bd-approval-title">
								Scout wants to {scenario.summary}
							</p>
							{mode === "background" ? (
								<p className="bd-approval-trigger">
									Triggered by Scheduled: Morning digest
								</p>
							) : null}
						</div>
					</div>
					<code className="bd-mono bd-approval-tool">{scenario.tool}</code>
					<div className="bd-approval-actions">
						<button
							type="button"
							className="bd-button"
							aria-pressed={decision === "deny"}
							onClick={() => setDecision("deny")}
						>
							Deny
						</button>
						<button
							type="button"
							className="bd-button"
							aria-pressed={decision === "once"}
							onClick={() => setDecision("once")}
						>
							Allow once
						</button>
						<button
							type="button"
							className="bd-button bd-button--primary"
							aria-pressed={decision === "always"}
							disabled={!grantable}
							onClick={() => setDecision("always")}
						>
							Always allow
						</button>
					</div>
					{!grantable ? (
						<p className="bd-note">
							Always asks first. Auto mode and standing approvals never cover a
							settings change.
						</p>
					) : (
						<p className="bd-note">
							Always allow stores{" "}
							<code className="bd-mono">{scenario.scope}</code>.
						</p>
					)}
				</div>
				<div className="bd-approval-side">
					{mode === "background" ? (
						<ol className="bd-approval-steps">
							<li>
								<strong>Standing answers first.</strong> An earlier Always allow
								runs the call; an earlier Deny refuses it.
							</li>
							<li>
								<strong>Then your own words.</strong>
								{grantable
									? " The bot's decision model checks whether your messages or the bot's instructions clearly cover this exact call. If it's sure, the call runs once, without saving an approval."
									: " Settings changes are never inferred, so this step is skipped."}
							</li>
							<li>
								<strong>Otherwise it waits for you.</strong> The turn pauses,
								this card lands in your 1:1 chat with the bot and you get a
								push. Unanswered cards expire after 7 days.
							</li>
						</ol>
					) : (
						<p className="bd-sub">
							While you're in the chat, the card appears right in the
							conversation and the bot waits for your answer.
						</p>
					)}
					{result ? (
						<Verdict tone={result.tone} title={result.title}>
							{result.detail}
						</Verdict>
					) : null}
				</div>
			</div>
		</section>
	);
}
