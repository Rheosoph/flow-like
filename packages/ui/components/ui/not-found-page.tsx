"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import { ArrowLeft, Home, RefreshCw } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { cn } from "../../lib/utils";
import { Button } from "./button";

interface FloatingOrbProps {
	delay: number;
	duration: number;
	size: number;
	color: string;
	startAngle: number;
}

function FloatingOrb({
	delay,
	duration,
	size,
	color,
	startAngle,
}: FloatingOrbProps) {
	return (
		<div
			className="absolute rounded-full blur-xl pointer-events-none animate-orbit"
			style={
				{
					width: size,
					height: size,
					background: `radial-gradient(circle, ${color} 0%, transparent 70%)`,
					animationDelay: `${delay}s`,
					animationDuration: `${duration}s`,
					"--start-angle": `${startAngle}deg`,
				} as React.CSSProperties
			}
		/>
	);
}

interface AnimatedDigitProps {
	digit: string;
	index: number;
	mouseX: number;
	mouseY: number;
}

function AnimatedDigit({ digit, index, mouseX, mouseY }: AnimatedDigitProps) {
	const offset = (index - 1) * 0.15;

	return (
		<span
			className="inline-block animate-float-digit relative"
			style={{
				animationDelay: `${index * 0.1}s`,
				transform: `
					translateY(${Math.sin(Date.now() / 1000 + index) * 3}px)
					rotateY(${mouseX * 0.5}deg)
					rotateX(${-mouseY * 0.3}deg)
				`,
				textShadow: `
					0 0 40px hsl(var(--primary) / 0.3),
					0 0 80px hsl(var(--primary) / 0.2),
					0 0 120px hsl(var(--primary) / 0.1)
				`,
			}}
		>
			<span className="relative z-10">{digit}</span>
			<span
				className="absolute inset-0 bg-clip-text text-transparent animate-gradient-text bg-size-[200%_200%]"
				style={{
					backgroundImage:
						"linear-gradient(135deg, hsl(var(--primary)) 0%, hsl(var(--foreground)) 25%, hsl(var(--primary)) 50%, hsl(var(--foreground)) 75%, hsl(var(--primary)) 100%)",
					animationDelay: `${offset}s`,
				}}
			>
				{digit}
			</span>
		</span>
	);
}

interface NotFoundPageProps {
	onGoBack?: () => void;
	onGoHome?: () => void;
	homeHref?: string;
	title?: string;
	subtitle?: string;
	showRefresh?: boolean;
}

