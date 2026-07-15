"use client";

import { Button } from "@flow-like/flow-like-ui";
import { GitHubLogoIcon } from "@radix-ui/react-icons";
import { AnimatePresence, motion } from "framer-motion";
import {
	ArrowLeft,
	ArrowRight,
	BookOpen,
	Boxes,
	Cloud,
	GitFork,
	LayoutGrid,
	type LucideIcon,
	Package,
	Search,
	Sparkles,
	X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
	FRAG,
	VERT,
	readTokenRGB,
	themeIsLight,
} from "../../global-chat/hero-variants/bubble-shader";

const DOCS_URL = "https://docs.flow-like.com";
const DISCORD_URL = "https://discord.gg/mdBA9kMjFJ";
const GITHUB_URL = "https://github.com/Rheosoph/flow-like";

// A11y: honor reduced motion preference.
function usePrefersReducedMotion() {
	const [reduced, setReduced] = useState(false);
	useEffect(() => {
		const mql = window.matchMedia("(prefers-reduced-motion: reduce)");
		const onChange = () => setReduced(mql.matches);
		onChange();
		mql.addEventListener?.("change", onChange);
		return () => mql.removeEventListener?.("change", onChange);
	}, []);
	return reduced;
}

// The CC0 Discord glyph (simple-icons, CC0 1.0) — lucide ships no brand marks.
function DiscordIcon({ className }: { className?: string }) {
	return (
		<svg
			viewBox="0 0 24 24"
			fill="currentColor"
			aria-hidden="true"
			className={className}
		>
			<path d="M20.317 4.3698a19.7913 19.7913 0 0 0-4.8851-1.5152.0741.0741 0 0 0-.0785.0371c-.211.3753-.4447.8648-.6083 1.2495-1.8447-.2762-3.68-.2762-5.4868 0-.1636-.3933-.4058-.8742-.6177-1.2495a.077.077 0 0 0-.0785-.037 19.7363 19.7363 0 0 0-4.8852 1.515.0699.0699 0 0 0-.0321.0277C.5334 9.0458-.319 13.5799.0992 18.0578a.0824.0824 0 0 0 .0312.0561c2.0528 1.5076 4.0413 2.4228 5.9929 3.0294a.0777.0777 0 0 0 .0842-.0276c.4616-.6304.8731-1.2952 1.226-1.9942a.076.076 0 0 0-.0416-.1057c-.6528-.2476-1.2743-.5495-1.8722-.8923a.077.077 0 0 1-.0076-.1277c.1258-.0943.2517-.1923.3718-.2914a.0743.0743 0 0 1 .0776-.0105c3.9278 1.7933 8.18 1.7933 12.0614 0a.0739.0739 0 0 1 .0785.0095c.1202.099.246.1981.3728.2924a.077.077 0 0 1-.0066.1276 12.2986 12.2986 0 0 1-1.873.8914.0766.0766 0 0 0-.0407.1067c.3604.698.7719 1.3628 1.225 1.9932a.076.076 0 0 0 .0842.0286c1.961-.6067 3.9495-1.5219 6.0023-3.0294a.077.077 0 0 0 .0313-.0552c.5004-5.177-.8382-9.6739-3.5485-13.6604a.061.061 0 0 0-.0312-.0286zM8.02 15.3312c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9555-2.4189 2.157-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.9555 2.4189-2.1569 2.4189zm7.9748 0c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9554-2.4189 2.1569-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.946 2.4189-2.1568 2.4189Z" />
		</svg>
	);
}

