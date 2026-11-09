import { type ReactNode, useState } from "react";
import type { IconType } from "react-icons";
import {
	LuBot,
	LuBraces,
	LuCalendarClock,
	LuClipboardList,
	LuLayoutDashboard,
	LuMail,
	LuMessagesSquare,
	LuPlug,
} from "react-icons/lu";
import CodeBlock from "../pillar/CodeBlock";

interface Interface {
	id: string;
	label: string;
	icon: IconType;
	summary: string;
	details: readonly string[];
	mock: ReactNode;
}

const BARS = [42, 55, 48, 71, 63];

function Bars() {
	return (
		<div className="ap-mock-bars" aria-hidden="true">
			{BARS.map((value, index) => (
				<i
					key={`${index}-${value}`}
					style={{ height: `${value}%` }}
					data-peak={index === 3}
				/>
			))}
		</div>
	);
}

const REST = `curl -X POST "$APP_URL/revenue" \\
  -H "Authorization: Bearer $TOKEN" \\
  -d '{"region": "EMEA"}'

# 200 OK
{ "region": "EMEA", "revenue": 412300 }`;

const MCP = `{
  "method": "tools/call",
  "params": {
    "name": "weekly_revenue",
    "arguments": { "region": "EMEA" }
  }
}`;

const INTERFACES: readonly Interface[] = [
	{
		id: "chat",
		label: "Chat UI",
		icon: LuMessagesSquare,
		summary: "A chat for people, built in.",
		details: [
			"History and attachments",
			"Tools, starter prompts and reactions",
			"Publish it on a link, sign-in by default",
		],
		mock: (
			<div className="ap-mock-chat">
				<p className="ap-mock-you">How did EMEA do last week?</p>
				<div className="ap-mock-bot">
					<p>
						Revenue was €412k, up 8% on the week before. Tuesday carried most of
						it.
					</p>
					<Bars />
				</div>
			</div>
		),
	},
	{
		id: "page",
		label: "Page",
		icon: LuLayoutDashboard,
		summary: "A full interface on its own route.",
		details: [
			"Layout, SEO and caching",
			"Events on load, on unload and on an interval",
			"Live updates from the flow",
		],
		mock: (
			<div className="ap-mock-page">
				<b>Weekly revenue</b>
				<div className="ap-mock-kpis">
					<span>
						<small>EMEA</small>€412k
					</span>
					<span>
						<small>Americas</small>€538k
					</span>
					<span>
						<small>APAC</small>€201k
					</span>
				</div>
				<Bars />
			</div>
		),
	},
	{
		id: "form",
		label: "Generic Form",
		icon: LuClipboardList,
		summary: "A form generated from the flow's inputs.",
		details: [
			"Fields from the flow's exposed variables",
			"A route of its own",
			"Quick Action is the one-click variant",
		],
		mock: (
			<div className="ap-mock-form">
				<b>New supplier request</b>
				<div className="ap-mock-field">
					Company<span>Northwind Traders</span>
				</div>
				<div className="ap-mock-field">
					VAT ID<span>DE 811 907 980</span>
				</div>
				<div className="ap-mock-field">
					Contact<span>lena@northwind.example</span>
				</div>
				<em>Submit request</em>
			</div>
		),
	},
	{
		id: "rest",
		label: "REST",
		icon: LuBraces,
		summary: "Endpoints for other systems to call.",
		details: [
			"Several endpoints in one surface",
			"Authentication built in",
			"Public, or only for your connected apps",
		],
		mock: (
			<CodeBlock code={REST} language="shell" title="request.sh" badge="REST" />
		),
	},
	{
		id: "mcp",
		label: "MCP",
		icon: LuPlug,
		summary:
			"An MCP server, so AI assistants and agents can use the flow as a tool.",
		details: [
			"Works with any MCP client",
			"Public, or only for your connected apps",
			"Runs with the caller's role",
		],
		mock: (
			<CodeBlock code={MCP} language="json" title="tools/call" badge="MCP" />
		),
	},
	{
		id: "cron",
		label: "Cron",
		icon: LuCalendarClock,
		summary: "A schedule that runs the flow without anyone asking.",
		details: [
			"Any cron expression",
			"Unattended runs act as the event's run-as person",
			"Every run shows up in Runs, with logs",
		],
		mock: (
			<div className="ap-mock-cron">
				<b>Every Monday · 08:00</b>
				<ul>
					<li>Mon 03 Nov · 08:00 · 1.2 s</li>
					<li>Mon 27 Oct · 08:00 · 1.1 s</li>
					<li>Mon 20 Oct · 08:00 · 1.4 s</li>
				</ul>
			</div>
		),
	},
	{
		id: "bots",
		label: "Telegram · Discord · Teams",
		icon: LuBot,
		summary: "Chat bots in the messengers your team already uses.",
		details: [
			"Telegram, Discord or Microsoft Teams",
			"Messages run the flow",
			"Replies land in the same chat",
		],
		mock: (
			<div className="ap-mock-chat ap-mock-chat--messenger">
				<p className="ap-mock-you">/revenue EMEA</p>
				<div className="ap-mock-bot">
					<p>EMEA: €412k last week (+8%).</p>
				</div>
			</div>
		),
	},
	{
		id: "email",
		label: "Email",
		icon: LuMail,
		summary: "Mail in, work done.",
		details: [
			"Read a mailbox over IMAP",
			"Or receive mail at an address of its own",
			"Attachments come along",
		],
		mock: (
			<div className="ap-mock-mail">
				<span>From: finance@northwind.example</span>
				<b>Invoice #4471</b>
				<p>Matched to order 2207 and filed. Approval requested from Lena.</p>
			</div>
		),
	},
];

const MORE = [
	"Quick Action",
	"API",
	"Daemon",
	"Deeplink",
	"Location Region",
	"Inbound Email",
];

export default function AppsInterfaces() {
	const [activeId, setActiveId] = useState(INTERFACES[0].id);
	const active =
		INTERFACES.find((item) => item.id === activeId) ?? INTERFACES[0];
	const Icon = active.icon;

	return (
		<div className="ap-ifaces">
			<fieldset className="ap-iface-tabs" aria-label="Interface">
				{INTERFACES.map((item) => {
					const TabIcon = item.icon;
					return (
						<button
							key={item.id}
							type="button"
							aria-pressed={item.id === active.id}
							onClick={() => setActiveId(item.id)}
						>
							<TabIcon aria-hidden="true" />
							{item.label}
						</button>
					);
				})}
			</fieldset>
			<div className="ap-iface-stage">
				<div className="ap-iface-info">
					<div className="ap-iface-flow" aria-hidden="true">
						<span className="ap-iface-node ap-iface-node--event">
							<Icon />
							{active.label}
						</span>
						<i />
						<span className="ap-iface-node">Weekly revenue flow</span>
						<i />
						<span className="ap-iface-node ap-iface-node--out">Answer</span>
					</div>
					<p className="ap-iface-summary">{active.summary}</p>
					<ul className="ap-iface-details">
						{active.details.map((detail) => (
							<li key={detail}>{detail}</li>
						))}
					</ul>
				</div>
				<div className="ap-iface-mock" key={active.id}>
					{active.mock}
				</div>
			</div>
			<p className="ap-iface-more">
				<span>Also</span>
				{MORE.map((name) => (
					<em key={name}>{name}</em>
				))}
			</p>
		</div>
	);
}
