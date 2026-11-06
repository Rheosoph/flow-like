import { BotAvatar } from "@flow-like/flow-like-ui/components/bots/bot-avatar";
import { type CSSProperties, useEffect, useId, useRef, useState } from "react";
import "./bot-crew-showcase.css";

type CrewStrings = readonly [string, string, string, string, string];

export interface BotCrewCopy {
	label: string;
	title: string;
	hint: string;
	pause: string;
	resume: string;
	statusWorking: string;
	statusDone: string;
	resultLabel: string;
	roles: CrewStrings;
	missions: readonly {
		label: string;
		prompt: string;
		result: string;
		tasks: CrewStrings;
	}[];
}

const crew = [
	{ name: "Scout", kind: "pip", color: "#a8d8a2", prop: "research" },
	{ name: "Byte", kind: "chip", color: "#87c8ed", prop: "data" },
	{ name: "Flux", kind: "flux", color: "#ff987a", prop: "build" },
	{ name: "Post", kind: "mochi", color: "#c6a8ed", prop: "inbox" },
	{ name: "Loop", kind: "orbi", color: "#efd28e", prop: "routine" },
] as const;

function WorkProp({ kind }: { kind: (typeof crew)[number]["prop"] }) {
	return (
		<svg
			className={`bot-crew-prop bot-crew-prop--${kind}`}
			viewBox="0 0 180 100"
			aria-hidden="true"
			focusable="false"
		>
			{kind === "research" && (
				<>
					<g className="bot-crew-paper">
						<rect
							x="35"
							y="16"
							width="62"
							height="73"
							rx="5"
							fill="#eee9db"
							transform="rotate(-12 66 53)"
						/>
						<rect x="54" y="7" width="63" height="79" rx="5" fill="#fff8e9" />
						<path
							d="M65 24h27M65 34h40M65 44h32M65 54h35M65 64h19"
							stroke="#aaa795"
							strokeWidth="4"
							strokeLinecap="round"
						/>
						<rect className="bot-crew-scan" x="59" y="20" width="53" height="10" rx="2" fill="#91bc7370" />
					</g>
					<g className="bot-crew-glass">
						<circle
							cx="115"
							cy="42"
							r="22"
							fill="#a8d8a22b"
							stroke="#b9dda6"
							strokeWidth="6"
						/>
						<path
							d="m130 59 18 20"
							stroke="#b9dda6"
							strokeWidth="8"
							strokeLinecap="round"
						/>
						<path
							d="M104 35q2-6 8-7"
							stroke="#fff"
							strokeWidth="3"
							strokeLinecap="round"
							fill="none"
						/>
					</g>
				</>
			)}
			{kind === "data" && (
				<>
					<rect
						x="25"
						y="6"
						width="132"
						height="79"
						rx="7"
						fill="#1d303d"
						stroke="#51778d"
					/>
					<path
						d="M36 69h108M36 48h108M36 27h108"
						stroke="#87c8ed"
						opacity=".14"
					/>
					{[30, 47, 37, 61].map((height, index) => (
						<rect
							key={height}
							className="bot-crew-bar"
							style={{ "--bar-delay": `${index * -0.35}s` } as CSSProperties}
							x={43 + index * 25}
							y={76 - height}
							width="13"
							height={height}
							rx="3"
							fill={index === 3 ? "#d2f4ff" : "#72add1"}
						/>
					))}
					<g className="bot-crew-trend" fill="#dff7ff" stroke="#dff7ff" strokeWidth="2">
						<path d="m40 54 29-21 29 8 32-26" fill="none" strokeLinecap="round" strokeLinejoin="round" />
						<circle cx="40" cy="54" r="3" /><circle cx="69" cy="33" r="3" /><circle cx="98" cy="41" r="3" /><circle cx="130" cy="15" r="3" />
					</g>
					<path
						d="M79 85v9m24-9v9M65 95h52"
						stroke="#597b8c"
						strokeWidth="4"
						strokeLinecap="round"
					/>
				</>
			)}
			{kind === "build" && (
				<>
					<rect
						x="24"
						y="4"
						width="133"
						height="77"
						rx="7"
						fill="#28242d"
						stroke="#a16d63"
						strokeWidth="2"
					/>
					<path d="M28 20h125" stroke="#4d3840" />
					<circle cx="34" cy="12" r="2" fill="#ff9779" />
					<circle cx="42" cy="12" r="2" fill="#ebc487" />
					<circle cx="50" cy="12" r="2" fill="#9ac4a2" />
					<g className="bot-crew-code" strokeWidth="1.5">
						<path className="bot-crew-flow-wire" d="M58 48h10q5 0 5-5v-5h13m-13 7v13h13m22-20h10q5 0 5 5v6h9m-24 9h10q5 0 5-5v-4" fill="none" stroke="#d9a6a0" />
						<g className="bot-crew-flow-node bot-crew-flow-node--one"><rect x="36" y="36" width="22" height="24" rx="4" fill="#9d5140" stroke="#ffb39a" /><path d="m46 42-3 7h7l-4 6" fill="none" stroke="#ffe0c8" strokeWidth="2" /></g>
						<g className="bot-crew-flow-node bot-crew-flow-node--two"><rect x="86" y="28" width="22" height="18" rx="4" fill="#544c73" stroke="#c0aae6" /><path d="M92 34h10m-10 5h7" stroke="#d9cef2" strokeLinecap="round" /></g>
						<g className="bot-crew-flow-node bot-crew-flow-node--three"><rect x="86" y="50" width="22" height="18" rx="4" fill="#355b56" stroke="#a2d4ad" /><path d="m92 59 3 3 7-7" fill="none" stroke="#d2edcf" /></g>
						<circle className="bot-crew-flow-output" cx="137" cy="49" r="7" fill="#e7c284" /><path d="m134 49 2 2 4-4" stroke="#684c2f" fill="none" />
					</g>
					<path d="M24 81 8 91q-2 5 8 5h148q10 0 8-5l-15-10" fill="#5b4650" />
					<path d="M73 83h35l5 5H68z" fill="#a88180" />
				</>
			)}
			{kind === "inbox" && (
				<>
					<rect
						x="30"
						y="48"
						width="120"
						height="43"
						rx="6"
						fill="#574466"
						stroke="#aa8ec5"
					/>
					<path
						d="m31 57 57 28 61-28"
						fill="none"
						stroke="#aa8ec5"
						strokeWidth="3"
					/>
					<g className="bot-crew-mail">
						<rect
							x="51"
							y="10"
							width="80"
							height="59"
							rx="5"
							fill="#eee6f5"
							transform="rotate(6 91 40)"
						/>
						<path
							d="m53 13 37 30 40-23"
							fill="none"
							stroke="#ad97c4"
							strokeWidth="3"
						/>
						<circle cx="130" cy="13" r="12" fill="#c2a5e5" />
						<path
							d="m125 13 4 4 7-8"
							fill="none"
							stroke="#292034"
							strokeWidth="2.5"
							strokeLinecap="round"
							strokeLinejoin="round"
						/>
					</g>
				</>
			)}
			{kind === "routine" && (
				<>
					<rect x="40" y="14" width="102" height="74" rx="8" fill="#f1e5c5" />
					<path d="M40 32h102" stroke="#d8bc7e" strokeWidth="3" />
					<path
						d="M61 8v15m59-15v15"
						stroke="#b79b65"
						strokeWidth="7"
						strokeLinecap="round"
					/>
					<path
						d="M54 48h9m11 0h9m11 0h9M54 62h9m11 0h9M54 76h9"
						stroke="#b7a47c"
						strokeWidth="5"
						strokeLinecap="round"
					/>
					<circle
						cx="124"
						cy="66"
						r="25"
						fill="#332e26"
						stroke="#e2c780"
						strokeWidth="4"
					/>
					<g className="bot-crew-clock">
						<path
							d="M124 48v18l12 7"
							stroke="#f5df9f"
							strokeWidth="3"
							strokeLinecap="round"
							strokeLinejoin="round"
							fill="none"
						/>
					</g>
					<circle cx="124" cy="66" r="3" fill="#f5df9f" />
				</>
			)}
		</svg>
	);
}

