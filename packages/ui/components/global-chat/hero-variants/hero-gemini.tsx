"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	ChevronDownIcon,
	LayoutGridIcon,
	PackageIcon,
	PaperclipIcon,
	PlusIcon,
	SendIcon,
	SparklesIcon,
	UserRoundIcon,
} from "lucide-react";
import { type KeyboardEvent, useCallback } from "react";
import { Textarea } from "../../../index";
import { HeroFileChips, HeroFileInput } from "./hero-file-chips";
import { HERO_SUGGESTIONS, useHeroComposer } from "./use-hero-composer";

const SUGGESTION_META = [
	{
		icon: PlusIcon,
		iconClass: "text-[#d97706] dark:text-[#ffb35c]",
		hot: true,
	},
	{
		icon: SparklesIcon,
		iconClass: "text-[#8b5cf6] dark:text-[#b49aff]",
		hot: false,
	},
	{
		icon: PackageIcon,
		iconClass: "text-[#d97706] dark:text-[#ffb35c]",
		hot: false,
	},
	{
		icon: UserRoundIcon,
		iconClass: "text-[#4f7dff] dark:text-[#7da2ff]",
		hot: false,
	},
] as const;

const SUGGESTION_ITEMS = HERO_SUGGESTIONS.map((label, index) => ({
	label,
	...SUGGESTION_META[index % SUGGESTION_META.length],
}));

