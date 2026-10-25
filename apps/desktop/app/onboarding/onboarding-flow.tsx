"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Check,
	Lightbulb,
	MessageCircle,
	Sparkles,
	Workflow,
} from "lucide-react";
import styles from "./onboarding-flow.module.css";

const connections = [
	{ name: "inputConnection", path: "M145 135C181 135 183 135 218 135" },
	{ name: "chatConnection", path: "M322 135C356 135 343 60 377 60" },
	{ name: "createConnection", path: "M322 135C347 135 352 138 377 138" },
	{ name: "automateConnection", path: "M322 135C356 135 343 216 377 216" },
] as const;

export function OnboardingFlow() {
	const { t } = useTranslation("common");

	return (
		<div className={styles.canvas} aria-hidden="true">
			<div className={styles.grid} />
			<div className={styles.glow} />
			<svg
				className={styles.connections}
				viewBox="0 0 520 270"
				fill="none"
				focusable="false"
				aria-hidden="true"
			>
				<g className={styles.orbits}>
					<circle cx="270" cy="135" r="83" />
					<circle cx="270" cy="135" r="111" />
					<ellipse
						cx="270"
						cy="135"
						rx="164"
						ry="48"
						transform="rotate(-27 270 135)"
					/>
				</g>
				<g className={styles.stars}>
					<circle cx="186" cy="61" r="2" />
					<circle cx="312" cy="32" r="2.5" />
					<circle cx="203" cy="222" r="2" />
					<path d="M345 82V90M341 86H349M149 194V200M146 197H152" />
				</g>
				{connections.map(({ name, path }) => (
					<g key={name} className={styles[name]}>
						<path className={styles.track} d={path} />
						<path className={styles.connection} d={path} pathLength="1" />
						<path className={styles.pulse} d={path} pathLength="1" />
					</g>
				))}
			</svg>

			<div className={`${styles.node} ${styles.idea}`}>
				<div className={styles.nodeHeader}>
					<span className={styles.icon}>
						<Lightbulb />
					</span>
					<span>{t("onboarding.yourIdea", "Your idea")}</span>
				</div>
				<div className={styles.ideaPreview}>
					<span />
					<span />
					<span />
				</div>
				<span className={`${styles.port} ${styles.outputPort}`} />
			</div>

			<div className={styles.brand}>
				<div className={styles.brandAura} />
				<div className={styles.brandMark}>
					<span className={`${styles.port} ${styles.inputPort}`} />
					<img
						src="/base-logo.svg"
						width={104}
						height={104}
						alt=""
						className={styles.brandLogo}
					/>
					<span className={`${styles.port} ${styles.outputPort}`} />
				</div>
				<span className={styles.brandLabel}>Flow-Like</span>
			</div>

			<div className={`${styles.node} ${styles.chat}`}>
				<span className={`${styles.port} ${styles.inputPort}`} />
				<div className={styles.nodeHeader}>
					<span className={styles.icon}>
						<MessageCircle />
					</span>
					<span>{t("onboarding.chat", "Chat")}</span>
					<Check className={styles.check} />
				</div>
				<div className={styles.chatPreview}>
					<span />
					<span />
				</div>
			</div>
			<div className={`${styles.node} ${styles.create}`}>
				<span className={`${styles.port} ${styles.inputPort}`} />
				<div className={styles.nodeHeader}>
					<span className={styles.icon}>
						<Sparkles />
					</span>
					<span>{t("onboarding.create", "Create")}</span>
					<Check className={styles.check} />
				</div>
				<div className={styles.createPreview}>
					<span />
					<span />
					<span />
				</div>
			</div>
			<div className={`${styles.node} ${styles.automate}`}>
				<span className={`${styles.port} ${styles.inputPort}`} />
				<div className={styles.nodeHeader}>
					<span className={styles.icon}>
						<Workflow />
					</span>
					<span>{t("onboarding.automate", "Automate")}</span>
					<Check className={styles.check} />
				</div>
				<div className={styles.automatePreview}>
					<span />
					<i />
					<span />
					<i />
					<span />
				</div>
			</div>
		</div>
	);
}
