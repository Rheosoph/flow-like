import type { BotEffect, BotPalette } from "./types";
import "./bot-effects.css";

interface EffectProps {
	effect: BotEffect;
	idPrefix: string;
	palette: BotPalette;
}

export function getBotEffectFilter(effect: BotEffect, idPrefix: string) {
	return effect === "none" || effect === "pixel" || effect === "offline"
		? undefined
		: `url(#${idPrefix}-look-${effect})`;
}

function ChannelShift({ distance }: { distance: number }) {
	return (
		<>
			<feColorMatrix
				in="SourceGraphic"
				values="1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 1 0"
				result="red"
			/>
			<feOffset in="red" dx={-distance} result="red-shift" />
			<feColorMatrix
				in="SourceGraphic"
				values="0 0 0 0 0 0 1 0 0 0 0 0 1 0 0 0 0 0 1 0"
				result="cyan"
			/>
			<feOffset in="cyan" dx={distance} result="cyan-shift" />
			<feBlend
				in="red-shift"
				in2="cyan-shift"
				mode="screen"
				result="channels"
			/>
		</>
	);
}

function PaperGrain({ strength = 0.2 }: { strength?: number }) {
	return (
		<>
			<feTurbulence
				type="fractalNoise"
				baseFrequency=".85"
				numOctaves="3"
				seed="17"
				result="paper"
			/>
			<feColorMatrix in="paper" type="saturate" values="0" />
			<feComponentTransfer result="grain">
				<feFuncR type="linear" slope={strength} intercept={1 - strength} />
				<feFuncG type="linear" slope={strength} intercept={1 - strength} />
				<feFuncB type="linear" slope={strength} intercept={1 - strength} />
				<feFuncA type="linear" slope="0" intercept="1" />
			</feComponentTransfer>
		</>
	);
}