export default function BotCrewShowcase({ copy }: { copy: BotCrewCopy }) {
	const root = useRef<HTMLDivElement>(null);
	const panelId = useId();
	const [missionIndex, setMissionIndex] = useState(0);
	const [phase, setPhase] = useState(0);
	const [paused, setPaused] = useState(false);
	const [motionAllowed, setMotionAllowed] = useState(false);
	const [selectedBot, setSelectedBot] = useState<number | null>(null);
	const [hoveredBot, setHoveredBot] = useState<number | null>(null);
	const spotlight = hoveredBot ?? selectedBot;
	const running = motionAllowed && !paused;
	const mission = copy.missions[missionIndex] ?? copy.missions[0];

	useEffect(() => {
		const syncPause = () => setPaused(document.documentElement.dataset.botMotionPaused === "true");
		syncPause();
		window.addEventListener("flowlike:bot-motion", syncPause);
		return () => window.removeEventListener("flowlike:bot-motion", syncPause);
	}, []);

	const togglePause = () => {
		const next = !paused;
		document.documentElement.dataset.botMotionPaused = String(next);
		window.dispatchEvent(new CustomEvent("flowlike:bot-motion", { detail: { paused: next } }));
	};

	useEffect(() => {
		const media = window.matchMedia("(prefers-reduced-motion: reduce)");
		let visible = false;
		const update = () =>
			setMotionAllowed(visible && !document.hidden && !media.matches);
		const observer = new IntersectionObserver(
			([entry]) => {
				visible = entry.isIntersecting;
				update();
			},
			{ threshold: 0.1 },
		);
		if (root.current) observer.observe(root.current);
		media.addEventListener("change", update);
		document.addEventListener("visibilitychange", update);
		return () => {
			observer.disconnect();
			media.removeEventListener("change", update);
			document.removeEventListener("visibilitychange", update);
		};
	}, []);

	useEffect(() => {
		if (!running) return;
		const interval = window.setInterval(
			() => setPhase((current) => (current + 1) % 4),
			2600,
		);
		return () => window.clearInterval(interval);
	}, [running]);

	if (!mission) return null;

	return (
		<div ref={root} className="bot-crew" data-running={running}>
			<header className="bot-crew-toolbar">
				<span className="bot-crew-label">
					<i aria-hidden="true" />
					{copy.label}
				</span>
				<button
					className="bot-crew-pause"
					type="button"
					onClick={togglePause}
					aria-pressed={paused}
				>
					<span aria-hidden="true">{paused ? "▷" : "Ⅱ"}</span>
					{paused ? copy.resume : copy.pause}
				</button>
			</header>
			<div className="bot-crew-intro">
				<p>{copy.title}</p>
				<span>{copy.hint}</span>
			</div>
			<div className="bot-crew-missions" role="group" aria-label={copy.title}>
				{copy.missions.map((item, index) => (
					<button
						type="button"
						key={item.label}
						aria-pressed={missionIndex === index}
						aria-controls={panelId}
						onClick={() => {
							setMissionIndex(index);
							setPhase(0);
						}}
					>
						<span aria-hidden="true">0{index + 1}</span>
						{item.label}
						<i aria-hidden="true">↗</i>
					</button>
				))}
			</div>
			<div id={panelId} className="bot-crew-scene" data-spotlight={spotlight !== null}>
				<div className="bot-crew-atmosphere" aria-hidden="true">
					{Array.from({ length: 9 }, (_, index) => <i key={index} style={{ "--particle-x": `${7 + index * 10.5}%`, "--particle-y": `${24 + (index % 3) * 20}%`, "--particle-delay": `${index * -1.7}s` } as CSSProperties} />)}
				</div>
				<div className="bot-crew-prompt">
					<span aria-hidden="true">✳</span>
					<p>{mission.prompt}</p>
				</div>
				<div className="bot-crew-floor" aria-hidden="true">
					<i />
					<i />
					<i />
					<i />
					<i />
				</div>
				<div className="bot-crew-workshop">
					{crew.map((bot, index) => {
						const done = (phase + index) % 4 === 3;
						const greeted = spotlight === index;
						return (
							<div
								className={`bot-crew-station bot-crew-station--${bot.prop}`}
								key={bot.name}
								data-spotlight={greeted}
								style={
									{
										"--crew-color": bot.color,
										"--crew-delay": `${index * -0.7}s`,
									} as CSSProperties
								}
							>
								<div className="bot-crew-status" data-done={done}>
									<i aria-hidden="true">{done ? "✓" : "···"}</i>
									{done ? copy.statusDone : copy.statusWorking}
								</div>
								<button
									className="bot-crew-character"
									type="button"
									aria-label={`${bot.name}: ${copy.roles[index]}`}
									aria-pressed={selectedBot === index}
									onClick={() => setSelectedBot((current) => current === index ? null : index)}
									onPointerEnter={() => setHoveredBot(index)}
									onPointerLeave={() => setHoveredBot(null)}
									onFocus={() => setHoveredBot(index)}
									onBlur={() => setHoveredBot(null)}
								>
									<BotAvatar
										appearance={{
											kind: bot.kind,
											dark: true,
											backdrop: "none",
											frame: "square",
										}}
										agentState={
											done ? "done" : index === phase ? "idea" : "working"
										}
										expression={greeted ? "happy" : undefined}
										expressionOverridesState={greeted}
										animated={running}
										size={index === 2 ? 190 : 156}
										decorative
										className="bot-crew-avatar"
									/>
								</button>
								<div className="bot-crew-desk">
									<WorkProp kind={bot.prop} />
									<div className="bot-crew-desk-bits" aria-hidden="true">
										{index % 2 === 0 ? <span className="bot-crew-cup"><i /><i /></span> : <span className="bot-crew-notes"><i /><i /><i /></span>}
									</div>
									<span className="bot-crew-desktop" />
									<i className="bot-crew-desk-light" />
								</div>
								<div className="bot-crew-bot-label">
									<strong>{bot.name}</strong>
									<span>{copy.roles[index]}</span>
								</div>
								<p className="bot-crew-task">{mission.tasks[index]}</p>
							</div>
						);
					})}
				</div>
			</div>
			<div className="bot-crew-result" aria-live="polite" aria-atomic="true">
				<span className="bot-crew-result-icon" aria-hidden="true">
					↳
				</span>
				<p>
					<span>{copy.resultLabel}</span>
					<strong>{mission.result}</strong>
				</p>
				<span className="bot-crew-result-spark" aria-hidden="true">
					✦
				</span>
			</div>
		</div>
	);
}
