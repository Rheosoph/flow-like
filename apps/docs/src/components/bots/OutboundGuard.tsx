import { useState } from "react";
import {
	CLASSIFICATION_LABELS,
	DATA_CLASSIFICATIONS,
	type DataClassification,
	classificationRank,
} from "./bot-docs-data";
import { Verdict } from "./ui";

type ToolId =
	| "email_send"
	| "web_request"
	| "connector_call"
	| "image_generate"
	| "file_save"
	| "memory_save";

const TOOLS: readonly { id: ToolId; label: string; outbound: boolean }[] = [
	{ id: "email_send", label: "Send an email", outbound: true },
	{ id: "web_request", label: "Call a web API", outbound: true },
	{ id: "connector_call", label: "Call a connected service", outbound: true },
	{ id: "image_generate", label: "Generate an image", outbound: true },
	{ id: "file_save", label: "Save a file in its workspace", outbound: false },
	{ id: "memory_save", label: "Remember a fact", outbound: false },
];

type Seen = "none" | DataClassification;

export default function OutboundGuard() {
	const [botLevel, setBotLevel] = useState<DataClassification>("internal");
	const [seen, setSeen] = useState<Seen>("confidential");
	const [toolId, setToolId] = useState<ToolId>("email_send");
	const tool = TOOLS.find((item) => item.id === toolId) ?? TOOLS[0];
	const above =
		seen !== "none" && classificationRank(seen) > classificationRank(botLevel);
	const refused = tool.outbound && above;

	return (
		<section className="bd not-content" aria-label="Outbound data guard">
			<div className="bd-head">
				<div>
					<span className="bd-eyebrow">Outbound guard</span>
					<p className="bd-title">Can this leave the bot?</p>
					<p className="bd-sub">
						Once a turn has read data labelled above the bot's own level, the
						tools that send data out refuse for the rest of that turn.
					</p>
				</div>
			</div>
			<div className="bd-split">
				<div>
					<label className="bd-field">
						<span className="bd-label">The bot may handle</span>
						<select
							className="bd-select"
							value={botLevel}
							onChange={(event) =>
								setBotLevel(event.target.value as DataClassification)
							}
						>
							{DATA_CLASSIFICATIONS.map((level) => (
								<option key={level} value={level}>
									{CLASSIFICATION_LABELS[level]}
								</option>
							))}
						</select>
					</label>
					<label className="bd-field">
						<span className="bd-label">
							Most sensitive data it read this turn
						</span>
						<select
							className="bd-select"
							value={seen}
							onChange={(event) => setSeen(event.target.value as Seen)}
						>
							<option value="none">Nothing labelled</option>
							{DATA_CLASSIFICATIONS.map((level) => (
								<option key={level} value={level}>
									{CLASSIFICATION_LABELS[level]}
								</option>
							))}
						</select>
					</label>
					<label className="bd-field">
						<span className="bd-label">Next, it wants to</span>
						<select
							className="bd-select"
							value={toolId}
							onChange={(event) => setToolId(event.target.value as ToolId)}
						>
							{TOOLS.map((item) => (
								<option key={item.id} value={item.id}>
									{item.label}
								</option>
							))}
						</select>
					</label>
				</div>
				<div>
					{refused ? (
						<Verdict tone="bad" title="Refused">
							{`It read ${CLASSIFICATION_LABELS[seen as DataClassification].toLowerCase()} data, above the bot's ${CLASSIFICATION_LABELS[botLevel].toLowerCase()} level, so `}
							<code className="bd-mono">{tool.id}</code>
							{" won't send anything out this turn."}
						</Verdict>
					) : tool.outbound ? (
						<Verdict tone="good" title="Allowed">
							{"Nothing it read is above its own level. "}
							<code className="bd-mono">{tool.id}</code>
							{" still asks you first where its approval rules say so."}
						</Verdict>
					) : (
						<Verdict tone="good" title="Allowed">
							{"Data stays inside the bot, so "}
							<code className="bd-mono">{tool.id}</code>
							{
								" is never blocked by labels. Files and results keep the highest label they were made from."
							}
						</Verdict>
					)}
					<p className="bd-note">
						Data without a label counts as public. Sub-tasks count toward the
						same turn. The personal-data flag is shown to the model but doesn't
						block tools.
					</p>
				</div>
			</div>
		</section>
	);
}
