"use client";

import { cn } from "../../lib";

interface FlowBackgroundProps {
	className?: string;
	children?: React.ReactNode;
}

function seededRandom(seed: number) {
	const x = Math.sin(seed) * 10000;
	return x - Math.floor(x);
}

export function FlowBackground({
	className,
	children,
}: Readonly<FlowBackgroundProps>) {
	return (
		<div className={cn("relative w-full h-full", className)}>
			{/* Gradient overlays */}
			<div className="absolute inset-0 bg-gradient-to-br from-background via-background to-background pointer-events-none">
				<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_top_left,rgba(59,130,246,0.08),transparent_50%)]" />
				<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_bottom_right,rgba(168,85,247,0.08),transparent_50%)]" />
				<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_center,rgba(236,72,153,0.04),transparent_60%)]" />
			</div>

			{/* Subtle grid */}
			<div
				className="absolute inset-0 pointer-events-none opacity-[0.02]"
				style={{
					backgroundImage: `linear-gradient(rgba(255,255,255,0.1) 1px, transparent 1px),
                                    linear-gradient(90deg, rgba(255,255,255,0.1) 1px, transparent 1px)`,
					backgroundSize: "80px 80px",
				}}
				aria-hidden="true"
			/>

			{/* Content - flex layout to allow proper scrolling inside children */}
			{children && (
				<div className="relative z-10 w-full h-full flex flex-col flex-1 min-h-0 overflow-auto">
					{children}
				</div>
			)}
		</div>
	);
}

export function FlowGradientOverlay({
	className,
	variant = "default",
}: Readonly<{ className?: string; variant?: "default" | "accent" | "muted" }>) {
	const gradients = {
		default: (
			<>
				<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_top_left,rgba(59,130,246,0.1),transparent_50%)]" />
				<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_bottom_right,rgba(168,85,247,0.1),transparent_50%)]" />
			</>
		),
		accent: (
			<>
				<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_top,rgba(236,72,153,0.12),transparent_60%)]" />
				<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_bottom,rgba(59,130,246,0.12),transparent_60%)]" />
			</>
		),
		muted: (
			<>
				<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_center,rgba(148,163,184,0.06),transparent_70%)]" />
			</>
		),
	};

	return (
		<div
			className={cn("absolute inset-0 pointer-events-none", className)}
			aria-hidden="true"
		>
			{gradients[variant]}
		</div>
	);
}

interface FloatingOrbsProps {
	className?: string;
	count?: number;
	colors?: string[];
}

export function FloatingOrbs({
	className,
	count = 5,
	colors = [
		"bg-blue-500/20",
		"bg-purple-500/20",
		"bg-pink-500/20",
		"bg-green-500/20",
		"bg-orange-500/20",
	],
}: FloatingOrbsProps) {
	const orbs = Array.from({ length: count }, (_, i) => {
		const size = 40 + seededRandom(i * 7) * 60;
		const x = seededRandom(i * 11) * 100;
		const y = seededRandom(i * 13) * 100;
		const duration = 15 + seededRandom(i * 17) * 10;
		const delay = seededRandom(i * 19) * -20;

		return {
			id: i,
			size,
			x,
			y,
			duration,
			delay,
			color: colors[i % colors.length],
		};
	});

	return (
		<div
			className={cn(
				"absolute inset-0 overflow-hidden pointer-events-none",
				className,
			)}
			aria-hidden="true"
		>
			{orbs.map((orb) => (
				<div
					key={orb.id}
					className={cn("absolute rounded-full blur-2xl", orb.color)}
					style={{
						width: orb.size,
						height: orb.size,
						left: `${orb.x}%`,
						top: `${orb.y}%`,
						animation: `float-orb ${orb.duration}s ease-in-out infinite`,
						animationDelay: `${orb.delay}s`,
						transform: "translate(-50%, -50%)",
					}}
				/>
			))}
		</div>
	);
}
