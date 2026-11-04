import { useState } from "react";
import {
	BotAvatar,
	type BotEquipmentDefinition,
	type BotFrame,
	defaultBotCatalog,
	isBotEquipmentCompatible,
} from "../../packages/ui/components/bots";

const REVIEW_HEADS = [
	"ruffled-bonnet",
	"headset",
	"crown",
	"chef-toque",
	"winter-earmuffs",
	"ear-defenders",
	"diving-hood",
	"motorcycle-helmet",
	"cat-ear-band",
	"flow-snapback",
	"tennis-visor",
];

export function BotEquipmentFit({
	parameters,
}: { parameters: URLSearchParams }) {
	const requested = parameters.get("kind") ?? "fern";
	const [kind, setKind] = useState(
		Object.hasOwn(defaultBotCatalog.shapes, requested) ? requested : "fern",
	);
	const shape = defaultBotCatalog.shapes[kind];
	const audit = parameters.has("audit");
	const allHeads = Object.values(defaultBotCatalog.equipment).filter(
		(item) => item.slot === "head",
	);
	const supportedHeads = allHeads.filter((item) =>
		isBotEquipmentCompatible(item, shape),
	);
	const requestedHeads = parameters.getAll("head");
	const headIds = requestedHeads.length ? requestedHeads : REVIEW_HEADS;
	const heads = audit
		? supportedHeads
		: supportedHeads.filter((item) => headIds.includes(item.id));
	const glasses = defaultBotCatalog.equipment.glasses;
	const eyewear =
		glasses && isBotEquipmentCompatible(glasses, shape) ? glasses : undefined;
	const looks: {
		head?: BotEquipmentDefinition;
		eyes?: BotEquipmentDefinition;
		label: string;
	}[] = audit
		? heads.map((head) => ({ head, label: head.label }))
		: [
				{ label: "Bare character" },
				...(eyewear ? [{ eyes: eyewear, label: eyewear.label }] : []),
				...heads.flatMap((head) => [
					{ head, label: head.label },
					...(eyewear
						? [
								{
									head,
									eyes: eyewear,
									label: `${head.label} + ${eyewear.label}`,
								},
							]
						: []),
				]),
			];
	const frames: readonly BotFrame[] = audit ? ["circle"] : ["circle", "square"];
	return (
		<main
			data-equipment-fit
			data-equipment-fit-kind={kind}
			data-supported-heads={supportedHeads.length}
			data-total-heads={allHeads.length}
		>
			<header>
				<span className="eyebrow">FLOW-LIKE / EQUIPMENT FIT</span>
				<h1>Headwear and face clearance</h1>
				<p>
					{supportedHeads.length} supported head items,{" "}
					{allHeads.length - supportedHeads.length} excluded for {shape.label}.
				</p>
			</header>
			<fieldset className="gallery-controls">
				<label>
					Character
					<select
						aria-label="Equipment fit character"
						value={kind}
						onChange={(event) => setKind(event.target.value)}
					>
						{Object.values(defaultBotCatalog.shapes).map((item) => (
							<option key={item.id} value={item.id}>
								{item.label}
							</option>
						))}
					</select>
				</label>
			</fieldset>
			{frames.map((frame) => (
				<section key={frame} data-fit-frame={frame}>
					<h2>
						{shape.label}: {frame}
					</h2>
					<div
						style={{
							display: "grid",
							gridTemplateColumns: `repeat(auto-fit, minmax(${audit ? 150 : 200}px, 1fr))`,
							gap: 16,
						}}
					>
						{looks.map(({ head, eyes, label }) => (
							<figure
								key={`${head?.id}-${eyes?.id}`}
								style={{
									margin: 0,
									padding: 12,
									background: "white",
									borderRadius: 12,
									textAlign: "center",
								}}
							>
								<div
									style={{
										display: "flex",
										alignItems: "end",
										justifyContent: "center",
										gap: 8,
									}}
								>
									{(audit ? [120] : [120, 32]).map((size) => (
										<div
											key={size}
											data-equipment-fit-sample
											data-head-id={head?.id ?? "none"}
											data-eyes-id={eyes?.id ?? "none"}
											data-size={size}
											data-frame={frame}
											data-eye-left={shape.anchors.eyes.left.join(",")}
											data-eye-right={shape.anchors.eyes.right.join(",")}
											data-eye-radius={shape.anchors.eyes.radius}
										>
											<BotAvatar
												appearance={{
													kind,
													frame,
													equipment: { head: head?.id, eyes: eyes?.id },
												}}
												pose="center"
												animated={false}
												size={size}
											/>
										</div>
									))}
								</div>
								<figcaption style={{ marginTop: 12, fontSize: 12 }}>
									{label}
								</figcaption>
							</figure>
						))}
					</div>
				</section>
			))}
		</main>
	);
}