// Reusable soap-film bubble — the exact FlowPilot shader (bubble-shader.ts), drawn as a free-floating
// round film (u_morph 0, full radius). Canvas is oversized + unclipped so the bloom fades to true
// transparency instead of a hard square edge, matching the launcher.
function BubbleOrb({ size = 120 }: { size?: number }) {
	const canvasRef = useRef<HTMLCanvasElement>(null);
	const [failed, setFailed] = useState(false);
	const canvasPx = Math.round(size * 2.11);
	const offset = (canvasPx - size) / 2;

	useEffect(() => {
		const canvas = canvasRef.current;
		if (!canvas) return;

		const gl = canvas.getContext("webgl", {
			alpha: true,
			premultipliedAlpha: true,
			antialias: false,
		});
		if (!gl) {
			setFailed(true);
			return;
		}

		const compile = (type: number, src: string) => {
			const shader = gl.createShader(type);
			if (!shader) return null;
			gl.shaderSource(shader, src);
			gl.compileShader(shader);
			if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
				gl.deleteShader(shader);
				return null;
			}
			return shader;
		};

		const vs = compile(gl.VERTEX_SHADER, VERT);
		const fs = compile(gl.FRAGMENT_SHADER, FRAG);
		const prog = vs && fs ? gl.createProgram() : null;
		if (!vs || !fs || !prog) {
			setFailed(true);
			return;
		}
		gl.attachShader(prog, vs);
		gl.attachShader(prog, fs);
		gl.linkProgram(prog);
		if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
			setFailed(true);
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

		const setPrimary = () => {
			const [r, g, b] = readTokenRGB("--primary", [242, 90, 60]);
			gl.uniform3f(uPrimary, r / 255, g / 255, b / 255);
		};
		setPrimary();

		let lightTarget = themeIsLight() ? 1 : 0;
		let light = lightTarget;
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

		const draw = (ms: number) => {
			gl.uniform2f(uRes, canvas.width, canvas.height);
			gl.uniform1f(uTime, ms / 1000);
			gl.uniform1f(uFocus, 0);
			gl.uniform2f(uBox, 0.48, 0.48);
			gl.uniform2f(uMouse, 0, 0);
			gl.uniform1f(uMstr, 0);
			gl.uniform1f(uMorph, 0);
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
			themeObserver.disconnect();
			gl.deleteBuffer(buf);
			gl.deleteProgram(prog);
			gl.deleteShader(vs);
			gl.deleteShader(fs);
		};
	}, []);

	if (failed) {
		return (
			<span
				aria-hidden="true"
				className="block rounded-full bg-linear-to-br from-primary/40 via-purple-500/30 to-purple-600/20 ring-1 ring-primary/40"
				style={{ width: size, height: size }}
			/>
		);
	}

	return (
		<span
			className="relative block"
			aria-hidden="true"
			style={{ width: size, height: size }}
		>
			<canvas
				ref={canvasRef}
				className="pointer-events-none absolute"
				style={{
					width: canvasPx,
					height: canvasPx,
					left: -offset,
					top: -offset,
				}}
			/>
		</span>
	);
}

interface Feature {
	icon: LucideIcon;
	title: string;
	body: string;
	tint: string;
}

interface StepDef {
	id: string;
	name: string;
	eyebrow: string;
	title: React.ReactNode;
	lead: React.ReactNode;
}

const STEPS: StepDef[] = [
	{
		id: "welcome",
		name: "Welcome",
		eyebrow: "Step 01 / 04",
		title: (
			<>
				Welcome to <span className="highlight">Flow-Like</span>
			</>
		),
		lead: (
			<>
				Flow-Like is a visual platform for building node-based automations —{" "}
				<b className="text-foreground">run them locally or in the cloud</b>, and
				extend them however you like.
			</>
		),
	},
	{
		id: "flowpilot",
		name: "FlowPilot",
		eyebrow: "Step 02 / 04 · First move",
		title: (
			<>
				Meet <span className="highlight">FlowPilot</span>, your copilot
			</>
		),
		lead: (
			<>
				<b className="text-foreground">Describe what you want to build</b> and
				FlowPilot does the wiring — it creates apps, finds packages, and
				navigates Flow-Like for you.
			</>
		),
	},
	{
		id: "explore",
		name: "Explore",
		eyebrow: "Step 03 / 04 · Second move",
		title: (
			<>
				Explore the <span className="highlight">app store</span>
			</>
		),
		lead: (
			<>
				<b className="text-foreground">Community apps, ready to use or fork.</b>{" "}
				Don't start from a blank canvas — install a template from Explore and
				make it yours in minutes.
			</>
		),
	},
	{
		id: "community",
		name: "Community",
		eyebrow: "Step 04 / 04 · Stay connected",
		title: (
			<>
				You're not building <span className="highlight">alone</span>
			</>
		),
		lead: "Get help, share what you make, and dig into the details whenever you need them.",
	},
];

