import type { BotEmote as BotEmoteName, BotRenderContext } from "./types";
import "./bot-emote.css";

export function BotEmote({
	emote,
	context,
}: {
	emote: BotEmoteName;
	context: BotRenderContext;
}) {
	if (emote === "none") return null;
	const palette = context.solidPalette ?? context.palette;
	const monochrome = context.effect === "lcd" || context.effect === "blueprint";
	const colors = {
		ink: palette.ink,
		red: monochrome ? palette.ink : "#e95070",
		yellow: monochrome ? palette.ink : "#f3b82f",
		blue: monochrome ? palette.ink : "#5ca9d5",
		cloud: monochrome ? palette.ink : "#8c9fbe",
		green: monochrome ? palette.ink : "#329d69",
	};

	return (
		<g data-emote={emote} transform="translate(80 24)">
			<g
				data-emote-bubble="thought"
				fill={palette.sclera}
				stroke={palette.ink}
				strokeWidth="1.3"
			>
				<circle cx="-13.5" cy="15.5" r="1.3" />
				<circle cx="-9" cy="11" r="2" />
				<circle r="10.5" />
			</g>
			<g
				data-emote-icon={emote}
				fill={colors.ink}
				strokeLinecap="round"
				strokeLinejoin="round"
			>
				{emote === "heart" && (
					<path
						d="M0 6.5C-2 5-7 1.3-7-2.6c0-4.2 5-5.5 7-1.7 2-3.8 7-2.5 7 1.7C7 1.3 2 5 0 6.5Z"
						fill={colors.red}
					/>
				)}
				{emote === "note" && (
					<>
						<path
							d="M-3.5 3V-5l8-1.7v8"
							fill="none"
							stroke={colors.ink}
							strokeWidth="2"
						/>
						<ellipse cx="-5" cy="3.8" rx="2.8" ry="2.1" />
						<ellipse cx="3" cy="2.1" rx="2.8" ry="2.1" />
					</>
				)}
				{emote === "spark" && (
					<path
						d="M0-8C1.5-2.5 2.5-1.5 7.5 0 2.5 1.5 1.5 2.5 0 8-1.5 2.5-2.5 1.5-7.5 0-2.5-1.5-1.5-2.5 0-8Z"
						fill={colors.yellow}
					/>
				)}
				{emote === "alert" && (
					<>
						<path d="M-1.6-6.6h3.2L1 2h-2Z" />
						<circle cy="5" r="1.6" />
					</>
				)}
				{emote === "question" && (
					<>
						<path
							d="M-3.7-3.7C-3.6-7.3 3.8-7.5 3.8-3.7 3.8-.7 0-.9 0 2"
							fill="none"
							stroke={colors.ink}
							strokeWidth="2.3"
						/>
						<circle cy="5.3" r="1.4" />
					</>
				)}
				{emote === "drop" && (
					<path
						d="M0-7.5C-1.5-5-5.5-.6-5.5 2.1a5.5 5.5 0 0 0 11 0C5.5-.6 1.5-5 0-7.5Z"
						fill={colors.blue}
					/>
				)}
				{emote === "zzz" && (
					<text
						x="0"
						y="0.5"
						textAnchor="middle"
						dominantBaseline="central"
						fontFamily="ui-sans-serif, system-ui, sans-serif"
						fontSize="11"
						fontWeight="800"
					>
						zZ
					</text>
				)}
				{emote === "rain" && (
					<>
						<path
							d="M-4.5 1a3.4 3.4 0 0 1-.3-6.8 4.6 4.6 0 0 1 8.5-.5A3.6 3.6 0 0 1 4.5 1Z"
							fill={colors.cloud}
						/>
						<path
							d="m-4 3-1 2.5M0 3l-1 3.5M4 3 3 5.5"
							fill="none"
							stroke={colors.blue}
							strokeWidth="1.9"
						/>
					</>
				)}
				{emote === "anger" && (
					<path
						d="M-1.8-6.5q.3 4.4-4.7 4.7M1.8-6.5q-.3 4.4 4.7 4.7M6.5 1.8q-5-.3-4.7 4.7M-6.5 1.8q5-.3 4.7 4.7"
						fill="none"
						stroke={colors.red}
						strokeWidth="2"
					/>
				)}
				{emote === "loading" &&
					[-4.5, 0, 4.5].map((x, index) => (
						<circle
							key={x}
							className="fl-bot-emote-dot"
							data-loading-dot={index}
							cx={x}
							r="1.7"
						/>
					))}
				{emote === "idea" && (
					<>
						<path
							d="M-2.7 3.7c0-2.2-3-3.1-3-6.2a5.7 5.7 0 0 1 11.4 0c0 3.1-3 4-3 6.2Z"
							fill={colors.yellow}
						/>
						<path
							d="M-2.5 6h5"
							fill="none"
							stroke={colors.ink}
							strokeWidth="2"
						/>
					</>
				)}
				{emote === "check" && (
					<path
						d="m-5.6.2 3.7 4L5.8-4"
						fill="none"
						stroke={colors.green}
						strokeWidth="3.2"
					/>
				)}
				{emote === "error" && (
					<path
						d="m-4.6-4.6 9.2 9.2m0-9.2-9.2 9.2"
						fill="none"
						stroke={colors.red}
						strokeWidth="3"
					/>
				)}
			</g>
		</g>
	);
}
