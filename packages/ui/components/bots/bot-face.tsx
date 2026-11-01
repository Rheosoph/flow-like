import type { BotRenderContext, BotShapeDefinition } from "./types";

function expressionColor(
	context: BotRenderContext,
	color: string,
	monochromeColor: string,
) {
	return context.effect === "lcd" ||
		context.effect === "blueprint" ||
		context.effect === "offline"
		? monochromeColor
		: color;
}

export function BotEyes({
	shape,
	context,
}: {
	shape: BotShapeDefinition;
	context: BotRenderContext;
}) {
	const { palette, expression, pose } = context;
	const eye = shape.anchors.eyes;
	const style = shape.eyeStyle ?? "round";
	const round = style === "round" || style === "sleepy";
	const eyeColor = style === "light" ? palette.sclera : palette.ink;
	const closedColor =
		palette[shape.closedEyeColor ?? (style === "light" ? "sclera" : "ink")];
	const eyes = [eye.left, eye.right];
	if (pose === "sleep" || pose === "blink")
		return (
			<g
				data-feature="closed-eyes"
				fill="none"
				stroke={closedColor}
				strokeWidth="1.8"
				strokeLinecap="round"
			>
				{eyes.map(([x, y]) => (
					<path
						key={`${x}-${y}`}
						d={`M${x - eye.radius * 0.72} ${y}q${eye.radius * 0.72} ${eye.radius * 0.5} ${eye.radius * 1.44} 0`}
					/>
				))}
			</g>
		);
	if (expression === "happy")
		return (
			<g
				data-part="pupils"
				className="fl-bot-pupils"
				fill="none"
				stroke={closedColor}
				strokeWidth="2"
				strokeLinecap="round"
			>
				{eyes.map(([x, y]) => (
					<path
						key={`${x}-${y}`}
						className="fl-bot-happy-eye"
						d={`M${x - eye.radius * 0.8} ${y}q${eye.radius * 0.8} ${-eye.radius} ${eye.radius * 1.6} 0`}
					/>
				))}
			</g>
		);
	if (expression === "love")
		return (
			<g
				data-feature="heart-eyes"
				data-part="pupils"
				className="fl-bot-pupils"
				fill={expressionColor(context, "#ec5578", closedColor)}
			>
				{eyes.map(([x, y]) => (
					<path
						key={`${x}-${y}`}
						transform={`translate(${x} ${y}) scale(${Math.max(eye.radius, 3.5) / 5})`}
						d="M0 4.8C-1 4-5.4 1.1-5.4-2.1C-5.4-5.1-1.8-5.7 0-3.1C1.8-5.7 5.4-5.1 5.4-2.1C5.4 1.1 1 4 0 4.8Z"
					/>
				))}
			</g>
		);
	if (expression === "oops")
		return (
			<g
				data-feature="squeezed-eyes"
				fill="none"
				stroke={closedColor}
				strokeWidth="2.1"
				strokeLinecap="round"
				strokeLinejoin="round"
			>
				{eyes.map(([x, y], side) => {
					const span = Math.max(eye.radius * 0.72, 2.3);
					const direction = side === 0 ? 1 : -1;
					return (
						<path
							key={`${x}-${y}`}
							d={`M${x - span * direction} ${y - span * 0.65}L${x + span * direction} ${y}L${x - span * direction} ${y + span * 0.65}`}
						/>
					);
				})}
			</g>
		);

	const clipId = `${context.idPrefix}-eyelids`;
	const pupilClipId = `${context.idPrefix}-eye-whites`;
	const sleepy = style === "sleepy" && expression === "neutral";
	const loweredLids =
		sleepy || expression === "focused" || expression === "sad";
	const eyeHeight =
		expression === "surprised" ? 1.3 : expression === "sad" ? 0.85 : 1.12;
	const pupilScale = expression === "surprised" ? 0.8 : 1;
	const openEyes = expression === "wink" ? [eye.left] : eyes;
	const lidY = (y: number) =>
		sleepy ? y - 1 : y - eye.radius * (expression === "sad" ? 0.35 : 0.08);
	return (
		<>
			{round && (
				<defs>
					<clipPath id={pupilClipId}>
						{openEyes.map(([x, y]) => (
							<ellipse
								key={`${x}-${y}`}
								cx={x}
								cy={y}
								rx={eye.radius}
								ry={eye.radius * eyeHeight}
							/>
						))}
					</clipPath>
				</defs>
			)}
			{loweredLids && (
				<defs>
					<clipPath id={clipId}>
						{openEyes.map(([x, y]) => (
							<rect
								className="lid"
								key={`${x}-${y}`}
								x={x - eye.radius - 1}
								y={lidY(y)}
								width={eye.radius * 2 + 2}
								height={eye.radius * 2 + 2}
							/>
						))}
					</clipPath>
				</defs>
			)}
			<g clipPath={loweredLids ? `url(#${clipId})` : undefined}>
				{round &&
					openEyes.map(([x, y]) => (
						<ellipse
							key={`${x}-${y}`}
							className="sclera"
							cx={x}
							cy={y}
							rx={eye.radius}
							ry={eye.radius * eyeHeight}
							fill={palette.sclera}
						/>
					))}
				{/* The clips stay still while pupils move inside the eye whites. */}
				<g clipPath={round ? `url(#${pupilClipId})` : undefined}>
					<g data-part="pupils" className="fl-bot-pupils">
						{openEyes.map(([x, y]) => (
							<g key={`${x}-${y}`}>
								<ellipse
									cx={x}
									cy={y + (round ? 0.5 : 0)}
									rx={eye.radius * (round ? 0.49 : 0.8) * pupilScale}
									ry={eye.radius * (round ? 0.59 : eyeHeight) * pupilScale}
									fill={eyeColor}
								/>
								{style !== "light" && (
									<circle
										cx={x - eye.radius * (round ? 0.16 : 0.22)}
										cy={y - eye.radius * (round ? 0.2 : 0.36)}
										r={eye.radius * (round ? 0.17 : 0.22)}
										fill={palette.sclera}
									/>
								)}
							</g>
						))}
					</g>
				</g>
			</g>
			{expression === "wink" && (
				<path
					data-feature="wink-eye"
					d={`M${eye.right[0] - eye.radius * 0.8} ${eye.right[1]}q${eye.radius * 0.8} ${-eye.radius} ${eye.radius * 1.6} 0`}
					fill="none"
					stroke={closedColor}
					strokeWidth="2"
					strokeLinecap="round"
				/>
			)}
			{(expression === "sad" || expression === "focused") && (
				<g
					data-feature="lowered-lids"
					fill="none"
					stroke={eyeColor}
					strokeWidth="1.4"
					strokeLinecap="round"
				>
					{eyes.map(([x, y]) => (
						<path
							key={`${x}-${y}`}
							d={`M${x - eye.radius * 0.9} ${lidY(y)}h${eye.radius * 1.8}`}
						/>
					))}
				</g>
			)}
		</>
	);
}

