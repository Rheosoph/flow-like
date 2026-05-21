import { GitHubLogoIcon } from "@radix-ui/react-icons";
import { Button } from "@flow-like/flow-like-ui";
import { Book, Heart, MessageCircle, Rocket, Zap } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

type WelcomeStep = "welcome" | "docs" | "discord" | "github";

const STEPS: WelcomeStep[] = ["welcome", "docs", "discord", "github"];

interface AnimatedBackgroundProps {
	children: React.ReactNode;
	variant?: WelcomeStep;
}

// A11y: honor reduced motion preference
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

// Spectacular animated starfield background with parallax and comets
function UniverseBackground({
	variant = "welcome",
}: { variant?: WelcomeStep }) {
	const canvasRef = useRef<HTMLCanvasElement | null>(null);
	const rafRef = useRef<number | null>(null);
	const starsRef = useRef<
		Array<{
			x: number;
			y: number;
			z: number; // depth 0..1
			size: number;
			baseAlpha: number;
			twinkle: number;
			hue: number;
		}>
	>([]);
	const cometsRef = useRef<
		Array<{
			x: number;
			y: number;
			vx: number;
			vy: number;
			life: number;
			maxLife: number;
		}>
	>([]);
	const pointerRef = useRef<{ x: number; y: number }>({ x: 0, y: 0 });
	const reducedMotion = usePrefersReducedMotion();

	const palette = useMemo(() => {
		switch (variant) {
			case "discord":
				return { hueMin: 230, hueMax: 260 };
			case "docs":
				return { hueMin: 200, hueMax: 220 };
			case "github":
				return { hueMin: 0, hueMax: 0 };
			default:
				return { hueMin: 250, hueMax: 300 };
		}
	}, [variant]);

	const resizeCanvas = useCallback((canvas: HTMLCanvasElement) => {
		const dpr = Math.min(window.devicePixelRatio || 1, 2);
		const { clientWidth, clientHeight } = canvas;
		canvas.width = Math.floor(clientWidth * dpr);
		canvas.height = Math.floor(clientHeight * dpr);
		const ctx = canvas.getContext("2d");
		if (ctx) ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
	}, []);

	const seedStars = useCallback(
		(canvas: HTMLCanvasElement) => {
			const area = canvas.clientWidth * canvas.clientHeight;
			const density = reducedMotion ? 0.00015 : 0.00035; // stars per px
			const count = Math.max(80, Math.min(900, Math.floor(area * density)));
			const arr: typeof starsRef.current = [];
			for (let i = 0; i < count; i++) {
				const z = Math.random();
				const size = 0.7 + Math.pow(z, 2) * 1.8;
				const baseAlpha = 0.25 + Math.random() * 0.55;
				const twinkle = 0.5 + Math.random() * 1.5;
				const hue =
					palette.hueMin + Math.random() * (palette.hueMax - palette.hueMin);
				arr.push({
					x: Math.random() * canvas.clientWidth,
					y: Math.random() * canvas.clientHeight,
					z,
					size,
					baseAlpha,
					twinkle,
					hue,
				});
			}
			starsRef.current = arr;
		},
		[palette, reducedMotion],
	);

	useEffect(() => {
		const canvas = canvasRef.current;
		if (!canvas) return;
		resizeCanvas(canvas);
		seedStars(canvas);

		let last = performance.now();
		const ctx = canvas.getContext("2d");
		if (!ctx) return;

		const draw = (now: number) => {
			const dt = Math.min(0.05, (now - last) / 1000);
			last = now;
			ctx.clearRect(0, 0, canvas.clientWidth, canvas.clientHeight);

			const centerX = canvas.clientWidth / 2;
			const centerY = canvas.clientHeight / 2;
			const parallaxX = (pointerRef.current.x - centerX) / centerX;
			const parallaxY = (pointerRef.current.y - centerY) / centerY;

			// Stars
			for (let i = 0; i < starsRef.current.length; i++) {
				const s = starsRef.current[i];
				const depth = 0.3 + s.z * 0.7;
				const px = s.x + parallaxX * (1 - depth) * 12;
				const py = s.y + parallaxY * (1 - depth) * 12;
				const alpha =
					s.baseAlpha *
					(reducedMotion
						? 1
						: 0.5 + 0.5 * Math.sin(now * 0.002 * s.twinkle + i));

				ctx.beginPath();
				ctx.fillStyle = `hsla(${s.hue}, 80%, ${variant === "github" ? 88 : 74}%, ${alpha})`;
				ctx.shadowBlur = 8 * depth;
				ctx.shadowColor = `hsla(${s.hue}, 100%, 70%, ${alpha})`;
				ctx.arc(px, py, s.size * depth, 0, Math.PI * 2);
				ctx.fill();
			}

			// Comets
			if (!reducedMotion && Math.random() < 0.012) {
				const fromTop = Math.random() < 0.5;
				const x = fromTop ? Math.random() * canvas.clientWidth : -40;
				const y = fromTop ? -40 : Math.random() * canvas.clientHeight * 0.4;
				const angle = (fromTop ? 1 : 0.7) + Math.random() * 0.3;
				const speed = 280 + Math.random() * 140;
				cometsRef.current.push({
					x,
					y,
					vx: speed * angle,
					vy: speed * 0.45,
					life: 0,
					maxLife: 1.8 + Math.random() * 0.8,
				});
			}

			ctx.globalCompositeOperation = "lighter";
			for (let i = cometsRef.current.length - 1; i >= 0; i--) {
				const c = cometsRef.current[i];
				c.life += dt;
				c.x += c.vx * dt;
				c.y += c.vy * dt;
				const t = 1 - c.life / c.maxLife;
				if (t <= 0) {
					cometsRef.current.splice(i, 1);
					continue;
				}
				const grad = ctx.createLinearGradient(c.x, c.y, c.x - 120, c.y - 60);
				grad.addColorStop(0, `hsla(${palette.hueMax}, 100%, 80%, ${0.7 * t})`);
				grad.addColorStop(1, `hsla(${palette.hueMin}, 100%, 60%, 0)`);
				ctx.strokeStyle = grad;
				ctx.lineWidth = 2;
				ctx.beginPath();
				ctx.moveTo(c.x, c.y);
				ctx.lineTo(c.x - 120, c.y - 60);
				ctx.stroke();
				ctx.beginPath();
				ctx.fillStyle = `hsla(${palette.hueMax}, 100%, 90%, ${0.9 * t})`;
				ctx.arc(c.x, c.y, 1.8 + 2.2 * t, 0, Math.PI * 2);
				ctx.fill();
			}
			ctx.globalCompositeOperation = "source-over";

			// Aurora waves (subtle flowing ribbons)
			if (!reducedMotion) {
				const baseHueA = palette.hueMin;
				const baseHueB = palette.hueMax || palette.hueMin + 20;
				const w = canvas.clientWidth;
				const h = canvas.clientHeight;

				const drawAurora = (
					offset: number,
					amp: number,
					thickness: number,
					speed: number,
				) => {
					ctx.save();
					ctx.globalCompositeOperation = "lighter";
					ctx.globalAlpha = 0.08;
					ctx.lineWidth = thickness;
					const grad = ctx.createLinearGradient(
						0,
						h * 0.25 + offset,
						w,
						h * 0.75 + offset,
					);
					grad.addColorStop(0, `hsla(${baseHueA}, 100%, 70%, 0.6)`);
					grad.addColorStop(0.5, `hsla(${baseHueB}, 100%, 60%, 0.35)`);
					grad.addColorStop(1, `hsla(${baseHueA}, 100%, 75%, 0.5)`);
					ctx.strokeStyle = grad;
					ctx.beginPath();
					const k = now * 0.001 * speed;
					for (let x = -50; x <= w + 50; x += 6) {
						const y =
							h * 0.5 +
							Math.sin(x * 0.008 + k + offset) * amp +
							Math.cos(x * 0.015 - k * 0.6) * (amp * 0.4);
						if (x === -50) ctx.moveTo(x, y);
						else ctx.lineTo(x, y);
					}
					ctx.stroke();
					ctx.restore();
				};
				drawAurora(-60, 32, 20, 1);
				drawAurora(40, 22, 14, 1.4);
			}

			if (!reducedMotion) rafRef.current = requestAnimationFrame(draw);
		};

		if (reducedMotion) {
			draw(performance.now());
			return; // draw once, no animation
		}

		rafRef.current = requestAnimationFrame(draw);

		const onResize = () => {
			resizeCanvas(canvas);
			seedStars(canvas);
		};
		const onPointerMove = (e: PointerEvent) => {
			pointerRef.current.x = e.clientX;
			pointerRef.current.y = e.clientY;
		};
		window.addEventListener("resize", onResize);
		window.addEventListener("pointermove", onPointerMove, { passive: true });
		return () => {
			if (rafRef.current) cancelAnimationFrame(rafRef.current);
			window.removeEventListener("resize", onResize);
			window.removeEventListener("pointermove", onPointerMove as any);
		};
	}, [palette, reducedMotion, resizeCanvas, seedStars, variant]);

	return (
		<div className="absolute inset-0 pointer-events-none">
			{/* Soft nebulas using theme colors to avoid hard-coded color tokens */}
			<div className="absolute -top-24 -left-24 w-[40vw] h-[40vw] max-w-[680px] max-h-[680px] rounded-full bg-primary/15 blur-3xl" />
			<div className="absolute -bottom-24 -right-24 w-[35vw] h-[35vw] max-w-[560px] max-h-[560px] rounded-full bg-secondary/15 blur-3xl" />
			<div className="absolute top-1/4 left-[10%] w-72 h-72 rounded-full bg-accent/10 blur-2xl" />
			<canvas ref={canvasRef} className="absolute inset-0 w-full h-full" />
		</div>
	);
}

