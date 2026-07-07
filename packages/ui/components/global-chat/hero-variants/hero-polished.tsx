"use client";

import {
	ArrowUpIcon,
	PackageIcon,
	PaperclipIcon,
	PlusIcon,
	SparklesIcon,
	UserRoundIcon,
} from "lucide-react";
import { Textarea } from "../../../index";
import { HeroFileChips, HeroFileInput } from "./hero-file-chips";
import { HERO_SUGGESTIONS, useHeroComposer } from "./use-hero-composer";

const SUGGESTION_ICONS = [
	PlusIcon,
	SparklesIcon,
	PackageIcon,
	UserRoundIcon,
] as const;

const SUGGESTIONS = HERO_SUGGESTIONS.map((label, index) => ({
	label,
	Icon: SUGGESTION_ICONS[index] ?? SparklesIcon,
}));

const FOCUS_RING =
	"outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0";

export function HeroSearchBarPolished() {
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

	return (
		<div className="w-full flex flex-col items-center gap-5 px-4 pt-14 pb-8 shrink-0">
			<style>{`
				@property --hero-v1-sweep {
					syntax: "<angle>";
					initial-value: 215deg;
					inherits: false;
				}
				.hero-v1-halo::before,
				.hero-v1-halo::after {
					content: "";
					position: absolute;
					border-radius: 50%;
					filter: blur(60px);
					pointer-events: none;
					z-index: 0;
				}
				.hero-v1-halo::before {
					width: 240px;
					height: 180px;
					left: -60px;
					top: -60px;
					background: rgba(244, 85, 58, 0.22);
				}
				.hero-v1-halo::after {
					width: 260px;
					height: 180px;
					right: -60px;
					bottom: -60px;
					background: rgba(139, 92, 246, 0.2);
				}
				.hero-v1-bar {
					padding: 15px 15px 15px 16px;
					border-radius: 32px;
					border: 1.5px solid transparent;
					background:
						linear-gradient(180deg, var(--card), var(--background)) padding-box,
						conic-gradient(from var(--hero-v1-sweep),
							rgba(244, 85, 58, 0.95),
							rgba(244, 85, 58, 0.18) 16%,
							rgba(79, 125, 255, 0.22) 38%,
							rgba(139, 92, 246, 0.6) 55%,
							rgba(232, 121, 249, 0.45) 72%,
							rgba(244, 85, 58, 0.5) 88%,
							rgba(244, 85, 58, 0.95)) border-box;
					box-shadow:
						0 30px 80px -30px rgba(244, 85, 58, 0.35),
						0 18px 60px -24px rgba(139, 92, 246, 0.3),
						inset 0 1px 0 rgba(255, 255, 255, 0.05);
					transition: box-shadow 0.4s ease;
				}
				.hero-v1-bar:focus-within {
					box-shadow:
						0 34px 90px -28px rgba(244, 85, 58, 0.5),
						0 20px 70px -22px rgba(139, 92, 246, 0.42),
						inset 0 1px 0 rgba(255, 255, 255, 0.07);
				}
				/* shimmer confined to the border ring via double-mask exclude */
				.hero-v1-bar::after {
					content: "";
					position: absolute;
					inset: -1.5px;
					border-radius: inherit;
					padding: 1.5px;
					background: linear-gradient(115deg, transparent 38%, rgba(255, 255, 255, 0.5) 50%, transparent 62%) no-repeat;
					background-size: 300% 100%;
					background-position: 130% 0;
					opacity: 0;
					-webkit-mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
					-webkit-mask-composite: xor;
					mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
					mask-composite: exclude;
					pointer-events: none;
				}
				.hero-v1-spark {
					flex: none;
					width: 54px;
					height: 54px;
					border-radius: 50%;
					display: grid;
					place-items: center;
					color: var(--foreground);
					background: linear-gradient(180deg, color-mix(in oklab, var(--card), var(--foreground) 5%), var(--card));
					border: 1px solid var(--border);
					box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.06), 0 6px 18px -8px rgba(0, 0, 0, 0.5);
					transition: border-color 0.3s, box-shadow 0.3s;
				}
				.hero-v1-spark:hover {
					border-color: rgba(139, 92, 246, 0.5);
					box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.06), 0 0 24px -6px rgba(139, 92, 246, 0.6);
				}
				.hero-v1-group {
					flex: none;
					display: flex;
					align-items: center;
					height: 46px;
					border-radius: 999px;
					border: 1px solid var(--border);
					background: color-mix(in oklab, var(--foreground) 3%, transparent);
					overflow: hidden;
				}
				.hero-v1-group-item {
					height: 100%;
					padding: 0 15px;
					display: grid;
					place-items: center;
					color: var(--muted-foreground);
					font-size: 15px;
					font-weight: 600;
					transition: color 0.2s, background 0.2s;
				}
				button.hero-v1-group-item {
					cursor: pointer;
				}
				button.hero-v1-group-item:hover {
					color: var(--foreground);
					background: color-mix(in oklab, var(--foreground) 6%, transparent);
				}
				.hero-v1-sep {
					flex: none;
					width: 1px;
					height: 22px;
					background: var(--border);
				}
				.hero-v1-send {
					flex: none;
					width: 54px;
					height: 54px;
					border-radius: 50%;
					display: grid;
					place-items: center;
					color: #fff;
					background: linear-gradient(140deg, #ff8a4d, #f4553a 55%, #e13d63);
					box-shadow: 0 10px 28px -8px rgba(244, 85, 58, 0.65), inset 0 1px 0 rgba(255, 255, 255, 0.3);
					transition: scale 0.2s cubic-bezier(0.34, 1.56, 0.64, 1), box-shadow 0.25s, filter 0.25s;
					cursor: pointer;
				}
				.hero-v1-send:not(:disabled):hover {
					scale: 1.07;
					box-shadow: 0 14px 36px -8px rgba(244, 85, 58, 0.8), inset 0 1px 0 rgba(255, 255, 255, 0.3);
				}
				.hero-v1-send:not(:disabled):active {
					scale: 0.96;
				}
				.hero-v1-send:disabled {
					filter: saturate(0.7) brightness(0.88);
					cursor: default;
				}
				.hero-v1-chip {
					display: inline-flex;
					align-items: center;
					gap: 8px;
					height: 38px;
					padding: 0 16px;
					border-radius: 999px;
					font-size: 13px;
					font-weight: 500;
					color: var(--muted-foreground);
					background: color-mix(in oklab, var(--foreground) 3%, transparent);
					border: 1px solid var(--border);
					transition: translate 0.25s cubic-bezier(0.22, 1, 0.36, 1), border-color 0.25s, background 0.25s, box-shadow 0.25s, color 0.25s;
					cursor: pointer;
				}
				.hero-v1-chip:hover {
					translate: 0 -2px;
					border-color: rgba(244, 85, 58, 0.45);
					background: rgba(244, 85, 58, 0.08);
					box-shadow: 0 8px 24px -10px rgba(244, 85, 58, 0.35);
					color: var(--foreground);
				}
				.hero-v1-chip svg {
					color: var(--muted-foreground);
					transition: color 0.25s;
				}
				.hero-v1-chip:hover svg {
					color: #ff6d3d;
				}
				@media (prefers-reduced-motion: no-preference) {
					.hero-v1-halo::before {
						animation: hero-v1-breathe 6s ease-in-out infinite;
					}
					.hero-v1-halo::after {
						animation: hero-v1-breathe 8s ease-in-out 1.2s infinite;
					}
					.hero-v1-bar {
						animation: hero-v1-sweep 16s linear infinite;
					}
					.hero-v1-bar::after {
						animation: hero-v1-shimmer 7s ease-in-out infinite;
					}
					.hero-v1-spark svg {
						animation: hero-v1-twinkle 3.4s ease-in-out infinite;
					}
					@keyframes hero-v1-breathe {
						0%, 100% { opacity: 0.55; scale: 1; }
						50% { opacity: 1; scale: 1.12; }
					}
					@keyframes hero-v1-sweep {
						to { --hero-v1-sweep: 575deg; }
					}
					@keyframes hero-v1-shimmer {
						0%, 55% { background-position: 130% 0; opacity: 0; }
						70% { opacity: 1; }
						85%, 100% { background-position: -30% 0; opacity: 0; }
					}
					@keyframes hero-v1-twinkle {
						0%, 100% { scale: 1; opacity: 0.9; rotate: 0deg; }
						50% { scale: 1.12; opacity: 1; rotate: 8deg; }
					}
				}
				@media (max-width: 760px) {
					.hero-v1-bar { border-radius: 26px; }
					.hero-v1-row { flex-wrap: wrap; }
					.hero-v1-row .hero-v1-input { order: -1; flex: 1 1 100%; }
				}
			`}</style>
			<div className="flex flex-col items-center gap-2 text-center">
				<h1 className="text-3xl md:text-4xl font-bold tracking-tight text-foreground">
					What do you want to{" "}
					<span className="bg-[linear-gradient(92deg,#ff6d3d,#c084fc_55%,#f472b6)] bg-clip-text text-transparent">
						build
					</span>
					?
				</h1>
				<p className="text-sm md:text-base text-muted-foreground">
					Ask FlowPilot to create apps, find packages, or navigate Flow-Like.
				</p>
			</div>
			<div className="hero-v1-halo relative w-full max-w-[880px]">
				<div className="hero-v1-bar relative z-[1] flex w-full flex-col">
					<HeroFileChips files={files} onRemove={removeFile} />
					<div className="hero-v1-row flex w-full items-center gap-3.5">
						<span className="hero-v1-spark" aria-hidden="true">
							<SparklesIcon className="size-5.5" strokeWidth={1.8} />
						</span>
						<Textarea
							value={value}
							onChange={(e) => setValue(e.target.value)}
							onKeyDown={(e) => {
								if (e.key === "Enter" && !e.shiftKey) {
									e.preventDefault();
									submit(value);
								}
							}}
							placeholder="Ask FlowPilot anything, or describe what you want to build…"
							rows={1}
							className="hero-v1-input flex-1 min-w-0 min-h-9 max-h-40 resize-none border-0 bg-transparent dark:bg-transparent shadow-none focus-visible:ring-0 py-1.5 px-2 text-[16.5px]"
						/>
						<HeroFileInput inputRef={fileInputRef} onAdd={addFiles} />
						<div className="hero-v1-group">
							<button
								type="button"
								aria-label="Attach images"
								className={`hero-v1-group-item ${FOCUS_RING}`}
								onClick={openFilePicker}
							>
								<PaperclipIcon
									className="size-[17px]"
									strokeWidth={1.8}
									aria-hidden="true"
								/>
							</button>
							<span className="hero-v1-sep" aria-hidden="true" />
							<span className="hero-v1-group-item" aria-hidden="true">
								/
							</span>
						</div>
						<button
							type="button"
							aria-label="Send"
							disabled={!canSend}
							className={`hero-v1-send ${FOCUS_RING}`}
							onClick={() => submit(value)}
						>
							<ArrowUpIcon
								className="size-[21px]"
								strokeWidth={2.2}
								aria-hidden="true"
							/>
						</button>
					</div>
				</div>
			</div>
			<div className="flex flex-wrap items-center justify-center gap-2.5 max-w-[880px]">
				{SUGGESTIONS.map(({ label, Icon }) => (
					<button
						key={label}
						type="button"
						className={`hero-v1-chip ${FOCUS_RING}`}
						onClick={() => submit(label)}
					>
						<Icon className="size-3.5" strokeWidth={1.8} aria-hidden="true" />
						{label}
					</button>
				))}
			</div>
		</div>
	);
}