const FEATURES: Record<string, Feature[]> = {
	welcome: [
		{
			icon: Boxes,
			title: "Design in nodes.",
			body: "Wire logic together on a visual canvas — no boilerplate.",
			tint: "text-primary",
		},
		{
			icon: Cloud,
			title: "Run anywhere.",
			body: "Execute on your machine or deploy to the cloud in one click.",
			tint: "text-[color:var(--tertiary)]",
		},
		{
			icon: Package,
			title: "Extend with packages.",
			body: "Add community apps and WASM nodes as you grow.",
			tint: "text-primary",
		},
	],
	flowpilot: [
		{
			icon: Sparkles,
			title: "Just describe it.",
			body: '"Build an app that summarizes my PDFs" — it starts building.',
			tint: "text-primary",
		},
		{
			icon: Search,
			title: "Finds the right nodes.",
			body: "It pulls in packages and connects them for you.",
			tint: "text-[color:var(--tertiary)]",
		},
	],
	explore: [
		{
			icon: LayoutGrid,
			title: "Browse & install.",
			body: "Ready-made apps you can run right away.",
			tint: "text-primary",
		},
		{
			icon: GitFork,
			title: "Fork & remix.",
			body: "Open any app, change what you need, publish your own.",
			tint: "text-[color:var(--tertiary)]",
		},
	],
};

function FeatureRow({ feature }: { feature: Feature }) {
	const Icon = feature.icon;
	return (
		<li className="flex items-start gap-3">
			<span
				className={`mt-0.5 flex size-8 flex-none items-center justify-center rounded-lg bg-muted ${feature.tint}`}
			>
				<Icon className="size-4" />
			</span>
			<div className="text-sm leading-snug">
				<b className="font-semibold">{feature.title}</b>{" "}
				<span className="text-muted-foreground">{feature.body}</span>
			</div>
		</li>
	);
}

const COMMUNITY_LINKS = [
	{
		href: DISCORD_URL,
		label: "Join our Discord",
		hint: "Ask questions, share builds, meet the community",
		icon: <DiscordIcon className="size-5" />,
		bg: "bg-[#5865F2]",
	},
	{
		href: DOCS_URL,
		label: "Read the docs",
		hint: "Quickstart, guides & the full node reference",
		icon: <BookOpen className="size-5" />,
		bg: "bg-primary",
	},
	{
		href: GITHUB_URL,
		label: "Star us on GitHub",
		hint: "Open source — explore the code & report issues",
		icon: <GitHubLogoIcon className="size-5" />,
		bg: "bg-foreground text-background",
	},
];

// Floating pin-coloured accent dots for the welcome stage (echo the node-editor data-type colours).
interface Dot {
	top?: string;
	left?: string;
	right?: string;
	bottom?: string;
	color: string;
	delay: string;
}
const WELCOME_DOTS: Dot[] = [
	{ top: "22%", left: "24%", color: "#00D3F2", delay: "0s" },
	{ top: "30%", right: "22%", color: "#FB64B6", delay: ".4s" },
	{ bottom: "26%", left: "30%", color: "#05DF72", delay: ".8s" },
	{ bottom: "30%", right: "28%", color: "#EEBD30", delay: ".2s" },
];

const FAN_CARDS = [
	{
		from: "#00D3F2",
		to: "#9810FA",
		rot: "-11deg",
		x: "-46px",
		z: 1,
		rating: "4.8 · Fork",
	},
	{
		from: "#05DF72",
		to: "#EEBD30",
		rot: "11deg",
		x: "46px",
		z: 1,
		rating: "4.9 · Use",
	},
	{
		from: "#FB562D",
		to: "#F19730",
		rot: "0deg",
		x: "0px",
		z: 3,
		rating: "4.9 · Use",
	},
];

