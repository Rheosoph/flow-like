import { useMemo, useState } from "react";
import {
	BOT_CAPABILITY_GROUPS,
	BOT_CAPABILITY_ICONS,
	BOT_LOCATIONS,
	type BotCapability,
	type BotCapabilityGroupId,
	type BotLocation,
	LOCAL_ONLY_BOT_CAPABILITIES,
	capabilityDescription,
	capabilityLabel,
	groupLabel,
	isBotCapabilityAvailable,
	locationLabel,
} from "./bot-docs-data";
import { CAPABILITY_DETAILS } from "./capability-details";
import { Segment } from "./ui";
import "./capability-explorer.css";

type LocationFilter = "all" | BotLocation;

const LOCATION_OPTIONS: readonly { value: LocationFilter; label: string }[] = [
	{ value: "all", label: "Anywhere" },
	...BOT_LOCATIONS.map((location) => ({
		value: location,
		label: locationLabel(location),
	})),
];

const GROUPS: readonly {
	id: BotCapabilityGroupId | "flux";
	capabilities: readonly BotCapability[];
}[] = [...BOT_CAPABILITY_GROUPS, { id: "flux", capabilities: ["flowpilot"] }];

function matches(id: BotCapability, query: string): boolean {
	if (!query) return true;
	const detail = CAPABILITY_DETAILS[id];
	const haystack = [
		id,
		capabilityLabel(id),
		capabilityDescription(id),
		detail.note ?? "",
		...detail.tools,
	]
		.join(" ")
		.toLowerCase();
	return haystack.includes(query);
}

function CapabilityCard({
	id,
	location,
}: {
	id: BotCapability;
	location: LocationFilter;
}) {
	const Icon = BOT_CAPABILITY_ICONS[id];
	const detail = CAPABILITY_DETAILS[id];
	const available =
		location === "all" || isBotCapabilityAvailable(id, location);
	const localOnly = LOCAL_ONLY_BOT_CAPABILITIES.includes(id);
	return (
		<article
			className="bd-cap"
			data-available={available}
			id={`capability-${id}`}
		>
			<header className="bd-cap-head">
				<span className="bd-cap-icon">
					<Icon aria-hidden="true" />
				</span>
				<div>
					<h4 className="bd-cap-name">{capabilityLabel(id)}</h4>
					<code className="bd-mono bd-cap-id">{id}</code>
				</div>
			</header>
			<p className="bd-cap-desc">{capabilityDescription(id)}</p>
			<div className="bd-chips bd-cap-flags">
				{localOnly ? (
					<span
						className={`bd-chip ${available ? "bd-chip--warn" : "bd-chip--bad"}`}
					>
						{available ? "Desktop or device only" : "Not in the cloud"}
					</span>
				) : null}
				{id === "flowpilot" ? (
					<span className="bd-chip bd-chip--accent">Flux only</span>
				) : null}
				{detail.asks ? <span className="bd-chip">Asks you first</span> : null}
			</div>
			<details className="bd-cap-more">
				<summary>Details</summary>
				{detail.note ? <p>{detail.note}</p> : null}
				{detail.asks ? (
					<p>
						<strong>Asks: </strong>
						{detail.asks}
					</p>
				) : null}
				{detail.tools.length > 0 ? (
					<div className="bd-chips">
						{detail.tools.map((tool) => (
							<code key={tool} className="bd-chip bd-mono">
								{tool}
							</code>
						))}
					</div>
				) : null}
			</details>
		</article>
	);
}

export default function CapabilityExplorer() {
	const [location, setLocation] = useState<LocationFilter>("all");
	const [query, setQuery] = useState("");
	const needle = query.trim().toLowerCase();

	const groups = useMemo(
		() =>
			GROUPS.map((group) => ({
				...group,
				visible: group.capabilities.filter((id) => matches(id, needle)),
			})).filter((group) => group.visible.length > 0),
		[needle],
	);

	const unavailable =
		location === "all"
			? 0
			: GROUPS.flatMap((group) => group.capabilities).filter(
					(id) => !isBotCapabilityAvailable(id, location),
				).length;

	return (
		<section className="bd not-content" aria-label="Bot capability explorer">
			<div className="bd-head">
				<div>
					<span className="bd-eyebrow">Capability explorer</span>
					<p className="bd-title">Every ability a bot can have</p>
					<p className="bd-sub">
						All abilities start switched off. Pick where the bot runs to see
						what it can hold there.
					</p>
				</div>
				<div className="bd-cap-controls">
					<Segment
						label="Where the bot runs"
						value={location}
						options={LOCATION_OPTIONS}
						onChange={setLocation}
					/>
					<input
						className="bd-search"
						type="search"
						placeholder="Search abilities or tools"
						aria-label="Search abilities or tools"
						value={query}
						onChange={(event) => setQuery(event.target.value)}
					/>
				</div>
			</div>
			{location === "server" && unavailable > 0 ? (
				<p className="bd-cap-banner">
					The cloud has no screen, shell or private key, so {unavailable}{" "}
					abilities are unavailable there. The editor switches them off when you
					move a bot to the cloud.
				</p>
			) : null}
			{groups.length === 0 ? (
				<p className="bd-note">No ability matches “{query}”.</p>
			) : (
				groups.map((group) => (
					<div className="bd-cap-group" key={group.id}>
						<h3 className="bd-eyebrow">
							{group.id === "flux" ? "Flux" : groupLabel(group.id)}
						</h3>
						<div className="bd-grid">
							{group.visible.map((id) => (
								<CapabilityCard key={id} id={id} location={location} />
							))}
						</div>
					</div>
				))
			)}
		</section>
	);
}