export function TutorialDialog() {
	const [showTutorial, setShowTutorial] = useState(false);
	const [currentStep, setCurrentStep] = useState<WelcomeStep>("welcome");
	const [supportsBackdrop, setSupportsBackdrop] = useState<boolean>(true);
	const containerRef = useRef<HTMLDivElement | null>(null);
	const confettiCanvasRef = useRef<HTMLCanvasElement | null>(null);
	const touchStartXRef = useRef<number | null>(null);
	const lastPointerMoveTs = useRef<number>(0);
	const reducedMotion = usePrefersReducedMotion();

	useEffect(() => {
		const hasFinishedTutorial = localStorage.getItem("tutorial-finished");
		setShowTutorial(hasFinishedTutorial !== "true");
	}, []);

	// Lock background scroll while tutorial is shown
	useEffect(() => {
		if (showTutorial) {
			const prev = document.body.style.overflow;
			document.body.style.overflow = "hidden";
			return () => {
				document.body.style.overflow = prev;
			};
		}
	}, [showTutorial]);

	// Detect backdrop-filter support and provide a Linux WebKit fallback.
	useEffect(() => {
		try {
			const ua = navigator.userAgent.toLowerCase();
			const isLinux = ua.includes("linux");
			const hasBackdrop =
				(CSS &&
					(CSS as any).supports &&
					(CSS as any).supports("backdrop-filter", "blur(4px)")) ||
				(CSS &&
					(CSS as any).supports &&
					(CSS as any).supports("-webkit-backdrop-filter", "blur(4px)"));
			// Some Linux WebKit builds lie about supports; prefer hard fallback on Linux + WebKit.
			const isWebKit =
				/applewebkit\//.test(ua) && !/chrome\//.test(ua)
					? true
					: /webkit/.test(ua);
			const forceFallback = isLinux && isWebKit;
			setSupportsBackdrop(Boolean(hasBackdrop) && !forceFallback);
		} catch {
			setSupportsBackdrop(false);
		}
	}, []);

	// Keyboard navigation and quick-exit
	useEffect(() => {
		if (!showTutorial) return;
		const onKey = (e: KeyboardEvent) => {
			if (e.key === "Escape") {
				handleSkip(true);
			} else if (e.key === "ArrowRight") {
				handleNext();
			} else if (e.key === "ArrowLeft") {
				handlePrevious();
			}
		};
		window.addEventListener("keydown", onKey);
		return () => window.removeEventListener("keydown", onKey);
	}, [showTutorial, currentStep]);

	const handleSkip = (celebrate = false) => {
		localStorage.setItem("tutorial-finished", "true");
		if (celebrate && !reducedMotion) {
			setTimeout(() => setShowTutorial(false), 500);
		} else {
			setShowTutorial(false);
		}
	};

	const handleNext = () => {
		const currentIndex = STEPS.indexOf(currentStep);
		if (currentIndex < STEPS.length - 1) {
			setCurrentStep(STEPS[currentIndex + 1]);
		} else {
			handleSkip(true);
		}
	};

	const handlePrevious = () => {
		const currentIndex = STEPS.indexOf(currentStep);
		if (currentIndex > 0) {
			setCurrentStep(STEPS[currentIndex - 1]);
		}
	};

	const getBackgroundGradient = (variant: WelcomeStep) => {
		switch (variant) {
			case "discord":
				return "bg-linear-to-br from-[#5865F2]/50 via-[#7289DA]/12 to-[#5865F2]/36";
			case "github":
				return "bg-linear-to-br from-foreground/20 via-foreground/10 to-foreground/8";
			case "docs":
				return "bg-linear-to-br from-primary/20 via-blue-500/10 to-primary/15";
			default:
				return "bg-linear-to-br from-primary/18 via-purple-500/8 to-secondary/12";
		}
	};

	const AnimatedBackground = ({
		children,
		variant = "welcome",
	}: AnimatedBackgroundProps) => (
		<div className="relative min-h-screen flex items-center justify-center p-2 sm:p-6 bg-background/90 backdrop-blur-sm">
			<div className={`absolute inset-0 ${getBackgroundGradient(variant)}`} />
			{/* Enhanced cosmic spectacle */}
			<UniverseBackground variant={variant} />
			{/* Soft pulses toned down to let the universe shine */}
			<div
				className="absolute inset-0 bg-linear-to-tr from-accent/10 via-transparent to-primary/10 animate-pulse opacity-30 sm:opacity-80"
				style={{ animationDuration: "10s" }}
			/>
			<div
				className="absolute inset-0 bg-linear-to-bl from-secondary/8 via-transparent to-accent/8 animate-pulse opacity-30 sm:opacity-80"
				style={{ animationDuration: "14s", animationDelay: "5s" }}
			/>

			{/* Confetti overlay */}
			<canvas
				ref={confettiCanvasRef}
				className="absolute inset-0 w-full h-full pointer-events-none z-[11]"
			/>

			<div className="relative z-10 w-full h-full flex items-stretch justify-center">
				{children}
			</div>
		</div>
	);

	const stepData = {
		welcome: {
			title: "Welcome to Flow Like",
			description:
				"Your comprehensive solution for modern software development",
		},
		docs: {
			title: "Documentation & Guides",
			description: "Everything you need to master Flow Like effectively",
		},
		discord: {
			title: "Join Our Community",
			description: "Connect with developers, get help, and share feedback",
		},
		github: {
			title: "Open Source Project",
			description: "Explore, contribute, and customize Flow Like",
		},
	};

	const FeatureItem = ({
		icon: Icon,
		text,
		color = "primary",
	}: {
		icon: React.ComponentType<{ className?: string }>;
		text: string;
		color?: string;
	}) => (
		<div
			className={
				supportsBackdrop
					? "flex items-center gap-3 p-3 rounded-xl bg-background/30 backdrop-blur-md border border-border/40 shadow-lg"
					: "flex items-center gap-3 p-3 rounded-xl bg-card border border-border/60 shadow-md"
			}
		>
			<Icon className={`w-5 h-5 text-${color}`} />
			<span className="text-sm font-medium">{text}</span>
		</div>
	);

	const BulletPoint = ({
		text,
		color = "primary",
	}: { text: string; color?: string }) => (
		<div className="flex items-center gap-2 text-sm">
			<div className={`w-2 h-2 bg-${color} rounded-full`} />
			<span>{text}</span>
		</div>
	);

	const WelcomeStep = () => (
		<div className="grid grid-cols-1 sm:grid-cols-2 gap-4 sm:gap-8 h-full p-4 sm:p-8">
			<div className="flex flex-col justify-center">
				<div className="relative mb-6">
					<img
						src="/app-logo.webp"
						alt="Flow Like Logo"
						className="w-24 h-24 sm:w-28 sm:h-28 mx-auto"
					/>
				</div>
				<h2 className="text-3xl sm:text-4xl font-bold text-center mb-4">
					<span className="text-primary">Flow</span> Like
				</h2>
				<div className="w-20 h-0.5 bg-primary mx-auto" />
			</div>

			<div className="flex flex-col justify-center space-y-4 sm:space-y-6">
				<div className="space-y-3">
					<FeatureItem icon={Rocket} text="Scalable Development" />
					<FeatureItem icon={Zap} text="Lightning Fast Performance" />
					<FeatureItem icon={Heart} text="Developer Friendly" />
				</div>
			</div>
		</div>
	);

	const DocsStep = () => (
		<div className="grid grid-cols-1 sm:grid-cols-2 gap-4 sm:gap-8 h-full p-4 sm:p-8">
			<div className="flex flex-col justify-center items-center">
				<div
					className={
						supportsBackdrop
							? "w-14 h-14 sm:w-28 sm:h-28 rounded-md sm:rounded-xl bg-primary/20 backdrop-blur-md flex items-center justify-center mb-6 border border-primary/30 shadow-lg"
							: "w-14 h-14 sm:w-28 sm:h-28 rounded-md sm:rounded-xl bg-primary/15 flex items-center justify-center mb-6 border border-primary/30 shadow-lg"
					}
				>
					<Book className="w-7 h-7 sm:w-14 sm:h-14 text-primary" />
				</div>
			</div>

			<div className="flex flex-col justify-center items-center sm:items-start pb-4 sm:pb-0 space-y-4 sm:space-y-6">
				<div className="space-y-3">
					<BulletPoint text="Quick Start Guide" />
					<BulletPoint text="API Reference" />
					<BulletPoint text="Best Practices" />
					<BulletPoint text="Advanced Features" />
				</div>
				<Button
					className={
						supportsBackdrop
							? "gap-2 w-fit bg-primary/90 backdrop-blur-xs hover:bg-primary"
							: "gap-2 w-fit bg-primary hover:bg-primary/90"
					}
					onClick={() => window.open("https://docs.flow-like.com", "_blank")}
				>
					<Book className="w-4 h-4" />
					Open Documentation
				</Button>
			</div>
		</div>
	);

	const DiscordStep = () => (
		<div className="grid grid-cols-1 sm:grid-cols-2 gap-4 sm:gap-8 h-full p-4 sm:p-8">
			<div className="flex flex-col justify-center items-center">
				<div
					className={
						supportsBackdrop
							? "w-14 h-14 sm:w-28 sm:h-28 rounded-md sm:rounded-xl bg-[#5865F2]/20 backdrop-blur-md flex items-center justify-center mb-6 border border-[#5865F2]/30 shadow-lg"
							: "w-14 h-14 sm:w-28 sm:h-28 rounded-md sm:rounded-xl bg-[#5865F2]/15 flex items-center justify-center mb-6 border border-[#5865F2]/30 shadow-lg"
					}
				>
					<MessageCircle className="w-7 h-7 sm:w-14 sm:h-14 text-[#5865F2]" />
				</div>
			</div>

			<div className="flex flex-col justify-center space-y-4 sm:space-y-6 items-center sm:items-start pb-4 sm:pb-0">
				<div className="space-y-3">
					<BulletPoint text="Get Help & Support" color="[#5865F2]" />
					<BulletPoint text="Share Your Projects" color="[#5865F2]" />
					<BulletPoint text="Feature Discussions" color="[#5865F2]" />
					<BulletPoint text="Connect with Developers" color="[#5865F2]" />
				</div>
				<Button
					className={
						supportsBackdrop
							? "gap-2 w-fit bg-[#5865F2]/90 backdrop-blur-xs hover:bg-[#5865F2] text-white"
							: "gap-2 w-fit bg-[#5865F2] hover:bg-[#5865F2]/90 text-white"
					}
					onClick={() => window.open("https://discord.gg/mdBA9kMjFJ", "_blank")}
				>
					<MessageCircle className="w-4 h-4" />
					Join Discord
				</Button>
			</div>
		</div>
	);

	const GithubStep = () => (
		<div className="grid grid-cols-1 sm:grid-cols-2 gap-4 sm:gap-8 h-full p-4 sm:p-8">
			<div className="flex flex-col justify-center items-center">
				<div
					className={
						supportsBackdrop
							? "w-14 h-14 sm:w-28 sm:h-28 rounded-md sm:rounded-xl bg-foreground/20 backdrop-blur-md flex items-center justify-center mb-6 border border-foreground/30 shadow-lg"
							: "w-14 h-14 sm:w-28 sm:h-28 rounded-md sm:rounded-xl bg-foreground/15 flex items-center justify-center mb-6 border border-foreground/30 shadow-lg"
					}
				>
					<GitHubLogoIcon className="w-7 h-7 sm:w-14 sm:h-14 text-foreground" />
				</div>
			</div>

			<div className="flex flex-col justify-center space-y-4 sm:space-y-6 items-center sm:items-start pb-4 sm:pb-0">
				<div className="space-y-3">
					<BulletPoint text="Explore Source Code" color="foreground" />
					<BulletPoint text="Report Issues" color="foreground" />
					<BulletPoint text="Submit Pull Requests" color="foreground" />
					<BulletPoint text="Star the Repository" color="foreground" />
				</div>
				<Button
					variant="outline"
					className={
						supportsBackdrop
							? "gap-2 w-fit border-foreground/40 hover:bg-foreground/10 bg-background/30 backdrop-blur-xs"
							: "gap-2 w-fit border-foreground/40 hover:bg-foreground/10 bg-card"
					}
					onClick={() =>
						window.open("https://github.com/Rheosoph/flow-like", "_blank")
					}
				>
					<GitHubLogoIcon className="w-4 h-4" />
					View Repository
				</Button>
			</div>
		</div>
	);

	const stepComponents = {
		welcome: WelcomeStep,
		docs: DocsStep,
		discord: DiscordStep,
		github: GithubStep,
	};

	const CurrentStepComponent = stepComponents[currentStep];

	if (!showTutorial) return null;

	return (
		<div
			className="fixed inset-0 z-50"
			role="dialog"
			aria-modal="true"
			aria-label="Welcome tour"
		>
			<AnimatedBackground variant={currentStep}>
				{/* Container: mobile centered card, desktop card */}
				<div
					className={`w-full max-w-[420px] mx-2 sm:mx-0 sm:w-[750px] sm:max-w-[90vw] h-auto max-h-[85dvh] sm:h-auto ${
						supportsBackdrop
							? "bg-background/25 backdrop-blur-2xl border"
							: "bg-card border"
					}
						border-border/40 rounded-2xl sm:rounded-3xl shadow-xl sm:shadow-2xl overflow-hidden flex flex-col`}
					ref={containerRef}
					onTouchStart={(e) => {
						touchStartXRef.current = e.touches[0]?.clientX ?? null;
					}}
					onTouchEnd={(e) => {
						if (touchStartXRef.current == null) return;
						const endX = e.changedTouches[0]?.clientX ?? touchStartXRef.current;
						const delta = endX - touchStartXRef.current;
						touchStartXRef.current = null;
						if (Math.abs(delta) > 48) {
							if (delta < 0) handleNext();
							else handlePrevious();
						}
					}}
					style={{}}
				>
					<div
						className={
							supportsBackdrop
								? "p-4 sm:p-8 border-b border-border/30 bg-background/15 backdrop-blur-xl"
								: "p-4 sm:p-8 border-b border-border/30 bg-card"
						}
					>
						<div className="relative">
							{/* Animated border gleam */}
							<div className="pointer-events-none absolute inset-[-1px] rounded-2xl sm:rounded-3xl overflow-hidden">
								<div className="absolute -inset-[1px] opacity-60">
									<div className="absolute inset-0 bg-[conic-gradient(var(--tw-gradient-stops))] from-primary via-accent to-secondary blur-[10px]" />
								</div>
							</div>
							<div className="text-center px-6 sm:px-8">
								<h1 className="text-2xl sm:text-3xl font-bold">
									{stepData[currentStep].title}
								</h1>
								<p className="text-muted-foreground mt-2 text-base sm:text-lg">
									{stepData[currentStep].description}
								</p>
							</div>
						</div>
					</div>

					{/* Body */}
					<div
						className={
							supportsBackdrop
								? "flex-1 min-h-0 overflow-y-auto bg-background/10 backdrop-blur-lg sm:h-[450px]"
								: "flex-1 min-h-0 overflow-y-auto bg-card sm:h-[450px]"
						}
					>
						{/* Step transition wrapper */}
						<div
							key={currentStep}
							className={
								reducedMotion
									? ""
									: "transition-all duration-500 ease-out transform"
							}
						>
							<CurrentStepComponent />
						</div>
					</div>

					{/* Footer */}
					<div
						className={
							supportsBackdrop
								? "p-4 sm:p-8 bg-background/15 backdrop-blur-xl border-t border-border/30"
								: "p-4 sm:p-8 bg-card border-t border-border/30"
						}
						style={{ paddingBottom: "max(var(--fl-safe-bottom), 0px)" }}
					>
						{/* Dots are enough; progress bar removed per feedback */}

						<div className="flex justify-center gap-3 mb-4 sm:mb-8">
							{STEPS.map((step) => (
								<div
									key={step}
									className={`w-3 h-3 rounded-full transition-all duration-300 ${
										step === currentStep
											? "bg-primary scale-125 shadow-lg"
											: "bg-muted-foreground/40"
									}`}
								/>
							))}
						</div>

						{/* Mobile controls */}
						<div className="sm:hidden flex flex-col gap-3">
							<Button
								onClick={handleNext}
								className={
									supportsBackdrop
										? "w-full bg-primary/90 backdrop-blur-xs hover:bg-primary rounded-xl shadow-lg hover:shadow-xl transition-shadow"
										: "w-full bg-primary hover:bg-primary/90 rounded-xl shadow-lg hover:shadow-xl transition-shadow"
								}
							>
								{currentStep === "github" ? "Get Started" : "Next"}
							</Button>
							<div className="flex items-center justify-between pb-2">
								{currentStep !== "welcome" ? (
									<Button
										variant="ghost"
										size="sm"
										onClick={handlePrevious}
										className={
											supportsBackdrop
												? "hover:bg-background/40 backdrop-blur-xs rounded-xl"
												: "hover:bg-muted rounded-xl"
										}
									>
										Previous
									</Button>
								) : (
									<div />
								)}
								<Button
									variant="ghost"
									size="sm"
									onClick={() => handleSkip(true)}
									className={
										supportsBackdrop
											? "hover:bg-background/40 backdrop-blur-xs rounded-xl"
											: "hover:bg-muted rounded-xl"
									}
								>
									Skip Tour
								</Button>
							</div>
						</div>

						{/* Desktop controls */}
						<div className="hidden sm:flex items-center justify-between pb-4">
							<div>
								{currentStep !== "welcome" && (
									<Button
										variant="outline"
										onClick={handlePrevious}
										className={
											supportsBackdrop
												? "bg-background/40 backdrop-blur-md border-border/50 hover:bg-background/60 rounded-xl"
												: "bg-muted/60 border-border/50 hover:bg-muted rounded-xl"
										}
									>
										Previous
									</Button>
								)}
							</div>
							<div className="flex gap-4">
								<Button
									variant="ghost"
									onClick={() => handleSkip(true)}
									className={
										supportsBackdrop
											? "hover:bg-background/40 backdrop-blur-xs rounded-xl"
											: "hover:bg-muted rounded-xl"
									}
								>
									Skip Tour
								</Button>
								<Button
									onClick={handleNext}
									className={
										supportsBackdrop
											? "bg-primary/90 backdrop-blur-xs hover:bg-primary rounded-xl shadow-lg hover:shadow-xl transition-shadow"
											: "bg-primary hover:bg-primary/90 rounded-xl shadow-lg hover:shadow-xl transition-shadow"
									}
								>
									{currentStep === "github" ? "Get Started" : "Next"}
								</Button>
							</div>
						</div>
					</div>
				</div>
			</AnimatedBackground>
		</div>
	);
}
