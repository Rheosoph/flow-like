import { useState } from "react";
import {
	ALL_CAPABILITIES,
	BOT_CAPABILITY_ICONS,
	type BotLocation,
	capabilityLabel,
	isBotCapabilityAvailable,
} from "./bot-site-data";

export interface BotsHostCopy {
	label: string;
	tagline: string;
	answers: string;
	models: string;
	bestFor: string;
}

export interface BotsHostsLabels {
	where: string;
	answers: string;
	models: string;
	bestFor: string;
	unavailable: string;
}

const ORDER: readonly BotLocation[] = ["server", "desktop", "device"];

function HostGlyph({ location }: { location: BotLocation }) {
	return (
		<svg viewBox="0 0 120 80" aria-hidden="true" className="bp-host-glyph">
			{location === "server" ? (
				<path
					d="M34 62h52a18 18 0 0 0 2-35.9A24 24 0 0 0 42.5 22 20 20 0 0 0 34 62Z"
					fill="none"
					stroke="currentColor"
					strokeWidth="3"
					strokeLinejoin="round"
				/>
			) : location === "desktop" ? (
				<g
					fill="none"
					stroke="currentColor"
					strokeWidth="3"
					strokeLinejoin="round"
				>
					<rect x="22" y="12" width="76" height="46" rx="5" />
					<path d="M48 70h24M60 58v12" strokeLinecap="round" />
				</g>
			) : (
				<g
					fill="none"
					stroke="currentColor"
					strokeWidth="3"
					strokeLinejoin="round"
				>
					<rect x="30" y="14" width="60" height="22" rx="4" />
					<rect x="30" y="44" width="60" height="22" rx="4" />
					<path
						d="M40 25h.01M40 55h.01"
						strokeLinecap="round"
						strokeWidth="5"
					/>
					<path d="M56 25h24M56 55h24" strokeLinecap="round" />
				</g>
			)}
		</svg>
	);
}

export default function BotsHosts({
	hosts,
	labels,
}: {
	hosts: Readonly<Record<BotLocation, BotsHostCopy>>;
	labels: BotsHostsLabels;
}) {
	const [location, setLocation] = useState<BotLocation>("server");
	const host = hosts[location];

	return (
		<div className="bp-hosts" data-location={location}>
			<fieldset className="bp-hosts-tabs" aria-label={labels.where}>
				{ORDER.map((id) => (
					<button
						key={id}
						type="button"
						aria-pressed={id === location}
						onClick={() => setLocation(id)}
					>
						{hosts[id].label}
					</button>
				))}
			</fieldset>
			<div className="bp-hosts-stage">
				<div className="bp-hosts-card">
					<HostGlyph location={location} />
					<h3>{host.label}</h3>
					<p className="bp-hosts-tagline">{host.tagline}</p>
					<dl>
						<div>
							<dt>{labels.answers}</dt>
							<dd>{host.answers}</dd>
						</div>
						<div>
							<dt>{labels.models}</dt>
							<dd>{host.models}</dd>
						</div>
						<div>
							<dt>{labels.bestFor}</dt>
							<dd>{host.bestFor}</dd>
						</div>
					</dl>
				</div>
				<ul className="bp-hosts-abilities">
					{ALL_CAPABILITIES.map((id) => {
						const Icon = BOT_CAPABILITY_ICONS[id];
						const available = isBotCapabilityAvailable(id, location);
						return (
							<li key={id} data-available={available}>
								<Icon aria-hidden="true" />
								<span>{capabilityLabel(id)}</span>
								{available ? null : <em>{labels.unavailable}</em>}
							</li>
						);
					})}
				</ul>
			</div>
		</div>
	);
}