function StageMotif({ stepId }: { stepId: string }) {
	if (stepId === "flowpilot") {
		return (
			<div className="relative flex items-center justify-center">
				<div className="absolute size-52 rounded-full bg-[#7c5cf0]/25 blur-3xl" />
				<BubbleOrb size={132} />
			</div>
		);
	}
	if (stepId === "explore") {
		return (
			<div className="relative h-40 w-56">
				{FAN_CARDS.map((c) => (
					<div
						key={c.rot + c.x}
						className="absolute left-1/2 top-1/2 h-36 w-28 overflow-hidden rounded-2xl border border-white/15 shadow-2xl"
						style={{
							background: "#0e1016",
							transform: `translate(-50%,-50%) translateX(${c.x}) rotate(${c.rot}) ${c.z === 3 ? "scale(1.05)" : ""}`,
							zIndex: c.z,
						}}
					>
						<div
							className="h-16 w-full"
							style={{
								background: `linear-gradient(140deg, ${c.from}, ${c.to})`,
							}}
						/>
						<div className="flex flex-col gap-1.5 p-2.5">
							<span
								className="-mt-5 size-6 rounded-md border-2"
								style={{
									borderColor: "#0e1016",
									background: `linear-gradient(140deg, ${c.from}, ${c.to})`,
								}}
							/>
							<span className="h-1.5 rounded bg-white/15" />
							<span className="h-1.5 w-3/5 rounded bg-white/15" />
							<span className="font-mono text-[8px] text-white/60">
								★ {c.rating}
							</span>
						</div>
					</div>
				))}
			</div>
		);
	}
	if (stepId === "community") {
		return (
			<div className="flex gap-4">
				{[
					{
						bg: "#5865F2",
						node: <DiscordIcon className="size-8 text-white" />,
					},
					{
						bg: "linear-gradient(150deg,#FB562D,#F19730)",
						node: <BookOpen className="size-8 text-white" />,
					},
					{
						bg: "#24292f",
						node: <GitHubLogoIcon className="size-8 text-white" />,
					},
				].map((t, i) => (
					<span
						key={t.bg}
						className="fl-onb-bob grid size-[70px] place-items-center rounded-2xl border border-white/15 shadow-xl"
						style={{ background: t.bg, animationDelay: `${i * 0.5}s` }}
					>
						{t.node}
					</span>
				))}
			</div>
		);
	}
	// welcome
	return (
		<div className="relative flex flex-col items-center">
			<div className="absolute inset-0 -z-10">
				{WELCOME_DOTS.map((d) => (
					<span
						key={d.color}
						className="fl-onb-bob absolute size-2.5 rounded-full"
						style={{
							top: d.top,
							left: d.left,
							right: d.right,
							bottom: d.bottom,
							background: d.color,
							boxShadow: `0 0 12px 2px ${d.color}`,
							animationDelay: d.delay,
						}}
					/>
				))}
			</div>
			<img
				src="/app-logo.webp"
				alt="Flow-Like"
				className="size-24 drop-shadow-[0_8px_30px_rgba(0,0,0,0.5)]"
			/>
			<span className="mt-4 font-mono text-[11px] uppercase tracking-[0.24em] text-white/70">
				visual · local · anywhere
			</span>
		</div>
	);
}

