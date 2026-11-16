import { BotAvatar } from "@flow-like/flow-like-ui/components/bots/bot-avatar";
import {
	type CSSProperties,
	type FocusEvent,
	type KeyboardEvent,
	type RefObject,
	useCallback,
	useEffect,
	useId,
	useRef,
	useState,
} from "react";
import "./bot-crew-showcase.css";

type CrewStrings = readonly [string, string, string, string, string];

export interface BotCrewCopy {
	title: string;
	pause: string;
	resume: string;
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

const motes = Array.from({ length: 9 }, (_, index) => ({
	id: `mote-${index}`,
	style: {
		"--particle-x": `${7 + index * 10.5}%`,
		"--particle-y": `${24 + (index % 3) * 20}%`,
		"--particle-delay": `${index * -1.7}s`,
	} as CSSProperties,
}));

const STEP_MS = 2300;
const RESULT_STEP = crew.length;
const MISSION_MS = STEP_MS * (RESULT_STEP + 1.6);

type StationState = "idle" | "working" | "done";

function stationState(index: number, phase: number): StationState {
	if (index < phase) return "done";
	return index === phase ? "working" : "idle";
}

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
						<rect
							className="bot-crew-scan"
							x="59"
							y="20"
							width="53"
							height="10"
							rx="2"
							fill="#91bc7370"
						/>
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
					<g
						className="bot-crew-trend"
						fill="#dff7ff"
						stroke="#dff7ff"
						strokeWidth="2"
					>
						<path
							d="m40 54 29-21 29 8 32-26"
							fill="none"
							strokeLinecap="round"
							strokeLinejoin="round"
						/>
						<circle cx="40" cy="54" r="3" />
						<circle cx="69" cy="33" r="3" />
						<circle cx="98" cy="41" r="3" />
						<circle cx="130" cy="15" r="3" />
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
						<path
							className="bot-crew-flow-wire"
							d="M58 48h10q5 0 5-5v-5h13m-13 7v13h13m22-20h10q5 0 5 5v6h9m-24 9h10q5 0 5-5v-4"
							fill="none"
							stroke="#d9a6a0"
						/>
						<g className="bot-crew-flow-node bot-crew-flow-node--one">
							<rect
								x="36"
								y="36"
								width="22"
								height="24"
								rx="4"
								fill="#9d5140"
								stroke="#ffb39a"
							/>
							<path
								d="m46 42-3 7h7l-4 6"
								fill="none"
								stroke="#ffe0c8"
								strokeWidth="2"
							/>
						</g>
						<g className="bot-crew-flow-node bot-crew-flow-node--two">
							<rect
								x="86"
								y="28"
								width="22"
								height="18"
								rx="4"
								fill="#544c73"
								stroke="#c0aae6"
							/>
							<path
								d="M92 34h10m-10 5h7"
								stroke="#d9cef2"
								strokeLinecap="round"
							/>
						</g>
						<g className="bot-crew-flow-node bot-crew-flow-node--three">
							<rect
								x="86"
								y="50"
								width="22"
								height="18"
								rx="4"
								fill="#355b56"
								stroke="#a2d4ad"
							/>
							<path d="m92 59 3 3 7-7" fill="none" stroke="#d2edcf" />
						</g>
						<circle
							className="bot-crew-flow-output"
							cx="137"
							cy="49"
							r="7"
							fill="#e7c284"
						/>
						<path d="m134 49 2 2 4-4" stroke="#684c2f" fill="none" />
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

function MotionIcon({ paused }: { paused: boolean }) {
	return (
		<svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
			{paused ? (
				<path d="M5 3.5v9l7-4.5z" fill="currentColor" />
			) : (
				<path
					d="M5.5 3.5v9m5-9v9"
					stroke="currentColor"
					strokeWidth="2"
					strokeLinecap="round"
				/>
			)}
		</svg>
	);
}

function useMotionGate(root: RefObject<HTMLElement | null>) {
	const [visible, setVisible] = useState(false);
	const [reducedMotion, setReducedMotion] = useState(false);
	const [paused, setPaused] = useState(false);

	useEffect(() => {
		const media = window.matchMedia("(prefers-reduced-motion: reduce)");
		let inView = false;
		const update = () => {
			setVisible(inView && !document.hidden);
			setReducedMotion(media.matches);
		};
		const syncPause = () =>
			setPaused(document.documentElement.dataset.botMotionPaused === "true");
		const observer = new IntersectionObserver(
			([entry]) => {
				inView = entry.isIntersecting;
				update();
			},
			{ threshold: 0.1 },
		);
		if (root.current) observer.observe(root.current);
		update();
		syncPause();
		media.addEventListener("change", update);
		document.addEventListener("visibilitychange", update);
		window.addEventListener("flowlike:bot-motion", syncPause);
		return () => {
			observer.disconnect();
			media.removeEventListener("change", update);
			document.removeEventListener("visibilitychange", update);
			window.removeEventListener("flowlike:bot-motion", syncPause);
		};
	}, [root]);

	const togglePause = useCallback(() => {
		const next = document.documentElement.dataset.botMotionPaused !== "true";
		document.documentElement.dataset.botMotionPaused = String(next);
		window.dispatchEvent(
			new CustomEvent("flowlike:bot-motion", { detail: { paused: next } }),
		);
	}, []);

	return {
		paused,
		reducedMotion,
		running: visible && !reducedMotion && !paused,
		togglePause,
	};
}

export default function BotCrewShowcase({ copy }: { copy: BotCrewCopy }) {
	const root = useRef<HTMLDivElement>(null);
	const progress = useRef<HTMLElement | null>(null);
	const tabs = useRef<(HTMLButtonElement | null)[]>([]);
	const elapsed = useRef(0);
	const autoplay = useRef(true);
	const focusHold = useRef(false);
	const baseId = useId();
	const [missionIndex, setMissionIndex] = useState(0);
	const [phase, setPhase] = useState(0);
	const [greeted, setGreeted] = useState<number | null>(null);
	const { paused, reducedMotion, running, togglePause } = useMotionGate(root);
	const missionCount = copy.missions.length;
	const mission = copy.missions[missionIndex] ?? copy.missions[0];
	const shownPhase = reducedMotion ? RESULT_STEP : phase;

	useEffect(() => {
		if (!running) return;
		let last = performance.now();
		let frame = 0;
		const tick = (now: number) => {
			elapsed.current += Math.max(0, now - last);
			last = now;
			if (elapsed.current >= MISSION_MS) {
				elapsed.current = 0;
				if (autoplay.current && !focusHold.current)
					setMissionIndex((current) => (current + 1) % missionCount);
			}
			progress.current?.style.setProperty(
				"transform",
				`scaleX(${elapsed.current / MISSION_MS})`,
			);
			setPhase(Math.min(Math.floor(elapsed.current / STEP_MS), RESULT_STEP));
			frame = requestAnimationFrame(tick);
		};
		frame = requestAnimationFrame(tick);
		return () => cancelAnimationFrame(frame);
	}, [running, missionCount]);

	const selectMission = (index: number) => {
		autoplay.current = false;
		elapsed.current = 0;
		progress.current?.style.setProperty("transform", "scaleX(0)");
		setPhase(0);
		setMissionIndex(index);
	};

	const onTabKey = (event: KeyboardEvent<HTMLButtonElement>) => {
		const moves: Record<string, number> = {
			ArrowRight: missionIndex + 1,
			ArrowLeft: missionIndex - 1,
			Home: 0,
			End: missionCount - 1,
		};
		const target = moves[event.key];
		if (target === undefined) return;
		event.preventDefault();
		const next = (target + missionCount) % missionCount;
		selectMission(next);
		tabs.current[next]?.focus();
	};

	const onFocusCapture = (event: FocusEvent<HTMLDivElement>) => {
		focusHold.current = event.target.matches(":focus-visible");
	};

	const onBlurCapture = (event: FocusEvent<HTMLDivElement>) => {
		if (!event.currentTarget.contains(event.relatedTarget as Node | null))
			focusHold.current = false;
	};

	if (!mission) return null;

	return (
		<div
			ref={root}
			className="bot-crew"
			data-running={running}
			onFocusCapture={onFocusCapture}
			onBlurCapture={onBlurCapture}
		>
			<div className="bot-crew-header">
				<div className="bot-crew-tabs" role="tablist" aria-label={copy.title}>
					{copy.missions.map((item, index) => {
						const selected = index === missionIndex;
						return (
							<button
								type="button"
								role="tab"
								key={item.label}
								id={`${baseId}-tab-${index}`}
								ref={(element) => {
									tabs.current[index] = element;
								}}
								className="bot-crew-tab"
								aria-selected={selected}
								aria-controls={`${baseId}-panel`}
								tabIndex={selected ? 0 : -1}
								onClick={() => selectMission(index)}
								onKeyDown={onTabKey}
							>
								{item.label}
								<span className="bot-crew-tab-track" aria-hidden="true">
									{selected && <i ref={progress} />}
								</span>
							</button>
						);
					})}
				</div>
				<button
					className="bot-crew-motion"
					type="button"
					onClick={togglePause}
					aria-label={paused ? copy.resume : copy.pause}
					title={paused ? copy.resume : copy.pause}
				>
					<MotionIcon paused={paused} />
				</button>
			</div>
			<div
				id={`${baseId}-panel`}
				role="tabpanel"
				aria-labelledby={`${baseId}-tab-${missionIndex}`}
			>
				<div className="bot-crew-scene">
					<div className="bot-crew-atmosphere" aria-hidden="true">
						{motes.map((mote) => (
							<i key={mote.id} style={mote.style} />
						))}
					</div>
					<div className="bot-crew-prompt">
						<span aria-hidden="true">✳</span>
						<p>{mission.prompt}</p>
					</div>
					<div className="bot-crew-floor" aria-hidden="true">
						<i />
						<i />
						<i />
					</div>
					<ul className="bot-crew-workshop">
						{crew.map((bot, index) => {
							const state = stationState(index, shownPhase);
							const happy = greeted === index;
							return (
								<li
									className={`bot-crew-station bot-crew-station--${bot.prop}`}
									key={bot.name}
									data-state={state}
									data-greeted={happy}
									onPointerEnter={() => setGreeted(index)}
									onPointerLeave={() => setGreeted(null)}
									style={
										{
											"--crew-color": bot.color,
											"--crew-delay": `${index * -0.7}s`,
										} as CSSProperties
									}
								>
									<div className="bot-crew-character">
										<BotAvatar
											appearance={{
												kind: bot.kind,
												dark: true,
												backdrop: "none",
												frame: "square",
											}}
											agentState={state}
											expression={happy ? "happy" : undefined}
											expressionOverridesState={happy}
											animated={running}
											size={index === 2 ? 190 : 156}
											decorative
											className="bot-crew-avatar"
										/>
									</div>
									<div className="bot-crew-desk" aria-hidden="true">
										<WorkProp kind={bot.prop} />
										<span className="bot-crew-desktop" />
										<i className="bot-crew-desk-light" />
									</div>
									<p className="bot-crew-bot-label">
										<strong>{bot.name}</strong>
										<span>{copy.roles[index]}</span>
									</p>
									<p className="bot-crew-task">{mission.tasks[index]}</p>
								</li>
							);
						})}
					</ul>
				</div>
				<div
					className="bot-crew-result"
					data-ready={shownPhase === RESULT_STEP}
				>
					<span className="bot-crew-result-icon" aria-hidden="true">
						↳
					</span>
					<p>
						<span>{copy.resultLabel}</span>
						<strong>{mission.result}</strong>
					</p>
				</div>
			</div>
		</div>
	);
}