export function NotFoundPage({
	onGoBack,
	onGoHome,
	homeHref = "/",
	title = i18next.t('pageNotFound', 'Page Not Found'),
	subtitle = i18next.t('thePageYoureLookingForDoesntExistOrHasBeenMoved', 'The page you\'re looking for doesn\'t exist or has been moved.'),
	showRefresh = true,
}: NotFoundPageProps) {
	const { t } = useTranslation("common");
	const [mounted, setMounted] = useState(false);
	const [mousePosition, setMousePosition] = useState({ x: 0, y: 0 });
	const [time, setTime] = useState(0);
	const containerRef = useRef<HTMLDivElement>(null);

	useEffect(() => {
		setMounted(true);
		const interval = setInterval(() => setTime((t) => t + 1), 50);
		return () => clearInterval(interval);
	}, []);

	useEffect(() => {
		const handleMouseMove = (e: MouseEvent) => {
			if (!containerRef.current) return;
			const rect = containerRef.current.getBoundingClientRect();
			const centerX = rect.left + rect.width / 2;
			const centerY = rect.top + rect.height / 2;
			setMousePosition({
				x: ((e.clientX - centerX) / (rect.width / 2)) * 15,
				y: ((e.clientY - centerY) / (rect.height / 2)) * 15,
			});
		};
		window.addEventListener("mousemove", handleMouseMove);
		return () => window.removeEventListener("mousemove", handleMouseMove);
	}, []);

	const handleGoBack = () => {
		if (onGoBack) {
			onGoBack();
		} else {
			window.history.back();
		}
	};

	const handleGoHome = () => {
		if (onGoHome) {
			onGoHome();
		} else if (typeof window !== "undefined") {
			window.location.href = homeHref;
		}
	};

	const orbs = [
		{
			delay: 0,
			duration: 20,
			size: 300,
			color: `hsl(var(--primary) / 0.15)`,
			startAngle: 0,
		},
		{
			delay: 5,
			duration: 25,
			size: 200,
			color: `hsl(var(--secondary) / 0.1)`,
			startAngle: 120,
		},
		{
			delay: 10,
			duration: 30,
			size: 250,
			color: `hsl(var(--primary) / 0.1)`,
			startAngle: 240,
		},
	];

	const digits = "404".split("");

	return (
		<main
			ref={containerRef}
			className="relative w-full h-full min-h-screen flex flex-col items-center justify-center overflow-hidden bg-background"
		>
			{/* Animated mesh gradient background */}
			<div className="absolute inset-0 overflow-hidden">
				<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_top,hsl(var(--primary)/0.1),transparent_50%)]" />
				<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_bottom_right,hsl(var(--secondary)/0.08),transparent_50%)]" />
				<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_bottom_left,hsl(var(--primary)/0.05),transparent_50%)]" />
			</div>

			{/* Animated grid with perspective */}
			<div
				className="absolute inset-0 opacity-[0.03] dark:opacity-[0.06]"
				style={{
					backgroundImage: `
						linear-gradient(to right, currentColor 1px, transparent 1px),
						linear-gradient(to bottom, currentColor 1px, transparent 1px)
					`,
					backgroundSize: "80px 80px",
					transform: `perspective(500px) rotateX(60deg) translateY(-50%)`,
					transformOrigin: "center top",
				}}
			/>

			{/* Floating orbs */}
			<div className="absolute inset-0 flex items-center justify-center pointer-events-none">
				{mounted && orbs.map((orb, i) => <FloatingOrb key={i} {...orb} />)}
			</div>

			{/* Orbiting rings */}
			<div className="absolute inset-0 flex items-center justify-center pointer-events-none">
				<div
					className="absolute w-125 h-125 rounded-full border border-primary/10 animate-spin-slow"
					style={{ animationDuration: "30s" }}
				/>
				<div
					className="absolute w-150 h-150 rounded-full border border-primary/5 animate-spin-slow"
					style={{ animationDuration: "40s", animationDirection: "reverse" }}
				/>
				<div
					className="absolute w-100 h-100 rounded-full border-dashed border border-primary/10 animate-spin-slow"
					style={{ animationDuration: "25s" }}
				/>
			</div>

			{/* Main content with 3D perspective */}
			<div
				className={cn(
					"relative z-10 flex flex-col items-center text-center px-6 transition-all duration-500 ease-out",
					mounted ? "opacity-100 translate-y-0" : "opacity-0 translate-y-12",
				)}
				style={{
					transform: mounted
						? `perspective(1000px) rotateY(${mousePosition.x * 0.3}deg) rotateX(${-mousePosition.y * 0.3}deg) translateZ(50px)`
						: undefined,
					transformStyle: "preserve-3d",
				}}
			>
				{/* 404 Number with individual animated digits */}
				<div
					className="relative mb-8 select-none"
					style={{ transformStyle: "preserve-3d" }}
				>
					<div className="text-[10rem] sm:text-[14rem] md:text-[18rem] font-black leading-none tracking-tighter flex">
						{digits.map((digit, i) => (
							<AnimatedDigit
								key={i}
								digit={digit}
								index={i}
								mouseX={mousePosition.x}
								mouseY={mousePosition.y}
							/>
						))}
					</div>

					{/* Glow effect behind text */}
					<div
						className="absolute inset-0 blur-3xl opacity-30 -z-10"
						style={{
							background:
								"radial-gradient(circle, hsl(var(--primary)) 0%, transparent 70%)",
							transform: `scale(1.2) translateZ(-50px)`,
						}}
					/>

					{/* Floating dots decoration */}
					<div className="absolute -top-8 -right-8 w-3 h-3 rounded-full bg-primary/60 animate-bounce-slow" />
					<div
						className="absolute top-1/4 -left-12 w-2 h-2 rounded-full bg-primary/40 animate-bounce-slow"
						style={{ animationDelay: "0.5s" }}
					/>
					<div
						className="absolute -bottom-4 right-1/4 w-4 h-4 rounded-full bg-primary/30 animate-bounce-slow"
						style={{ animationDelay: "1s" }}
					/>
				</div>

				{/* Error message with fade-in effect */}
				<div
					className={cn(
						"space-y-3 mb-10 transition-all duration-700 delay-300",
						mounted ? "opacity-100 translate-y-0" : "opacity-0 translate-y-4",
					)}
				>
					<h1 className="text-2xl sm:text-3xl md:text-4xl font-bold text-foreground">
						{title}
					</h1>
					<p className="text-muted-foreground text-base sm:text-lg max-w-md mx-auto">
						{subtitle}
					</p>
				</div>

				{/* Action buttons with staggered animation */}
				<div
					className={cn(
						"flex flex-col sm:flex-row gap-3 transition-all duration-700 delay-500",
						mounted ? "opacity-100 translate-y-0" : "opacity-0 translate-y-4",
					)}
				>
					<Button
						variant="default"
						size="lg"
						onClick={handleGoBack}
						className="group min-w-40 gap-2 relative overflow-hidden"
					>
						<span className="absolute inset-0 bg-white/10 -translate-x-full group-hover:translate-x-full transition-transform duration-500" />
						<ArrowLeft className="w-4 h-4 transition-transform group-hover:-translate-x-1" />
						{t('goBack', 'Go Back')}
					</Button>

					<Button
						variant="outline"
						size="lg"
						onClick={handleGoHome}
						className="group min-w-40 gap-2 relative overflow-hidden"
					>
						<span className="absolute inset-0 bg-primary/5 -translate-x-full group-hover:translate-x-full transition-transform duration-500" />
						<Home className="w-4 h-4 transition-transform group-hover:scale-110" />
						{t('home', 'Home')}
					</Button>

					{showRefresh && (
						<Button
							variant="ghost"
							size="lg"
							onClick={() => window.location.reload()}
							className="group min-w-40 gap-2"
						>
							<RefreshCw className="w-4 h-4 transition-transform group-hover:rotate-180 duration-500" />
							{t('refresh', 'Refresh')}
						</Button>
					)}
				</div>

				{/* Fun message */}
				<p
					className={cn(
						"mt-12 text-xs text-muted-foreground/60 max-w-xs transition-all duration-700 delay-700",
						mounted ? "opacity-100" : "opacity-0",
					)}
				>
					{t('lostInTheFlowDontWorryEvenTheBestWorkflowsTakeUnexpectedTurnsSometimes', "Lost in the flow? Don't worry, even the best workflows take unexpected turns sometimes.")}
				</p>
			</div>

			{/* CSS for animations */}
			<style>{`@keyframes orbit { 0% { transform: rotate(var(--start-angle, 0deg)) translateX(200px) rotate(calc(-1 * var(--start-angle, 0deg))); } 100% { transform: rotate(calc(var(--start-angle, 0deg) + 360deg)) translateX(200px) rotate(calc(-1 * (var(--start-angle, 0deg) + 360deg))); } } @keyframes float-digit { 0%, 100% { transform: translateY(0); } 50% { transform: translateY(-8px); } } @keyframes gradient-text { 0% { background-position: 0% 50%; } 50% { background-position: 100% 50%; } 100% { background-position: 0% 50%; } } @keyframes bounce-slow { 0%, 100% { transform: translateY(0) scale(1); } 50% { transform: translateY(-10px) scale(1.1); } } @keyframes spin-slow { from { transform: rotate(0deg); } to { transform: rotate(360deg); } } .animate-orbit { animation: orbit linear infinite; } .animate-float-digit { animation: float-digit 3s ease-in-out infinite; } .animate-gradient-text { animation: gradient-text 4s ease-in-out infinite; } .animate-bounce-slow { animation: bounce-slow 2s ease-in-out infinite; } .animate-spin-slow { animation: spin-slow linear infinite; }`}</style>
		</main>
	);
}