/** Filters belong to the complete rig so clothing and thought bubbles share the look. */
export function BotEffectDefs({ effect, idPrefix, palette }: EffectProps) {
	const filter = getBotEffectFilter(effect, idPrefix);
	if (!filter) return null;
	return (
		<>
			<filter
				id={`${idPrefix}-look-${effect}`}
				data-effect-filter={effect}
				x="-15"
				y="-15"
				width="130"
				height="130"
				filterUnits="userSpaceOnUse"
				primitiveUnits="userSpaceOnUse"
				colorInterpolationFilters="sRGB"
			>
				{effect === "lcd" && (
					<>
						<feColorMatrix type="saturate" values="0" />
						<feComponentTransfer>
							<feFuncR type="discrete" tableValues=".1725 .2824 .5686 .8392" />
							<feFuncG type="discrete" tableValues=".2745 .3922 .6588 .8745" />
							<feFuncB type="discrete" tableValues=".1882 .2275 .4 .6784" />
						</feComponentTransfer>
					</>
				)}
				{effect === "blueprint" && (
					<>
						<feColorMatrix type="saturate" values="0" />
						<feComponentTransfer>
							<feFuncR type="table" tableValues=".07 .12 .2 .55 .9" />
							<feFuncG type="table" tableValues=".2 .34 .48 .76 .96" />
							<feFuncB type="table" tableValues=".36 .54 .73 .91 1" />
						</feComponentTransfer>
					</>
				)}
				{(effect === "crt" || effect === "glitch") && (
					<>
						<ChannelShift distance={effect === "crt" ? 0.6 : 1.25} />
						{effect === "crt" && (
							<>
								<feGaussianBlur
									in="channels"
									stdDeviation=".45"
									result="glow"
								/>
								<feBlend in="channels" in2="glow" mode="screen" />
							</>
						)}
					</>
				)}
				{effect === "thermal" && (
					<>
						<feGaussianBlur in="SourceGraphic" stdDeviation=".65" />
						<feColorMatrix type="saturate" values="0" />
						<feComponentTransfer result="heat">
							<feFuncR type="table" tableValues=".22 .7 1 1 1" />
							<feFuncG type="table" tableValues=".02 .05 .17 .59 .94" />
							<feFuncB type="table" tableValues=".35 .63 .22 0 .24" />
						</feComponentTransfer>
						<feGaussianBlur
							in="SourceAlpha"
							stdDeviation="1.1"
							result="heat-surface"
						/>
						<feSpecularLighting
							in="heat-surface"
							surfaceScale="3"
							specularConstant=".55"
							specularExponent="9"
							lightingColor="#fff176"
							result="heat-light"
						>
							<feDistantLight azimuth="225" elevation="55" />
						</feSpecularLighting>
						<feComposite
							in="heat-light"
							in2="SourceAlpha"
							operator="in"
							result="heat-highlight"
						/>
						<feMerge>
							<feMergeNode in="heat" />
							<feMergeNode in="heat-highlight" />
						</feMerge>
					</>
				)}
				{effect === "riso" && (
					<>
						<feColorMatrix in="SourceGraphic" type="saturate" values="0" />
						<feComponentTransfer result="ink">
							<feFuncR type="discrete" tableValues=".23 .82 .95" />
							<feFuncG type="discrete" tableValues=".2 .35 .65" />
							<feFuncB type="discrete" tableValues=".38 .55 .65" />
						</feComponentTransfer>
						<PaperGrain strength={0.28} />
						<feBlend in="ink" in2="grain" mode="multiply" />
						<feComposite in2="SourceAlpha" operator="in" />
					</>
				)}
				{effect === "raster" && <feColorMatrix type="saturate" values=".65" />}
				{effect === "comic" && (
					<>
						<feMorphology
							in="SourceAlpha"
							operator="dilate"
							radius="1.1"
							result="outline"
						/>
						<feFlood floodColor="#242034" />
						<feComposite in2="outline" operator="in" result="ink-outline" />
						<feComponentTransfer in="SourceGraphic" result="flat-colors">
							<feFuncR type="discrete" tableValues=".07 .3 .55 .8 1" />
							<feFuncG type="discrete" tableValues=".07 .3 .55 .8 1" />
							<feFuncB type="discrete" tableValues=".07 .3 .55 .8 1" />
						</feComponentTransfer>
						<feMerge>
							<feMergeNode in="ink-outline" />
							<feMergeNode in="flat-colors" />
						</feMerge>
					</>
				)}
				{effect === "watercolor" && (
					<>
						<feTurbulence
							type="fractalNoise"
							baseFrequency=".09"
							numOctaves="3"
							seed="8"
							result="paper-fibers"
						/>
						<feDisplacementMap
							in="SourceGraphic"
							in2="paper-fibers"
							scale=".85"
							xChannelSelector="R"
							yChannelSelector="G"
							result="pigment"
						/>
						<feGaussianBlur in="pigment" stdDeviation=".23" result="wash" />
						<feMorphology
							in="SourceAlpha"
							operator="erode"
							radius=".75"
							result="inner"
						/>
						<feComposite
							in="pigment"
							in2="inner"
							operator="out"
							result="edge"
						/>
						<feColorMatrix
							in="edge"
							values=".65 0 0 0 0 0 .65 0 0 0 0 0 .65 0 0 0 0 0 .45 0"
							result="pigment-edge"
						/>
						<feMerge result="paint">
							<feMergeNode in="wash" />
							<feMergeNode in="pigment-edge" />
						</feMerge>
						<PaperGrain strength={0.22} />
						<feBlend in="paint" in2="grain" mode="multiply" />
						<feComposite in2="SourceAlpha" operator="in" />
					</>
				)}
				{effect === "sticker" && (
					<>
						<feMorphology
							in="SourceAlpha"
							operator="dilate"
							radius="2.2"
							result="cutout"
						/>
						<feGaussianBlur in="cutout" stdDeviation="1.1" />
						<feOffset dy="2" />
						<feComponentTransfer result="shadow">
							<feFuncA type="linear" slope=".23" />
						</feComponentTransfer>
						<feFlood floodColor="#fff" />
						<feComposite in2="cutout" operator="in" result="white-edge" />
						<feMerge>
							<feMergeNode in="shadow" />
							<feMergeNode in="white-edge" />
							<feMergeNode in="SourceGraphic" />
						</feMerge>
					</>
				)}
				{effect === "vinyl" && (
					<>
						<feGaussianBlur
							in="SourceAlpha"
							stdDeviation="2"
							result="rounded-surface"
						/>
						<feDiffuseLighting
							in="rounded-surface"
							surfaceScale="5"
							diffuseConstant="1.2"
							lightingColor="#fff"
							result="surface-shade"
						>
							<feDistantLight azimuth="225" elevation="55" />
						</feDiffuseLighting>
						<feComposite
							in="SourceGraphic"
							in2="surface-shade"
							operator="arithmetic"
							k1=".8"
							k2=".2"
							result="shaded-toy"
						/>
						<feSpecularLighting
							in="rounded-surface"
							surfaceScale="5"
							specularConstant="1.1"
							specularExponent="22"
							lightingColor="#fffbe7"
							result="shine"
						>
							<feDistantLight azimuth="225" elevation="40" />
						</feSpecularLighting>
						<feComponentTransfer in="shine" result="specular-rim">
							<feFuncA type="linear" slope="1.8" intercept="-.18" />
						</feComponentTransfer>
						<feComposite
							in="specular-rim"
							in2="SourceAlpha"
							operator="in"
							result="highlight"
						/>
						<feMerge>
							<feMergeNode in="shaded-toy" />
							<feMergeNode in="highlight" />
						</feMerge>
					</>
				)}
				{effect === "neon" && (
					<>
						<feConvolveMatrix
							in="SourceGraphic"
							order="3"
							kernelMatrix="0 -1 0 -1 4 -1 0 -1 0"
							kernelUnitLength=".6 .6"
							preserveAlpha="true"
							result="contrast-edges"
						/>
						<feColorMatrix
							in="contrast-edges"
							type="luminanceToAlpha"
							result="detail-edges"
						/>
						<feMorphology
							in="SourceAlpha"
							operator="erode"
							radius=".85"
							result="inner"
						/>
						<feComposite
							in="SourceAlpha"
							in2="inner"
							operator="out"
							result="rim"
						/>
						<feComposite
							in="detail-edges"
							in2="rim"
							operator="arithmetic"
							k2="1.4"
							k3="1"
							result="edges"
						/>
						<feFlood floodColor={palette.body} />
						<feComposite in2="edges" operator="in" />
						<feComponentTransfer result="tubes">
							<feFuncR type="linear" slope=".5" intercept=".5" />
							<feFuncG type="linear" slope=".5" intercept=".5" />
							<feFuncB type="linear" slope=".5" intercept=".5" />
							<feFuncA type="linear" slope="1.8" />
						</feComponentTransfer>
						<feGaussianBlur in="tubes" stdDeviation="1.5" result="glow" />
						<feMerge>
							<feMergeNode in="glow" />
							<feMergeNode in="glow" />
							<feMergeNode in="tubes" />
						</feMerge>
					</>
				)}
			</filter>
			{(effect === "riso" || effect === "watercolor") && (
				<filter id={`${idPrefix}-paper`} x="0" y="0" width="100%" height="100%">
					<feTurbulence
						type="fractalNoise"
						baseFrequency=".85"
						numOctaves="3"
						seed="17"
					/>
					<feColorMatrix type="saturate" values="0" />
				</filter>
			)}
		</>
	);
}