export function BotFace({
	shape,
	context,
}: { shape: BotShapeDefinition; context: BotRenderContext }) {
	const { palette, pose, expression } = context;
	const {
		eyes: eye,
		mouth: [mx, my],
	} = shape.anchors;
	const style = shape.mouthStyle ?? "smile";
	const scale = shape.mouthScale ?? 1;
	const mouthColor = palette[shape.mouthColor ?? "ink"];
	const sleeping = pose === "sleep";
	const expressive = !sleeping && expression !== "neutral";
	const smiling =
		expression === "happy" || expression === "wink" || expression === "love";
	const mouthPath = sleeping
		? "M-2 0q2 1.5 4 0"
		: expression === "sad" || expression === "angry"
			? "M-4 2q4 -5 8 0"
			: expression === "focused"
				? "M-3 0H3"
				: expression === "oops"
					? "M-4 1q2 -3 4 0t4 0"
					: expression === "concerned"
						? "M-3 1q3 -2 6 0"
						: expression === "curious"
							? "M-2 0q2 1 5 -1"
							: smiling
								? style === "cat"
									? "M-4 -1q2 5 4 0q2 5 4 0"
									: "M-4 -1q4 7 8 0"
								: style === "line"
									? "M-3 0H3"
									: style === "cat"
										? "M-4 -1q2 4 4 0q2 4 4 0"
										: "M-3 -1q3 4 6 0";
	const showMouth =
		style !== "none" ||
		(expressive && expression !== "concerned" && expression !== "curious");
	return (
		<>
			{expressive && !smiling && expression !== "oops" && (
				<g
					data-feature="brows"
					fill="none"
					stroke={shape.eyeStyle === "light" ? palette.sclera : palette.ink}
					strokeWidth="1.5"
					strokeLinecap="round"
				>
					{[eye.left, eye.right].map(([x, y], side) => {
						const span = Math.max(eye.radius * 0.8, 2.5);
						const baseline = y - eye.radius * 1.5;
						const inward = side === 0 ? -1 : 1;
						const slope = expression === "sad" ? 2.6 : 1.7;
						const lift = expression === "curious" && side === 1 ? 3 : 0;
						return (
							<path
								key={`${x}-${y}`}
								d={
									expression === "focused"
										? `M${x - span} ${baseline}h${span * 2}`
										: expression === "angry"
											? `M${x - span} ${baseline + inward * 2.2}L${x + span} ${baseline - inward * 2.2}`
											: expression === "surprised"
												? `M${x - span} ${baseline - 1}q${span} -3 ${span * 2} 0`
												: expression === "curious"
													? `M${x - span} ${baseline - lift}q${span} ${side === 1 ? -2 : 1} ${span * 2} 0`
													: `M${x - span} ${baseline - inward * slope}Q${x} ${baseline - 1} ${x + span} ${baseline + inward * slope}`
								}
							/>
						);
					})}
				</g>
			)}
			{shape.cheeks !== false && (
				<g data-feature="cheeks" fill={palette.cheek} opacity=".75">
					<ellipse cx={eye.left[0] - 6} cy={my - 1} rx="3.5" ry="1.8" />
					<ellipse cx={eye.right[0] + 6} cy={my - 1} rx="3.5" ry="1.8" />
				</g>
			)}
			{!sleeping && expression === "sad" && (
				<path
					data-feature="tear"
					transform={`translate(${eye.right[0] + eye.radius * 0.85} ${eye.right[1] + eye.radius * 1.05})`}
					d="M0 0C-.8 1.1-2 2.5-2 3.6a2 2 0 0 0 4 0C2 2.5.8 1.1 0 0Z"
					fill={expressionColor(
						context,
						"#65bce8",
						palette[shape.eyeStyle === "light" ? "sclera" : "ink"],
					)}
				/>
			)}
			{showMouth && (
				<g
					data-feature="mouth"
					transform={`translate(${mx} ${my}) scale(${scale})`}
				>
					{style === "beak" ? (
						<>
							<path
								d={
									!sleeping && expression === "surprised"
										? "M-5 -3Q0 -5 5 -3L0 6Z"
										: "M-4 -2Q0 -4 4 -2L0 4Z"
								}
								fill={palette.accent}
							/>
							{expressive &&
								(expression === "focused" ||
									expression === "sad" ||
									expression === "angry" ||
									expression === "oops") && (
									<path
										d={mouthPath}
										transform="translate(0 -.5) scale(.55)"
										fill="none"
										stroke={mouthColor}
										strokeWidth="1.7"
										strokeLinecap="round"
									/>
								)}
						</>
					) : !sleeping &&
						(expression === "surprised" || (!expressive && style === "o")) ? (
						<ellipse rx="2" ry="3" fill={mouthColor} />
					) : (
						<path
							d={mouthPath}
							fill="none"
							stroke={mouthColor}
							strokeWidth="1.7"
							strokeLinecap="round"
						/>
					)}
				</g>
			)}
			{shape.renderFaceDetails?.(context)}
		</>
	);
}