export function TutorialDialog() {
	const [showTutorial, setShowTutorial] = useState(false);
	const [step, setStep] = useState(0);
	const [supportsBackdrop, setSupportsBackdrop] = useState(true);
	const reduced = usePrefersReducedMotion();
	const total = STEPS.length;
	const active = STEPS[step];

	useEffect(() => {
		setShowTutorial(localStorage.getItem("tutorial-finished") !== "true");
	}, []);

	// Lock background scroll while the tour is shown.
	useEffect(() => {
		if (!showTutorial) return;
		const prev = document.body.style.overflow;
		document.body.style.overflow = "hidden";
		return () => {
			document.body.style.overflow = prev;
		};
	}, [showTutorial]);

	// Detect backdrop-filter support with a Linux/WebKit hard fallback.
	useEffect(() => {
		try {
			const ua = navigator.userAgent.toLowerCase();
			const isLinux = ua.includes("linux");
			const hasBackdrop =
				typeof CSS !== "undefined" &&
				(CSS.supports("backdrop-filter", "blur(4px)") ||
					CSS.supports("-webkit-backdrop-filter", "blur(4px)"));
			const isWebKit =
				/applewebkit\//.test(ua) && !/chrome\//.test(ua)
					? true
					: /webkit/.test(ua);
			setSupportsBackdrop(hasBackdrop && !(isLinux && isWebKit));
		} catch {
			setSupportsBackdrop(false);
		}
	}, []);

	const finish = useCallback(() => {
		localStorage.setItem("tutorial-finished", "true");
		setShowTutorial(false);
	}, []);

	const goTo = useCallback(
		(n: number) => setStep(Math.max(0, Math.min(total - 1, n))),
		[total],
	);

	const next = useCallback(() => {
		setStep((s) => {
			if (s < total - 1) return s + 1;
			finish();
			return s;
		});
	}, [finish, total]);

	const prev = useCallback(() => goTo(step - 1), [goTo, step]);

	useEffect(() => {
		if (!showTutorial) return;
		const onKey = (e: KeyboardEvent) => {
			if (e.key === "Escape") finish();
			else if (e.key === "ArrowRight") next();
			else if (e.key === "ArrowLeft") prev();
		};
		window.addEventListener("keydown", onKey);
		return () => window.removeEventListener("keydown", onKey);
	}, [showTutorial, finish, next, prev]);

	const fade = useMemo(
		() =>
			reduced
				? { initial: false, animate: {}, exit: {}, transition: { duration: 0 } }
				: {
						initial: { opacity: 0, x: 16 },
						animate: { opacity: 1, x: 0 },
						exit: { opacity: 0, x: -16 },
						transition: { duration: 0.32, ease: "easeOut" },
					},
		[reduced],
	);

	if (!showTutorial) return null;

	return (
		<div
			className="fixed inset-0 z-50 flex items-center justify-center p-0 sm:p-6"
			// biome-ignore lint/a11y/useSemanticElements: custom-composited overlay; a native <dialog> cannot host the WebGL stage + framer-motion layout
			role="dialog"
			aria-modal="true"
			aria-label="Getting started with Flow-Like"
		>
			<style>{`
@keyframes fl-onb-drift{0%{transform:translate3d(-3%,-2%,0) scale(1.05)}100%{transform:translate3d(4%,3%,0) scale(1.12)}}
@keyframes fl-onb-bob{0%,100%{transform:translateY(0)}50%{transform:translateY(-10px)}}
.fl-onb-bob{animation:fl-onb-bob 5s ease-in-out infinite}
@media (prefers-reduced-motion: reduce){.fl-onb-bob{animation:none}}
`}</style>

			{/* Scrim */}
			<button
				type="button"
				aria-label="Close getting started"
				onClick={finish}
				className={`absolute inset-0 bg-background/80 ${supportsBackdrop ? "backdrop-blur-md" : "bg-background/92"}`}
			/>

			{/* Panel */}
			<div className="relative flex h-full w-full flex-col overflow-hidden border-border bg-card shadow-2xl sm:h-auto sm:max-h-[88dvh] sm:w-full sm:max-w-[1000px] sm:grid sm:grid-cols-[minmax(0,0.82fr)_1fr] sm:rounded-2xl sm:border">
				{/* ── LEFT: branded stage (committed warm-dark in both themes) ── */}
				<aside
					className="relative flex h-40 shrink-0 items-center justify-center overflow-hidden text-white sm:h-auto"
					style={{
						background:
							"radial-gradient(120% 90% at 20% 15%, #2a1410, #160a12 55%, #0a0810 100%)",
					}}
					aria-hidden="true"
				>
					<div
						className="pointer-events-none absolute -inset-[30%] opacity-90 blur-md"
						style={{
							background:
								"radial-gradient(38% 42% at 26% 28%, rgba(251,86,45,.85), transparent 60%),radial-gradient(40% 44% at 74% 40%, rgba(241,151,48,.6), transparent 60%),radial-gradient(46% 46% at 55% 88%, rgba(124,92,240,.55), transparent 62%),radial-gradient(34% 34% at 84% 78%, rgba(238,189,48,.4), transparent 60%)",
							animation: reduced
								? undefined
								: "fl-onb-drift 14s ease-in-out infinite alternate",
						}}
					/>
					<div className="absolute left-6 top-5 z-10 flex items-center gap-2.5">
						<img src="/app-logo.webp" alt="" className="size-6 rounded-md" />
						<b className="text-sm font-bold tracking-tight drop-shadow">
							Flow-Like
						</b>
					</div>

					<div className="relative z-[2] flex items-center justify-center px-6">
						<AnimatePresence mode="wait">
							<motion.div
								key={active.id}
								initial={reduced ? false : { opacity: 0, scale: 0.94 }}
								animate={{ opacity: 1, scale: 1 }}
								exit={reduced ? {} : { opacity: 0, scale: 0.96 }}
								transition={{ duration: 0.4, ease: "easeOut" }}
								className="flex items-center justify-center"
							>
								<StageMotif stepId={active.id} />
							</motion.div>
						</AnimatePresence>
					</div>

					<span className="absolute bottom-5 left-6 z-10 hidden font-mono text-[11px] uppercase tracking-[0.14em] text-white/70 sm:block">
						Step 0{step + 1} — {active.name}
					</span>
				</aside>

				{/* ── RIGHT: stepper content ── */}
				<div className="relative flex min-h-0 flex-1 flex-col p-6 sm:p-8">
					<button
						type="button"
						onClick={finish}
						aria-label="Close"
						className="absolute right-4 top-4 grid size-8 place-items-center rounded-lg text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/50"
					>
						<X className="size-4" />
					</button>

					{/* Progress track */}
					<nav className="mb-6 flex items-center pr-10" aria-label="Progress">
						{STEPS.map((s, i) => (
							<div
								key={s.id}
								className={`flex items-center ${i === total - 1 ? "" : "flex-1"}`}
							>
								<button
									type="button"
									onClick={() => goTo(i)}
									aria-label={`Step ${i + 1}: ${s.name}`}
									aria-current={i === step ? "step" : undefined}
									className={`grid size-7 flex-none place-items-center rounded-full border font-mono text-[11px] font-semibold transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/50 ${
										i <= step
											? "border-transparent bg-primary text-primary-foreground shadow"
											: "border-border bg-muted text-muted-foreground"
									} ${i === step ? "scale-110" : ""}`}
								>
									{i + 1}
								</button>
								{i !== total - 1 && (
									<span className="mx-1.5 h-0.5 flex-1 overflow-hidden rounded-full bg-border">
										<span
											className="block h-full bg-primary transition-[width] duration-300"
											style={{ width: i < step ? "100%" : "0%" }}
										/>
									</span>
								)}
							</div>
						))}
					</nav>

					{/* Panes */}
					<div className="relative min-h-0 flex-1 overflow-y-auto">
						<AnimatePresence mode="wait">
							<motion.section
								key={active.id}
								{...fade}
								className="flex flex-col"
							>
								<span className="font-mono text-[11px] font-semibold uppercase tracking-[0.16em] text-primary">
									{active.eyebrow}
								</span>
								<h2 className="mt-3 text-balance text-2xl font-extrabold leading-tight tracking-tight sm:text-[2rem]">
									{active.title}
								</h2>
								<p className="mt-3.5 max-w-[46ch] text-[15px] leading-relaxed text-muted-foreground">
									{active.lead}
								</p>

								{FEATURES[active.id] && (
									<ul className="mt-5 flex flex-col gap-3">
										{FEATURES[active.id].map((f) => (
											<FeatureRow key={f.title} feature={f} />
										))}
									</ul>
								)}

								{active.id === "flowpilot" && (
									<div className="mt-4 flex items-center gap-3 rounded-xl border border-border bg-muted/50 p-3">
										<BubbleOrb size={40} />
										<p className="text-[13px] leading-snug text-muted-foreground">
											<b className="text-foreground">
												Look to the bottom-right corner.
											</b>{" "}
											The shimmering bubble is FlowPilot — click it anytime to
											start a chat.
										</p>
									</div>
								)}

								{active.id === "community" && (
									<div className="mt-5 flex flex-col gap-2.5">
										{COMMUNITY_LINKS.map((l) => (
											<a
												key={l.href}
												href={l.href}
												target="_blank"
												rel="noopener noreferrer"
												className="group flex items-center gap-3.5 rounded-xl border border-border bg-card p-3.5 transition-all hover:-translate-y-0.5 hover:border-primary/40 hover:shadow-lg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/50"
											>
												<span
													className={`grid size-10 flex-none place-items-center rounded-lg text-white ${l.bg}`}
												>
													{l.icon}
												</span>
												<span className="flex flex-col leading-tight">
													<b className="text-sm font-bold">{l.label}</b>
													<small className="text-xs text-muted-foreground">
														{l.hint}
													</small>
												</span>
												<ArrowRight className="ml-auto size-4 text-muted-foreground transition-transform group-hover:translate-x-0.5 group-hover:text-foreground" />
											</a>
										))}
									</div>
								)}
							</motion.section>
						</AnimatePresence>
					</div>

					{/* Nav */}
					<div className="mt-5 flex items-center gap-3 border-t border-border/70 pt-4">
						<Button
							variant="outline"
							onClick={prev}
							disabled={step === 0}
							className="rounded-xl"
						>
							<ArrowLeft className="size-4" />
							Back
						</Button>
						<Button
							variant="ghost"
							onClick={finish}
							className="rounded-xl text-muted-foreground"
						>
							Skip intro
						</Button>
						<Button onClick={next} className="ml-auto rounded-xl">
							{step === total - 1 ? "Start building" : "Next"}
							<ArrowRight className="size-4" />
						</Button>
					</div>
				</div>
			</div>
		</div>
	);
}