export function BotEffectOverlay({ effect, idPrefix, palette }: EffectProps) {
	const screen = effect === "crt" || effect === "lcd";
	const print = effect === "raster" || effect === "comic";
	const paper = effect === "riso" || effect === "watercolor";
	if (!screen && !print && !paper && effect !== "blueprint") return null;
	const patternId = `${idPrefix}-look-texture`;
	return (
		<g
			data-effect-overlay={effect}
			className="fl-bot-effect-overlay"
			pointerEvents="none"
		>
			{paper ? (
				<rect
					width="100"
					height="100"
					filter={`url(#${idPrefix}-paper)`}
					opacity={effect === "riso" ? 0.15 : 0.12}
					style={{ mixBlendMode: "multiply" }}
				/>
			) : (
				<>
					<defs>
						<pattern
							id={patternId}
							width={effect === "blueprint" ? 5 : effect === "lcd" ? 1.5 : 2}
							height={effect === "blueprint" ? 5 : effect === "lcd" ? 1.5 : 2}
							patternUnits="userSpaceOnUse"
						>
							{effect === "blueprint" ? (
								<path
									d="M0 5V0H5"
									fill="none"
									stroke="#c6e9ff"
									strokeWidth=".28"
								/>
							) : effect === "lcd" ? (
								<path
									d="M0 1.5V0H1.5"
									fill="none"
									stroke={palette.ink}
									strokeWidth=".2"
								/>
							) : print ? (
								<circle
									cx=".7"
									cy=".7"
									r={effect === "comic" ? 0.38 : 0.46}
									fill="#272239"
								/>
							) : (
								<path
									d="M0 .5H2"
									fill="none"
									stroke="#33233c"
									strokeWidth=".55"
								/>
							)}
						</pattern>
					</defs>
					<rect
						width="100"
						height="100"
						fill={`url(#${patternId})`}
						opacity={
							effect === "blueprint"
								? 0.4
								: effect === "crt"
									? 0.24
									: effect === "raster"
										? 0.35
										: 0.2
						}
					/>
				</>
			)}
		</g>
	);
}