const HERO_V3_CSS = `
@property --hero-v3-sweep {
	syntax: "<angle>";
	initial-value: 0deg;
	inherits: false;
}
.hero-v3-outer {
	position: relative;
	width: min(900px, 100%);
}
.hero-v3-outer::before,
.hero-v3-outer::after {
	content: "";
	position: absolute;
	border-radius: 50%;
	filter: blur(70px);
	pointer-events: none;
}
.hero-v3-outer::before {
	width: 56%;
	height: 100px;
	right: 3%;
	top: -32px;
	background: rgba(255, 168, 88, 0.14);
}
.hero-v3-outer::after {
	width: 55%;
	height: 150px;
	left: -3%;
	bottom: -56px;
	background: rgba(139, 92, 246, 0.3);
}
/* blurred bloom ring hugging the border — sibling BEHIND the bar: a negative
   z-index pseudo would paint above the bar's own background and flood the interior */
.hero-v3-bloom {
	position: absolute;
	inset: -4px;
	border-radius: 999px;
	background: linear-gradient(180deg,
		rgba(255, 186, 100, 0.8),
		rgba(255, 158, 92, 0.3) 30%,
		rgba(120, 96, 170, 0.3) 60%,
		rgba(139, 92, 246, 0.7) 100%);
	filter: blur(20px);
	opacity: 0.45;
	z-index: 0;
	pointer-events: none;
}
/* colored halos hugging the two orbs, behind the bar */
.hero-v3-bloom::before,
.hero-v3-bloom::after {
	content: "";
	position: absolute;
	top: 50%;
	translate: 0 -50%;
	border-radius: 50%;
	filter: blur(8px);
}
.hero-v3-bloom::before {
	left: -36px;
	width: 230px;
	height: 230px;
	background: radial-gradient(circle, rgba(139, 92, 246, 0.55), transparent 66%);
}
.hero-v3-bloom::after {
	right: -44px;
	width: 250px;
	height: 240px;
	background: radial-gradient(circle, rgba(240, 68, 124, 0.4), transparent 64%);
}
.hero-v3-bar {
	--hero-v3-ink: rgba(0, 0, 0, 0.25);
	position: relative;
	z-index: 1;
	display: flex;
	align-items: center;
	gap: 22px;
	padding: 24px 24px 24px 26px;
	border-radius: 999px;
	border: 2px solid transparent;
	background:
		linear-gradient(180deg, var(--card), var(--background) 45%, var(--background)) padding-box,
		conic-gradient(from var(--hero-v3-sweep),
			transparent 0 68%,
			rgba(255, 226, 176, 0.9) 76%,
			transparent 84%) border-box,
		linear-gradient(180deg,
			rgba(255, 196, 120, 0.95),
			rgba(255, 170, 110, 0.55) 22%,
			rgba(170, 130, 230, 0.5) 55%,
			rgba(150, 100, 250, 0.95) 100%) border-box;
	box-shadow:
		0 -12px 50px -20px rgba(255, 170, 90, 0.4),
		0 36px 80px -24px var(--hero-v3-ink),
		0 30px 100px -30px rgba(139, 92, 246, 0.5),
		0 14px 60px -20px rgba(240, 68, 124, 0.3),
		inset 0 2px 3px -1px rgba(255, 255, 255, 0.2),
		inset 0 18px 40px -30px rgba(255, 190, 120, 0.12),
		inset 0 -16px 44px -26px rgba(120, 90, 220, 0.28);
	transition: box-shadow 0.4s;
}
.dark .hero-v3-bar {
	--hero-v3-ink: rgba(0, 0, 0, 0.9);
	background:
		linear-gradient(180deg, #1d1f2c, #14161f 45%, #10121a) padding-box,
		conic-gradient(from var(--hero-v3-sweep),
			transparent 0 68%,
			rgba(255, 226, 176, 0.9) 76%,
			transparent 84%) border-box,
		linear-gradient(180deg,
			rgba(255, 196, 120, 0.95),
			rgba(255, 170, 110, 0.55) 22%,
			rgba(170, 130, 230, 0.5) 55%,
			rgba(150, 100, 250, 0.95) 100%) border-box;
}
.hero-v3-bar:focus-within {
	box-shadow:
		0 -14px 60px -18px rgba(255, 170, 90, 0.55),
		0 36px 80px -24px var(--hero-v3-ink),
		0 34px 110px -28px rgba(139, 92, 246, 0.62),
		0 16px 70px -18px rgba(240, 68, 124, 0.4),
		inset 0 2px 3px -1px rgba(255, 255, 255, 0.24),
		inset 0 18px 40px -30px rgba(255, 190, 120, 0.16),
		inset 0 -16px 44px -26px rgba(120, 90, 220, 0.34);
}
.hero-v3-bar button:focus-visible,
.hero-v3-suggestion:focus-visible {
	outline: 2px solid rgba(244, 85, 58, 0.6);
	outline-offset: 2px;
}
.hero-v3-star {
	position: relative;
	flex: none;
	width: 68px;
	height: 68px;
	border-radius: 50%;
	display: grid;
	place-items: center;
	background: radial-gradient(circle at 35% 30%, #453a70, #1e1a33 75%);
	border: 1px solid rgba(215, 190, 255, 0.55);
	box-shadow:
		0 0 0 5px rgba(139, 92, 246, 0.12),
		0 0 44px -2px rgba(139, 92, 246, 0.8),
		inset 0 1px 0 rgba(255, 255, 255, 0.16);
	color: #f4f0ff;
}
.hero-v3-star::before {
	content: "";
	position: absolute;
	inset: -22px;
	border-radius: 50%;
	background: radial-gradient(circle, rgba(139, 92, 246, 0.6), transparent 70%);
	pointer-events: none;
}
.hero-v3-star svg {
	position: relative;
}
.hero-v3-main {
	flex: 1;
	min-width: 0;
	display: flex;
	flex-direction: column;
	gap: 14px;
}
.hero-v3-chips {
	display: flex;
	align-items: center;
	gap: 10px;
}
.hero-v3-chip {
	display: inline-flex;
	align-items: center;
	gap: 8px;
	height: 40px;
	padding: 0 17px;
	border-radius: 999px;
	font-size: 13.5px;
	font-weight: 600;
	color: var(--muted-foreground);
	background: color-mix(in oklab, var(--foreground) 5%, transparent);
	border: 1px solid var(--border);
	transition: border-color 0.25s, background 0.25s, box-shadow 0.25s, translate 0.25s;
}
button.hero-v3-chip {
	cursor: pointer;
}
.hero-v3-chip:hover {
	translate: 0 -1px;
	border-color: rgba(255, 170, 84, 0.5);
	background: rgba(255, 170, 84, 0.07);
	box-shadow: 0 6px 20px -10px rgba(255, 158, 74, 0.5);
}
.hero-v3-chip svg {
	color: var(--muted-foreground);
	transition: color 0.25s;
}
.hero-v3-chip:hover svg {
	color: #d97706;
}
.dark .hero-v3-chip:hover svg {
	color: #ffb35c;
}
.hero-v3-send {
	flex: none;
	width: 62px;
	height: 62px;
	border-radius: 50%;
	display: grid;
	place-items: center;
	color: #fff;
	background: linear-gradient(140deg, #ff7d9c, #f0447c 60%, #d92d6f);
	box-shadow: 0 12px 40px -8px rgba(240, 68, 124, 0.85), 0 0 30px -6px rgba(240, 68, 124, 0.5), inset 0 1px 0 rgba(255, 255, 255, 0.32);
	cursor: pointer;
	transition: scale 0.2s cubic-bezier(0.34, 1.56, 0.64, 1), box-shadow 0.25s, opacity 0.25s;
}
.hero-v3-send svg {
	transition: translate 0.25s;
}
.hero-v3-send:not(:disabled):hover {
	scale: 1.07;
	box-shadow: 0 16px 48px -8px rgba(240, 68, 124, 1), 0 0 38px -4px rgba(240, 68, 124, 0.6), inset 0 1px 0 rgba(255, 255, 255, 0.32);
}
.hero-v3-send:not(:disabled):hover svg {
	translate: 1px -1px;
}
.hero-v3-send:not(:disabled):active {
	scale: 0.95;
}
.hero-v3-send:disabled {
	opacity: 0.82;
	cursor: default;
}
.hero-v3-suggestion {
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
	cursor: pointer;
	transition: translate 0.25s cubic-bezier(0.22, 1, 0.36, 1), border-color 0.25s, background 0.25s, box-shadow 0.25s, color 0.25s;
}
.hero-v3-suggestion:hover {
	translate: 0 -2px;
	border-color: rgba(244, 85, 58, 0.45);
	background: rgba(244, 85, 58, 0.08);
	box-shadow: 0 8px 24px -10px rgba(244, 85, 58, 0.35);
	color: var(--foreground);
}
.hero-v3-suggestion-hot {
	border-color: rgba(255, 170, 84, 0.55);
	background: rgba(255, 170, 84, 0.08);
	color: #b45309;
}
.dark .hero-v3-suggestion-hot {
	color: #ffd9ae;
}
@media (max-width: 760px) {
	.hero-v3-bar {
		border-radius: 40px;
		flex-wrap: wrap;
	}
	.hero-v3-chips {
		flex-wrap: wrap;
	}
}
@media (prefers-reduced-motion: no-preference) {
	.hero-v3-outer::before {
		animation: hero-v3-breathe 7s ease-in-out infinite;
	}
	.hero-v3-outer::after {
		animation: hero-v3-breathe 9s ease-in-out 1.5s infinite;
	}
	.hero-v3-bar {
		animation: hero-v3-sweep 11s linear infinite;
	}
	.hero-v3-bloom {
		animation: hero-v3-breathe 8s ease-in-out infinite;
	}
	.hero-v3-star::before {
		animation: hero-v3-breathe 4.5s ease-in-out infinite;
	}
	.hero-v3-star svg {
		animation: hero-v3-starspin 9s ease-in-out infinite;
	}
	@keyframes hero-v3-sweep {
		to { --hero-v3-sweep: 360deg; }
	}
	@keyframes hero-v3-breathe {
		0%, 100% { opacity: 0.55; scale: 1; }
		50% { opacity: 1; scale: 1.12; }
	}
	@keyframes hero-v3-starspin {
		0%, 100% { rotate: 0deg; scale: 1; }
		50% { rotate: 180deg; scale: 1.08; }
	}
}
`;

