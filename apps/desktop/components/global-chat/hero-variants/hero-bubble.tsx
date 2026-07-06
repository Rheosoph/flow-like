"use client";

import { Textarea } from "@flow-like/flow-like-ui";
import {
	ArrowUpIcon,
	AudioLinesIcon,
	type LucideIcon,
	PackageIcon,
	PaperclipIcon,
	PlusIcon,
	SparklesIcon,
	UserRoundIcon,
} from "lucide-react";
import {
	type CSSProperties,
	type KeyboardEvent,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import { FRAG, VERT, readTokenRGB, themeIsLight } from "./bubble-shader";
import { HeroAgentControls } from "./hero-agent-controls";
import { HeroFileChips, HeroFileInput } from "./hero-file-chips";
import { HERO_SUGGESTIONS, useHeroComposer } from "./use-hero-composer";

const HERO_BUBBLE_CSS = `
.hero-bubble-wrap {
	position: relative;
	width: min(940px, 100%);
	height: 258px;
	cursor: text;
}
.hero-bubble-canvas {
	position: absolute;
	left: -12%;
	top: -45%;
	width: 124%;
	height: 190%;
	pointer-events: none;
}
.hero-bubble-fallback {
	position: absolute;
	inset: 0;
	border-radius: 46% 54% 52% 48% / 58% 46% 54% 42%;
	background:
		radial-gradient(80% 120% at 30% 20%, rgba(139, 92, 246, 0.14), transparent 60%),
		radial-gradient(70% 110% at 75% 80%, rgba(244, 114, 182, 0.1), transparent 60%),
		rgba(13, 15, 24, 0.92);
	border: 1.5px solid rgba(139, 92, 246, 0.5);
	box-shadow:
		0 0 60px -10px rgba(79, 125, 255, 0.4),
		0 0 90px -20px rgba(232, 121, 249, 0.35),
		inset 0 0 60px -20px rgba(79, 125, 255, 0.3);
	pointer-events: none;
}
@keyframes hero-bubble-blobmorph {
	0%, 100% { border-radius: 46% 54% 52% 48% / 58% 46% 54% 42%; }
	33% { border-radius: 54% 46% 44% 56% / 46% 58% 42% 54%; }
	66% { border-radius: 48% 52% 58% 42% / 52% 44% 56% 48%; }
}
.hero-bubble-content {
	position: absolute;
	inset: 46px 190px 42px 96px;
	display: flex;
	flex-direction: column;
	justify-content: space-between;
	z-index: 2;
}
.hero-bubble-row {
	display: flex;
	align-items: center;
	gap: 10px;
}
.hero-bubble-spark {
	color: #7c5cf0;
	padding: 6px;
}
.dark .hero-bubble-spark { color: #cfc6ff; }
.hero-bubble-pill {
	display: inline-flex;
	align-items: center;
	gap: 7px;
	height: 36px;
	padding: 0 14px;
	border-radius: 999px;
	font-size: 13px;
	font-weight: 600;
	color: #4a4661;
	background: rgba(100, 85, 190, 0.07);
	border: 1px solid rgba(100, 85, 190, 0.22);
	transition: background 0.25s, border-color 0.25s, box-shadow 0.25s;
	cursor: default;
}
.dark .hero-bubble-pill {
	color: #d9d4f5;
	background: rgba(255, 255, 255, 0.05);
	border-color: rgba(255, 255, 255, 0.1);
}
.hero-bubble-pill:hover {
	background: rgba(100, 85, 190, 0.12);
	border-color: rgba(124, 92, 240, 0.4);
}
.dark button.hero-bubble-pill {
	cursor: pointer;
}
.hero-bubble-pill:hover {
	background: rgba(255, 255, 255, 0.09);
	border-color: rgba(180, 154, 255, 0.4);
}
.light .hero-bubble-pill:hover {
	background: rgba(100, 85, 190, 0.12);
}
/* model pill keeps a comfortable cap so long ids don't blow out the row */
.hero-bubble-model {
	max-width: 12rem;
}
.hero-bubble-model:disabled {
	opacity: 0.55;
	cursor: default;
}
.hero-bubble-flowpilot {
	background: linear-gradient(140deg, rgba(139, 92, 246, 0.32), rgba(88, 63, 182, 0.22));
	border-color: rgba(160, 128, 255, 0.45);
	box-shadow: 0 0 22px -6px rgba(139, 92, 246, 0.55), inset 0 1px 0 rgba(255, 255, 255, 0.1);
	color: #efeaff;
}
.hero-bubble-flowpilot:hover {
	box-shadow: 0 0 30px -6px rgba(139, 92, 246, 0.8), inset 0 1px 0 rgba(255, 255, 255, 0.12);
}
.hero-bubble-dot {
	width: 16px;
	height: 16px;
	border-radius: 5px;
	display: grid;
	place-items: center;
	background: linear-gradient(140deg, #a78bfa, #7c3aed);
	color: #fff;
}
.hero-bubble-duo {
	padding: 0;
	overflow: hidden;
}
.hero-bubble-duo span {
	padding: 0 13px;
	display: flex;
	align-items: center;
	gap: 6px;
	height: 100%;
}
.hero-bubble-duo span + span {
	border-left: 1px solid rgba(100, 85, 190, 0.2);
	color: #79809a;
}
.dark .hero-bubble-duo span + span {
	border-left-color: rgba(255, 255, 255, 0.1);
	color: #98a0b3;
}
.hero-bubble-side {
	position: absolute;
	right: 26px;
	top: 50%;
	translate: 0 -50%;
	display: flex;
	align-items: center;
	gap: 14px;
	z-index: 2;
}
/* the composer only materializes once the bubble straightens out;
   .hero-bubble-side keeps its -50% vertical centering through the transition */
.hero-bubble-content,
.hero-bubble-side {
	opacity: 0;
	pointer-events: none;
	transition: opacity 0.3s ease, translate 0.3s ease;
}
.hero-bubble-content { translate: 0 10px; }
.hero-bubble-side { translate: 0 calc(-50% + 10px); }
.hero-bubble-wrap.hero-bubble-open .hero-bubble-content,
.hero-bubble-wrap.hero-bubble-open .hero-bubble-side {
	opacity: 1;
	pointer-events: auto;
	transition-delay: 0.18s;
}
.hero-bubble-wrap.hero-bubble-open .hero-bubble-content { translate: 0 0; }
.hero-bubble-wrap.hero-bubble-open .hero-bubble-side { translate: 0 -50%; }
/* idle affordance: breathing hint in the middle of the bare bubble */
.hero-bubble-hint {
	position: absolute;
	left: 50%;
	top: 50%;
	translate: -50% -50%;
	display: inline-flex;
	align-items: center;
	gap: 10px;
	color: #565d75;
	font-size: 15.5px;
	font-weight: 500;
	white-space: nowrap;
	pointer-events: none;
	z-index: 2;
	transition: opacity 0.25s, scale 0.25s;
}
.dark .hero-bubble-hint { color: #c3c9e8; }
.hero-bubble-hint svg { color: #7c5cf0; }
.dark .hero-bubble-hint svg { color: #cfc6ff; }
/* animation would out-cascade the opacity here — disable it when open */
.hero-bubble-wrap.hero-bubble-open .hero-bubble-hint {
	animation: none;
	opacity: 0;
	scale: 0.9;
}
@keyframes hero-bubble-hint {
	0%, 100% { opacity: 0.55; }
	50% { opacity: 0.95; }
}
/* suggestion satellites: little glass bubbles orbiting the big one */
.hero-bubble-sat {
	position: absolute;
	z-index: 3;
	display: inline-flex;
	align-items: center;
	gap: 8px;
	padding: 10px 16px;
	border-radius: 999px;
	font-size: 13px;
	font-weight: 600;
	white-space: nowrap;
	color: #3d4156;
	background: radial-gradient(120% 140% at 32% 28%, rgba(255, 255, 255, 0.9), rgba(210, 205, 250, 0.35) 50%, rgba(180, 175, 235, 0.25) 100%);
	box-shadow:
		inset 0 0 0 1px rgba(124, 92, 240, 0.3),
		inset -3px -3px 10px rgba(90, 150, 240, 0.16),
		inset 2px 2px 8px rgba(255, 255, 255, 0.8),
		0 4px 18px -6px rgba(120, 100, 220, 0.35);
	backdrop-filter: blur(6px);
	cursor: pointer;
	visibility: visible;
	transition: box-shadow 0.25s, scale 0.25s cubic-bezier(0.34, 1.56, 0.64, 1), opacity 0.3s;
}
.dark .hero-bubble-sat {
	color: #dfe3f5;
	background: radial-gradient(120% 140% at 32% 28%, rgba(255, 255, 255, 0.1), rgba(150, 150, 220, 0.05) 50%, rgba(20, 22, 34, 0.55) 100%);
	box-shadow:
		inset 0 0 0 1px rgba(190, 180, 255, 0.28),
		inset -3px -3px 10px rgba(120, 180, 255, 0.18),
		inset 2px 2px 8px rgba(255, 255, 255, 0.08),
		0 0 18px -4px rgba(150, 140, 255, 0.35);
}
.hero-bubble-sat svg { color: #7c5cf0; }
.dark .hero-bubble-sat svg { color: #b9a8ff; }
.hero-bubble-sat:hover {
	scale: 1.06;
	box-shadow:
		inset 0 0 0 1px rgba(124, 92, 240, 0.55),
		inset -3px -3px 10px rgba(90, 150, 240, 0.22),
		inset 2px 2px 8px rgba(255, 255, 255, 0.9),
		0 6px 24px -6px rgba(120, 100, 220, 0.5);
}
.dark .hero-bubble-sat:hover {
	box-shadow:
		inset 0 0 0 1px rgba(210, 200, 255, 0.5),
		inset -3px -3px 10px rgba(120, 180, 255, 0.26),
		inset 2px 2px 8px rgba(255, 255, 255, 0.12),
		0 0 26px -4px rgba(150, 140, 255, 0.6);
}
/* visibility drops after the fade so hidden satellites can't take focus */
.hero-bubble-wrap.hero-bubble-open .hero-bubble-sat {
	opacity: 0;
	scale: 0.9;
	pointer-events: none;
	visibility: hidden;
	transition: box-shadow 0.25s, scale 0.3s, opacity 0.3s, visibility 0s 0.3s;
}
@keyframes hero-bubble-satbob {
	0%, 100% { transform: translateY(0); }
	50% { transform: translateY(-9px); }
}
.hero-bubble-icon {
	color: #6a7188;
	padding: 8px;
	border-radius: 50%;
	transition: color 0.25s, background 0.25s;
}
.dark .hero-bubble-icon { color: #9aa2bd; }
button.hero-bubble-icon {
	cursor: pointer;
}
/* voice input is not offered yet — show it dimmed + inert */
.hero-bubble-icon-off {
	opacity: 0.3;
	cursor: not-allowed;
	pointer-events: none;
}
.hero-bubble-icon:hover {
	color: #23222a;
	background: rgba(0, 0, 0, 0.05);
}
.dark .hero-bubble-icon:hover {
	color: #f2efec;
	background: rgba(255, 255, 255, 0.06);
}
.hero-bubble-send {
	position: relative;
	width: 68px;
	height: 68px;
	border-radius: 50%;
	display: grid;
	place-items: center;
	color: #fff;
	background: radial-gradient(circle at 32% 28%, #b49aff, #7c5cf0 55%, #5b3fd4);
	box-shadow:
		0 0 0 6px rgba(139, 92, 246, 0.14),
		0 0 0 14px rgba(139, 92, 246, 0.06),
		0 12px 40px -8px rgba(124, 92, 240, 0.8);
	transition: scale 0.2s cubic-bezier(0.34, 1.56, 0.64, 1), filter 0.25s;
	cursor: pointer;
}
.hero-bubble-send:hover:not(:disabled) { scale: 1.07; }
.hero-bubble-send:active:not(:disabled) { scale: 0.95; }
.hero-bubble-send:disabled {
	filter: saturate(0.4) brightness(0.75);
	cursor: default;
}
@keyframes hero-bubble-pulse {
	0%, 100% { box-shadow: 0 0 0 6px rgba(139, 92, 246, 0.14), 0 0 0 14px rgba(139, 92, 246, 0.06), 0 12px 40px -8px rgba(124, 92, 240, 0.8); }
	50% { box-shadow: 0 0 0 9px rgba(139, 92, 246, 0.2), 0 0 0 20px rgba(139, 92, 246, 0.08), 0 12px 52px -6px rgba(124, 92, 240, 1); }
}
.hero-bubble-mote {
	position: absolute;
	width: 5px;
	height: 5px;
	border-radius: 50%;
	/* var(--foreground) keeps the motes visible over the page in both themes */
	background: radial-gradient(circle, var(--foreground), transparent 70%);
	pointer-events: none;
	z-index: 1;
}
.hero-bubble-mote-star {
	width: 14px;
	height: 14px;
	background: none;
	color: #cfc0ff;
}
@keyframes hero-bubble-mote {
	0%, 100% { opacity: 0.15; translate: 0 0; scale: 0.8; }
	50% { opacity: 0.95; translate: 0 -9px; scale: 1.15; }
}
/* rotating verb: brand primary color */
.hero-rot {
	display: inline-block;
	position: relative;
	white-space: nowrap;
	vertical-align: bottom;
	transition: width 0.5s cubic-bezier(0.22, 1, 0.36, 1);
}
.hero-rot-sizer {
	position: absolute;
	visibility: hidden;
	white-space: nowrap;
}
.hero-rot-word {
	position: absolute;
	left: 0;
	top: 0;
	white-space: nowrap;
	background: var(--primary);
	-webkit-background-clip: text;
	background-clip: text;
	color: transparent;
}
.dark .hero-rot-word {
	background: var(--primary);
	-webkit-background-clip: text;
	background-clip: text;
}
.hero-rot-word .l { display: inline-block; }
.hero-rot[data-fx="pop"] .hero-rot-word {
	background: none;
	color: inherit;
}
.hero-rot[data-fx="morph"] .hero-rot-word.in { animation: hero-rot-morph-in 0.5s cubic-bezier(0.22, 1, 0.36, 1) both; }
.hero-rot[data-fx="morph"] .hero-rot-word.out { animation: hero-rot-morph-out 0.45s ease both; }
@keyframes hero-rot-morph-in {
	from { opacity: 0; translate: 0 16px; filter: blur(8px); }
	to { opacity: 1; translate: 0 0; filter: blur(0); }
}
@keyframes hero-rot-morph-out {
	to { opacity: 0; translate: 0 -16px; filter: blur(8px); }
}
.hero-rot[data-fx="wipe"] .hero-rot-word.in { animation: hero-rot-wipe-in 0.55s ease both; }
.hero-rot[data-fx="wipe"] .hero-rot-word.out { animation: hero-rot-wipe-out 0.5s ease both; }
@keyframes hero-rot-wipe-in {
	from { clip-path: inset(-10% 100% -10% 0); }
	to { clip-path: inset(-10% -5% -10% 0); }
}
@keyframes hero-rot-wipe-out {
	from { clip-path: inset(-10% -5% -10% 0); }
	to { clip-path: inset(-10% -5% -10% 105%); }
}
.hero-rot[data-fx="pop"] .hero-rot-word.in .l { animation: hero-rot-pop-in 0.5s cubic-bezier(0.34, 1.56, 0.64, 1) calc(var(--i) * 60ms) both; }
.hero-rot[data-fx="pop"] .hero-rot-word.out .l { animation: hero-rot-pop-out 0.3s ease calc(var(--i) * 35ms) both; }
@keyframes hero-rot-pop-in {
	from { opacity: 0; translate: 0 14px; scale: 0.6; }
	to { opacity: 1; translate: 0 0; scale: 1; }
}
@keyframes hero-rot-pop-out {
	to { opacity: 0; translate: 0 -12px; scale: 1.2; }
}
@keyframes hero-bubble-twinkle {
	0%, 100% { scale: 1; opacity: 0.9; rotate: 0deg; }
	50% { scale: 1.12; opacity: 1; rotate: 8deg; }
}
/* little soap bubbles drifting up and popping; invisible unless animated */
.hero-bubble-minib {
	position: absolute;
	border-radius: 50%;
	background: radial-gradient(circle at 35% 30%, rgba(255, 255, 255, 0.9), rgba(150, 140, 240, 0.15) 45%, transparent 62%);
	box-shadow:
		inset 0 0 0 1px rgba(124, 92, 240, 0.35),
		inset -2px -2px 6px rgba(90, 150, 240, 0.25),
		0 0 12px -2px rgba(120, 100, 220, 0.35);
	pointer-events: none;
	z-index: 1;
	opacity: 0;
}
.dark .hero-bubble-minib {
	background: radial-gradient(circle at 35% 30%, rgba(255, 255, 255, 0.28), rgba(160, 150, 255, 0.08) 45%, transparent 62%);
	box-shadow:
		inset 0 0 0 1px rgba(190, 180, 255, 0.4),
		inset -2px -2px 6px rgba(120, 180, 255, 0.3),
		0 0 12px -2px rgba(150, 140, 255, 0.45);
}
@keyframes hero-bubble-minib {
	0% { translate: 0 12px; opacity: 0; scale: 0.7; }
	16% { opacity: 0.9; }
	62% { opacity: 0.75; }
	100% { translate: 8px -52px; opacity: 0; scale: 1.08; }
}
@media (prefers-reduced-motion: no-preference) {
	.hero-bubble-fallback { animation: hero-bubble-blobmorph 9s ease-in-out infinite; }
	.hero-bubble-spark { animation: hero-bubble-twinkle 4s ease-in-out infinite; }
	.hero-bubble-send { animation: hero-bubble-pulse 3.2s ease-in-out infinite; }
	.hero-bubble-mote { animation: hero-bubble-mote 5s ease-in-out infinite; }
	.hero-bubble-minib { animation: hero-bubble-minib 9s ease-in-out infinite; }
	.hero-bubble-hint { animation: hero-bubble-hint 3.5s ease-in-out infinite; }
	.hero-bubble-sat { animation: hero-bubble-satbob var(--bob, 6s) ease-in-out var(--bobd, 0s) infinite; }
	.hero-bubble-wrap.hero-bubble-open .hero-bubble-hint { animation: none; }
}
/* the floating satellites are desktop-only; below the bubble on phones instead */
.hero-bubble-mobile-sats { display: none; }
.hero-bubble-mobile-chip {
	display: inline-flex;
	align-items: center;
	gap: 7px;
	max-width: 100%;
	padding: 8px 14px;
	border-radius: 999px;
	font-size: 12.5px;
	font-weight: 600;
	color: var(--foreground);
	background: color-mix(in oklab, var(--foreground) 5%, transparent);
	border: 1px solid var(--border);
	cursor: pointer;
}
.hero-bubble-mobile-chip svg { color: #7c5cf0; }
.dark .hero-bubble-mobile-chip svg { color: #b9a8ff; }

/* tablet & below: floating satellites overlap the bubble — reflow to the chip row */
@media (max-width: 768px) {
	.hero-bubble-wrap { width: 100%; }
	.hero-bubble-sat { display: none; }
	.hero-bubble-mobile-sats {
		display: flex;
		flex-wrap: wrap;
		justify-content: center;
		gap: 8px;
		width: 100%;
		max-width: 100%;
	}
}
/* phones: compact the composer so text + controls fit a narrow bubble */
@media (max-width: 560px) {
	.hero-bubble-wrap { height: 200px; }
	/* small insets; the send orb moves to the bottom-right corner */
	.hero-bubble-content {
		inset: 14px 14px 12px 14px;
		gap: 4px;
	}
	/* reserve the bottom-right corner for attach+send so the pills can never run
	   under them — the pills wrap to their own line first instead */
	.hero-bubble-content .hero-bubble-row {
		gap: 6px;
		flex-wrap: wrap;
		padding-right: 88px;
	}
	.hero-bubble-spark { display: none; }
	.hero-bubble-pill {
		height: 34px;
		font-size: 12px;
		padding: 0 12px;
	}
	.hero-bubble-side {
		top: auto;
		bottom: 14px;
		right: 14px;
		translate: none;
		gap: 8px;
	}
	.hero-bubble-wrap.hero-bubble-open .hero-bubble-side { translate: none; }
	/* smaller send with a tight drop shadow — the wide pulsing rings overflowed
	   the composer's rounded corner on phones */
	.hero-bubble-send {
		width: 42px;
		height: 42px;
		box-shadow: 0 8px 22px -8px rgba(124, 92, 240, 0.75);
		animation: none;
	}
	.hero-bubble-send svg { width: 18px; height: 18px; }
	.hero-bubble-side .hero-bubble-icon { padding: 5px; }
	/* voice is not offered — drop it entirely on the tightest layouts */
	.hero-bubble-icon-off { display: none; }
	.hero-bubble-model { max-width: 8.5rem; }
}
`;

const MOTES = [
	{ left: "6%", top: "12%", delay: "0s" },
	{ left: "14%", top: "78%", delay: "1.4s" },
	{ left: "46%", top: "-6%", delay: "0.8s" },
	{ left: "88%", top: "8%", delay: "2.1s" },
	{ left: "96%", top: "64%", delay: "0.4s" },
	{ left: "70%", top: "102%", delay: "1.9s" },
	{ left: "30%", top: "110%", delay: "3.2s" },
	{ left: "58%", top: "-12%", delay: "1.1s" },
	{ left: "-3%", top: "60%", delay: "2.3s" },
];

const MINI_BUBBLES = [
	{ left: "2%", top: "24%", size: 16, delay: "0s" },
	{ left: "90%", top: "80%", size: 22, delay: "3s" },
	{ left: "75%", top: "-4%", size: 12, delay: "6s" },
];

const SATELLITE_META: ReadonlyArray<{
	icon: LucideIcon;
	strokeWidth: number;
	style: CSSProperties;
}> = [
	{
		icon: PlusIcon,
		strokeWidth: 2,
		style: {
			left: "1%",
			top: "14%",
			"--bob": "5.5s",
			"--bobd": "0s",
		} as CSSProperties,
	},
	{
		icon: SparklesIcon,
		strokeWidth: 1.8,
		style: {
			right: "-1%",
			top: "20%",
			"--bob": "6.4s",
			"--bobd": "0.9s",
		} as CSSProperties,
	},
	{
		icon: PackageIcon,
		strokeWidth: 1.8,
		style: {
			left: "4%",
			bottom: "8%",
			"--bob": "7s",
			"--bobd": "1.7s",
		} as CSSProperties,
	},
	{
		icon: UserRoundIcon,
		strokeWidth: 1.8,
		style: {
			right: "6%",
			bottom: "16%",
			"--bob": "6s",
			"--bobd": "2.6s",
		} as CSSProperties,
	},
];

const SATELLITES = HERO_SUGGESTIONS.map((label, index) => ({
	label,
	...SATELLITE_META[index % SATELLITE_META.length],
}));

const ROTATING_WORDS = ["build", "do", "know", "automate"];

type RotatingWordFx = "morph" | "wipe" | "pop";

function wordLetters(word: string) {
	const seen: Record<string, number> = {};
	return [...word].map((ch) => {
		seen[ch] = (seen[ch] ?? 0) + 1;
		return { ch, id: `${ch}${seen[ch]}` };
	});
}

function RotatingWord({ fx = "pop" }: Readonly<{ fx?: RotatingWordFx }>) {
	const [rot, setRot] = useState({
		prev: null as string | null,
		cur: ROTATING_WORDS[0],
		n: 0,
	});
	const [width, setWidth] = useState<number | null>(null);
	const widthRef = useRef<number | null>(null);
	const sizerRef = useRef<HTMLSpanElement>(null);

	useEffect(() => {
		if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
		const id = setInterval(() => {
			setRot(({ cur, n }) => ({
				prev: cur,
				cur: ROTATING_WORDS[(n + 1) % ROTATING_WORDS.length],
				n: n + 1,
			}));
		}, 2600);
		return () => clearInterval(id);
	}, []);

	useEffect(() => {
		const sizer = sizerRef.current;
		if (!sizer || sizer.textContent !== rot.cur) return;
		const next = sizer.offsetWidth;
		const prev = widthRef.current;
		widthRef.current = next;
		if (prev !== null && next < prev) {
			// shrinking early would slide the "?" over the outgoing word
			const t = setTimeout(() => setWidth(next), 240);
			return () => clearTimeout(t);
		}
		setWidth(next);
	}, [rot.cur]);

	useEffect(() => {
		let cancelled = false;
		// late font loads change metrics — re-measure once settled
		document.fonts?.ready.then(() => {
			if (!cancelled && sizerRef.current) {
				widthRef.current = sizerRef.current.offsetWidth;
				setWidth(widthRef.current);
			}
		});
		return () => {
			cancelled = true;
		};
	}, []);

	const renderWord = (word: string, phase: "in" | "out", key: string) => (
		<span key={key} className={`hero-rot-word${rot.n > 0 ? ` ${phase}` : ""}`}>
			{fx === "pop"
				? wordLetters(word).map(({ ch, id }, i) => (
						<span
							key={`${key}-${id}`}
							className="l"
							style={
								{
									"--i": i,
									color: "var(--primary)",
								} as CSSProperties
							}
						>
							{ch}
						</span>
					))
				: word}
		</span>
	);

	return (
		<span
			className="hero-rot"
			data-fx={fx}
			aria-hidden="true"
			style={width !== null ? { width } : undefined}
		>
			{"​"}
			<span ref={sizerRef} className="hero-rot-sizer">
				{rot.cur}
			</span>
			{rot.prev !== null && renderWord(rot.prev, "out", `out-${rot.n}`)}
			{renderWord(rot.cur, "in", `in-${rot.n}`)}
		</span>
	);
}

function FourPointStar({ size }: Readonly<{ size: number }>) {
	return (
		<svg
			width={size}
			height={size}
			viewBox="0 0 24 24"
			fill="currentColor"
			aria-hidden="true"
		>
			<path d="M12 3c1 5 4 8 9 9-5 1-8 4-9 9-1-5-4-8-9-9 5-1 8-4 9-9Z" />
		</svg>
	);
}

export function HeroSearchBarBubble() {
	const {
		value,
		setValue,
		files,
		addFiles,
		removeFile,
		openFilePicker,
		fileInputRef,
		submit,
		canSend,
	} = useHeroComposer();
	const [webglFailed, setWebglFailed] = useState(false);
	const wrapRef = useRef<HTMLDivElement>(null);
	const canvasRef = useRef<HTMLCanvasElement>(null);
	const focusRef = useRef(0);
	const focusTargetRef = useRef(0);

	useEffect(() => {
		const wrap = wrapRef.current;
		const canvas = canvasRef.current;
		if (!wrap || !canvas) return;

		const gl = canvas.getContext("webgl", {
			alpha: true,
			premultipliedAlpha: true,
			antialias: false,
		});
		if (!gl) {
			setWebglFailed(true);
			return;
		}

		const compile = (type: number, src: string) => {
			const shader = gl.createShader(type);
			if (!shader) return null;
			gl.shaderSource(shader, src);
			gl.compileShader(shader);
			if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
				console.error(gl.getShaderInfoLog(shader));
				gl.deleteShader(shader);
				return null;
			}
			return shader;
		};

		const vs = compile(gl.VERTEX_SHADER, VERT);
		const fs = compile(gl.FRAGMENT_SHADER, FRAG);
		const prog = vs && fs ? gl.createProgram() : null;
		if (!vs || !fs || !prog) {
			setWebglFailed(true);
			return;
		}
		gl.attachShader(prog, vs);
		gl.attachShader(prog, fs);
		gl.linkProgram(prog);
		if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
			console.error(gl.getProgramInfoLog(prog));
			setWebglFailed(true);
			gl.deleteProgram(prog);
			return;
		}
		gl.useProgram(prog);

		const buf = gl.createBuffer();
		gl.bindBuffer(gl.ARRAY_BUFFER, buf);
		gl.bufferData(
			gl.ARRAY_BUFFER,
			new Float32Array([-1, -1, 3, -1, -1, 3]),
			gl.STATIC_DRAW,
		);
		const loc = gl.getAttribLocation(prog, "p");
		gl.enableVertexAttribArray(loc);
		gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);

		const uRes = gl.getUniformLocation(prog, "u_res");
		const uTime = gl.getUniformLocation(prog, "u_time");
		const uFocus = gl.getUniformLocation(prog, "u_focus");
		const uBox = gl.getUniformLocation(prog, "u_box");
		const uMouse = gl.getUniformLocation(prog, "u_mouse");
		const uMstr = gl.getUniformLocation(prog, "u_mstr");
		const uMorph = gl.getUniformLocation(prog, "u_morph");
		const uLight = gl.getUniformLocation(prog, "u_light");
		const uPrimary = gl.getUniformLocation(prog, "u_primary");

		const reduced = window.matchMedia(
			"(prefers-reduced-motion: reduce)",
		).matches;

		// feed the app's --primary (brand ember) into the film so the bubble
		// harmonizes with the active scheme; re-read whenever the theme changes
		const setPrimary = () => {
			const [r, g, b] = readTokenRGB("--primary", [242, 90, 60]);
			gl.uniform3f(uPrimary, r / 255, g / 255, b / 255);
		};
		setPrimary();

		let lightTarget = themeIsLight() ? 1 : 0;
		let light = lightTarget;
		// reduced motion has no loop to poll in, so react to theme changes directly
		const themeObserver = new MutationObserver(() => {
			lightTarget = themeIsLight() ? 1 : 0;
			setPrimary();
			if (reduced) {
				light = lightTarget;
				draw(4200);
			}
		});
		themeObserver.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ["class", "style", "data-theme"],
		});

		let mstr = 0;
		let mstrTarget = 0;
		let mx = 0;
		let my = 0;
		let mxTarget = 0;
		let myTarget = 0;
		let morph = 0;
		let morphV = 0;
		let morphTarget = 0;

		const input = wrap.querySelector("textarea");
		// open/close is click-outside based (not blur) so using the attach/send
		// buttons inside the composer doesn't collapse it mid-interaction
		const setOpen = (open: boolean) => {
			morphTarget = open ? 1 : 0;
			focusTargetRef.current = open ? 1 : 0;
			wrap.classList.toggle("hero-bubble-open", open);
			if (reduced) {
				morph = open ? 1 : 0;
				draw(4200);
			}
		};
		const onFocus = () => setOpen(true);
		const onBlur = () => {
			focusTargetRef.current = 0;
		};
		// clicking anywhere on the bubble hands focus to the composer
		const onWrapClick = (e: MouseEvent) => {
			const target = e.target as HTMLElement | null;
			if (!target?.closest("button, input, textarea, a")) input?.focus();
		};
		const onDocPointerDown = (e: PointerEvent) => {
			const target = e.target as HTMLElement | null;
			if (!target) return;
			// the provider/model dropdowns portal outside the wrap; a click inside one —
			// or while one is open — must not collapse the composer
			if (
				target.closest(
					'[data-radix-popper-content-wrapper],[data-slot="dropdown-menu-content"],[role="menu"]',
				)
			) {
				return;
			}
			if (
				document.querySelector(
					'[data-slot="dropdown-menu-content"][data-state="open"]',
				)
			) {
				return;
			}
			if (!wrap.contains(target)) setOpen(false);
		};
		const onDocKeyDown = (e: globalThis.KeyboardEvent) => {
			if (e.key === "Escape" && wrap.classList.contains("hero-bubble-open")) {
				setOpen(false);
				input?.blur();
			}
		};
		const onPointerEnter = () => {
			focusTargetRef.current = Math.max(focusTargetRef.current, 0.45);
			mstrTarget = 1;
		};
		const onPointerLeave = () => {
			focusTargetRef.current =
				input && document.activeElement === input ? 1 : 0;
			mstrTarget = 0;
		};
		const onPointerMove = (e: PointerEvent) => {
			// pointer position in the shader's uv space (origin center, y up)
			const r = canvas.getBoundingClientRect();
			if (!r.height) return;
			mxTarget = ((e.clientX - r.left) * 2 - r.width) / r.height;
			myTarget = (r.height - 2 * (e.clientY - r.top)) / r.height;
		};
		input?.addEventListener("focus", onFocus);
		input?.addEventListener("blur", onBlur);
		wrap.addEventListener("pointerenter", onPointerEnter);
		wrap.addEventListener("pointerleave", onPointerLeave);
		wrap.addEventListener("pointermove", onPointerMove);
		wrap.addEventListener("click", onWrapClick);
		document.addEventListener("pointerdown", onDocPointerDown);
		document.addEventListener("keydown", onDocKeyDown);

		const draw = (ms: number) => {
			const halfH = canvas.clientHeight / 2;
			if (!halfH) return;
			// half-extents in shader uv units: a compact bubble at rest that
			// spreads to the full composer rectangle as the morph progresses
			const rectBx = (wrap.clientWidth / 2 - 14) / halfH;
			const rectBy = (wrap.clientHeight / 2 - 16) / halfH;
			const k = Math.min(Math.max(morph, 0), 1.15);
			const bx = rectBx * (0.4 + 0.6 * k);
			const by = rectBy * (0.92 + 0.08 * k);
			gl.uniform2f(uRes, canvas.width, canvas.height);
			gl.uniform1f(uTime, ms / 1000);
			gl.uniform1f(uFocus, focusRef.current);
			gl.uniform2f(uBox, bx, by);
			gl.uniform2f(uMouse, mx, my);
			gl.uniform1f(uMstr, mstr);
			gl.uniform1f(uMorph, morph);
			gl.uniform1f(uLight, light);
			gl.clearColor(0, 0, 0, 0);
			gl.clear(gl.COLOR_BUFFER_BIT);
			gl.drawArrays(gl.TRIANGLES, 0, 3);
		};

		const resize = () => {
			const dpr = Math.min(window.devicePixelRatio || 1, 2);
			const w = canvas.clientWidth;
			const h = canvas.clientHeight;
			if (!w || !h) return;
			canvas.width = Math.round(w * dpr);
			canvas.height = Math.round(h * dpr);
			gl.viewport(0, 0, canvas.width, canvas.height);
			// resizing clears the canvas; repaint the single static frame
			if (reduced) draw(4200);
		};
		resize();
		const observer = new ResizeObserver(resize);
		observer.observe(canvas);

		let raf = 0;
		let frame = 0;
		if (reduced) {
			draw(4200);
		} else {
			const loop = (ms: number) => {
				focusRef.current += (focusTargetRef.current - focusRef.current) * 0.05;
				mstr += (mstrTarget - mstr) * 0.06;
				mx += (mxTarget - mx) * 0.12;
				my += (myTarget - my) * 0.12;
				// spring: the bubble snaps into the rectangle with a slight overshoot
				morphV += (morphTarget - morph) * 0.045;
				morphV *= 0.86;
				morph += morphV;
				// re-read the theme ~3x/sec so the bubble can never get stuck in
				// the wrong palette regardless of how/when the app flips themes
				if (++frame % 20 === 0) lightTarget = themeIsLight() ? 1 : 0;
				light += (lightTarget - light) * 0.08;
				draw(ms);
				raf = requestAnimationFrame(loop);
			};
			raf = requestAnimationFrame(loop);
		}

		return () => {
			cancelAnimationFrame(raf);
			observer.disconnect();
			input?.removeEventListener("focus", onFocus);
			input?.removeEventListener("blur", onBlur);
			wrap.removeEventListener("pointerenter", onPointerEnter);
			wrap.removeEventListener("pointerleave", onPointerLeave);
			wrap.removeEventListener("pointermove", onPointerMove);
			wrap.removeEventListener("click", onWrapClick);
			document.removeEventListener("pointerdown", onDocPointerDown);
			document.removeEventListener("keydown", onDocKeyDown);
			themeObserver.disconnect();
			// themeProbe is a shared singleton in bubble-shader (reused by the launcher);
			// leave it mounted — it is an invisible 0×0 span, cheap to keep.
			// no loseContext(): StrictMode remounts reuse the canvas, and a lost
			// context would make the second mount fail into the CSS fallback
			gl.deleteBuffer(buf);
			gl.deleteProgram(prog);
			gl.deleteShader(vs);
			gl.deleteShader(fs);
		};
	}, []);

	const handleKeyDown = useCallback(
		(e: KeyboardEvent<HTMLTextAreaElement>) => {
			if (e.key === "Enter" && !e.shiftKey) {
				e.preventDefault();
				submit(value);
			}
		},
		[submit, value],
	);

	return (
		<div className="w-full flex flex-col items-center gap-5 px-4 pt-14 pb-8 shrink-0">
			<style>{HERO_BUBBLE_CSS}</style>
			<div className="flex flex-col items-center gap-2 text-center">
				<h1 className="text-[26px] sm:text-3xl md:text-4xl font-bold tracking-tight text-balance">
					What do you want to <span className="sr-only">build</span>
					<RotatingWord />?
				</h1>
				<p className="text-sm md:text-base text-muted-foreground">
					Ask FlowPilot to create apps, find packages, or navigate Flow-Like.
				</p>
			</div>
			<div ref={wrapRef} className="hero-bubble-wrap">
				{webglFailed ? (
					<div className="hero-bubble-fallback" aria-hidden="true" />
				) : (
					<canvas ref={canvasRef} className="hero-bubble-canvas" />
				)}
				{MOTES.map((mote) => (
					<span
						key={`${mote.left}-${mote.top}`}
						className="hero-bubble-mote"
						style={{
							left: mote.left,
							top: mote.top,
							animationDelay: mote.delay,
						}}
						aria-hidden="true"
					/>
				))}
				<span
					className="hero-bubble-mote hero-bubble-mote-star"
					style={{ left: "3%", top: "34%", animationDelay: "1s" }}
					aria-hidden="true"
				>
					<FourPointStar size={14} />
				</span>
				<span
					className="hero-bubble-mote hero-bubble-mote-star"
					style={{ left: "93%", top: "-10%", animationDelay: "2.6s" }}
					aria-hidden="true"
				>
					<FourPointStar size={11} />
				</span>
				<span
					className="hero-bubble-mote hero-bubble-mote-star"
					style={{ left: "50%", top: "116%", animationDelay: "0.6s" }}
					aria-hidden="true"
				>
					<FourPointStar size={10} />
				</span>
				{MINI_BUBBLES.map((bubble) => (
					<span
						key={`${bubble.left}-${bubble.top}`}
						className="hero-bubble-minib"
						style={{
							left: bubble.left,
							top: bubble.top,
							width: bubble.size,
							height: bubble.size,
							animationDelay: bubble.delay,
						}}
						aria-hidden="true"
					/>
				))}
				<span className="hero-bubble-hint" aria-hidden="true">
					<SparklesIcon className="size-4.25" strokeWidth={1.8} />
					Click to ask FlowPilot
				</span>
				{SATELLITES.map(({ label, icon: Icon, strokeWidth, style }) => (
					<button
						key={label}
						type="button"
						className="hero-bubble-sat outline-none focus-visible:ring-2 focus-visible:ring-primary/40"
						style={style}
						onClick={() => submit(label)}
					>
						<Icon
							className="size-3.5"
							strokeWidth={strokeWidth}
							aria-hidden="true"
						/>
						{label}
					</button>
				))}
				<div className="hero-bubble-content">
					<div className="flex flex-col">
						<HeroFileChips files={files} onRemove={removeFile} />
						<Textarea
							value={value}
							onChange={(e) => setValue(e.target.value)}
							onKeyDown={handleKeyDown}
							placeholder="Ask FlowPilot anything, or describe what you want to build…"
							rows={1}
							aria-label="Ask FlowPilot"
							className="min-h-9 max-h-40 resize-none border-0 bg-transparent dark:bg-transparent shadow-none focus-visible:ring-0 py-1.5 px-2 text-[17px] text-foreground placeholder:text-muted-foreground"
						/>
					</div>
					<div className="hero-bubble-row">
						<span className="hero-bubble-spark" aria-hidden="true">
							<SparklesIcon className="size-4.75" strokeWidth={1.8} />
						</span>
						<HeroAgentControls />
					</div>
				</div>
				<div className="hero-bubble-side">
					<button
						type="button"
						className="hero-bubble-icon outline-none focus-visible:ring-2 focus-visible:ring-primary/40"
						aria-label="Attach images"
						onClick={openFilePicker}
					>
						<PaperclipIcon className="size-4.75" strokeWidth={1.8} />
					</button>
					<button
						type="button"
						className="hero-bubble-send outline-none focus-visible:ring-2 focus-visible:ring-primary/40"
						aria-label="Send"
						disabled={!canSend}
						onClick={() => submit(value)}
					>
						<ArrowUpIcon className="size-6" strokeWidth={2.2} />
					</button>
					<span
						className="hero-bubble-icon hero-bubble-icon-off"
						aria-hidden="true"
						title="Voice input isn't available yet"
					>
						<AudioLinesIcon className="size-4.75" strokeWidth={1.8} />
					</span>
				</div>
				<HeroFileInput inputRef={fileInputRef} onAdd={addFiles} />
			</div>
			{/* phone-width fallback: the floating satellites can't sit beside a
			    narrow bubble, so the suggestions reflow into a chip row below it */}
			<div className="hero-bubble-mobile-sats">
				{SATELLITES.map(({ label, icon: Icon, strokeWidth }) => (
					<button
						key={label}
						type="button"
						className="hero-bubble-mobile-chip outline-none focus-visible:ring-2 focus-visible:ring-primary/40"
						onClick={() => submit(label)}
					>
						<Icon
							className="size-3.5 shrink-0"
							strokeWidth={strokeWidth}
							aria-hidden="true"
						/>
						{label}
					</button>
				))}
			</div>
		</div>
	);
}