export function HeroSearchBarGemini() {
	const { t } = useTranslation("chat");
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

	const handleKeyDown = useCallback(
		(e: KeyboardEvent<HTMLTextAreaElement>) => {
			if (e.key === "Enter" && !e.shiftKey) {
				e.preventDefault();
				submit(value);
			}
		},
		[submit, value],
	);

	const handleSend = useCallback(() => {
		submit(value);
	}, [submit, value]);

	return (
		<div className="w-full flex flex-col items-center gap-5 px-4 pt-14 pb-8 shrink-0">
			<style>{HERO_V3_CSS}</style>
			<div className="flex flex-col items-center gap-2 text-center">
				<h1 className="text-3xl md:text-4xl font-bold tracking-tight">
					{t('whatDoYouWantTo', 'What do you want to')}{" "}<Trans i18nKey="spanClassnamebglinear92degff6d3dc084fc_55f472b6BgcliptextTexttransparentBuildSpan"><span className="bg-linear-[92deg,#ff6d3d,#c084fc_55%,#f472b6] bg-clip-text text-transparent">
						build
					</span>
					?</Trans></h1>
				<p className="text-sm md:text-base text-muted-foreground">
					{t('askFlowpilotToCreateAppsFindPackagesOrNavigateFlowlike', 'Ask FlowPilot to create apps, find packages, or navigate Flow-Like.')}
				</p>
			</div>
			<div className="hero-v3-outer">
				<div className="hero-v3-bloom" aria-hidden="true" />
				<div className="hero-v3-bar">
					<span className="hero-v3-star" aria-hidden="true">
						{/* Four-point star glyph kept inline to match the reference exactly. */}
						<svg
							width="30"
							height="30"
							viewBox="0 0 24 24"
							fill="currentColor"
							aria-hidden="true"
						>
							<path d="M12 2c.7 4.2 3.1 6.6 7.3 7.3.9.15.9 1.25 0 1.4-4.2.7-6.6 3.1-7.3 7.3-.15.9-1.25.9-1.4 0-.7-4.2-3.1-6.6-7.3-7.3-.9-.15-.9-1.25 0-1.4 4.2-.7 6.6-3.1 7.3-7.3.15-.9 1.25-.9 1.4 0Z" />
						</svg>
					</span>
					<div className="hero-v3-main">
						<HeroFileChips files={files} onRemove={removeFile} />
						<Textarea
							value={value}
							onChange={(e) => setValue(e.target.value)}
							onKeyDown={handleKeyDown}
							placeholder={t('askFlowpilotAnythingOrDescribeWhatYouWantToBuild', 'Ask FlowPilot anything, or describe what you want to build…')}
							rows={1}
							aria-label={t('askFlowpilot', 'Ask FlowPilot')}
							className="min-h-9 max-h-40 resize-none border-0 bg-transparent dark:bg-transparent shadow-none focus-visible:ring-0 py-1.5 px-2 text-[17px]"
						/>
						<div className="hero-v3-chips">
							<span className="hero-v3-chip" aria-hidden="true">
								<SparklesIcon className="size-3.5 text-[#7c3aed] dark:text-[#c9b8ff]" />
								{t('smart', 'Smart')}
								<ChevronDownIcon className="size-3.25" />
							</span>
							<button
								type="button"
								className="hero-v3-chip"
								onClick={openFilePicker}
								aria-label={t('attachImages', 'Attach images')}
							>
								<PaperclipIcon className="size-3.5" />
								{t('attach', 'Attach')}
							</button>
							<span className="hero-v3-chip" aria-hidden="true">
								<LayoutGridIcon className="size-3.5" />
								{t('templates', 'Templates')}
							</span>
						</div>
					</div>
					<HeroFileInput inputRef={fileInputRef} onAdd={addFiles} />
					<button
						type="button"
						className="hero-v3-send"
						onClick={handleSend}
						disabled={!canSend}
						aria-label={t('send', 'Send')}
					>
						<SendIcon
							className="size-5.5"
							strokeWidth={1.9}
							aria-hidden="true"
						/>
					</button>
				</div>
			</div>
			<div className="flex flex-wrap items-center justify-center gap-2.5 max-w-2xl">
				{SUGGESTION_ITEMS.map(({ label, icon: Icon, iconClass, hot }) => (
					<button
						key={label}
						type="button"
						className={
							hot
								? "hero-v3-suggestion hero-v3-suggestion-hot"
								: "hero-v3-suggestion"
						}
						onClick={() => submit(label)}
					>
						<Icon className={`size-3.5 ${iconClass}`} />
						{label}
					</button>
				))}
			</div>
		</div>
	);
}
