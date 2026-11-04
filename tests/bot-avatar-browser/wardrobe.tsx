import { useState } from "react";
import {
	BotAvatar,
	defaultBotCatalog,
	getBotEquipmentPacks,
	getBotPackContents,
	isBotEquipmentCompatible,
} from "../../packages/ui/components/bots";

export function BotWardrobe({ parameters }: { parameters: URLSearchParams }) {
	const requestedKind = parameters.get("kind") ?? "mochi";
	const [kind, setKind] = useState(
		Object.hasOwn(defaultBotCatalog.shapes, requestedKind)
			? requestedKind
			: "mochi",
	);
	const [size, setSize] = useState(120);
	const [packId, setPackId] = useState(parameters.get("pack") ?? "all");
	const shape = defaultBotCatalog.shapes[kind];
	const collections = [
		{
			id: "essentials",
			label: "Essentials",
			equipment: Object.values(defaultBotCatalog.equipment).filter(
				(item) => !getBotEquipmentPacks(item.id, defaultBotCatalog).length,
			),
		},
		...Object.values(defaultBotCatalog.packs).map((pack) => ({
			id: pack.id,
			label: pack.label,
			equipment: getBotPackContents(pack, defaultBotCatalog).equipment,
		})),
	].map((collection) => ({
		...collection,
		total: collection.equipment.length,
		equipment: collection.equipment.filter((item) =>
			isBotEquipmentCompatible(item, shape),
		),
	}));
	const supported = collections.reduce(
		(count, collection) => count + collection.equipment.length,
		0,
	);
	const total = Object.keys(defaultBotCatalog.equipment).length;
	return (
		<main
			data-wardrobe-kind={kind}
			data-compatible-equipment={supported}
			data-excluded-equipment={total - supported}
		>
			<header>
				<span className="eyebrow">FLOW-LIKE / WARDROBE</span>
				<h1>Clothes for every kind of day</h1>
				<p>
					{supported} of {total} accessories and outfits fit {shape.label}.
				</p>
				<a href="/">Customize a bot</a>
				{" · "}
				<a href="/?gallery">Meet the crew</a>
			</header>
			<fieldset className="gallery-controls" aria-label="Wardrobe controls">
				<label>
					Character
					<select
						aria-label="Wardrobe character"
						value={kind}
						onChange={(event) => setKind(event.target.value)}
					>
						{Object.values(defaultBotCatalog.shapes).map((shape) => (
							<option key={shape.id} value={shape.id}>
								{shape.label}
							</option>
						))}
					</select>
				</label>
				<label>
					Collection
					<select
						aria-label="Wardrobe collection"
						value={packId}
						onChange={(event) => setPackId(event.target.value)}
					>
						<option value="all">All collections</option>
						{collections.map((collection) => (
							<option key={collection.id} value={collection.id}>
								{collection.label}
							</option>
						))}
					</select>
				</label>
				<label>
					Size
					<select
						aria-label="Wardrobe size"
						value={size}
						onChange={(event) => setSize(Number(event.target.value))}
					>
						{[120, 64, 32].map((value) => (
							<option key={value} value={value}>
								{value} px
							</option>
						))}
					</select>
				</label>
			</fieldset>
			<div className="crew-sheet" data-wardrobe>
				{collections
					.filter((collection) => packId === "all" || collection.id === packId)
					.map((collection) => (
						<section key={collection.id} data-wardrobe-pack={collection.id}>
							<h2>{collection.label}</h2>
							<p>
								{collection.equipment.length} compatible items
								{collection.total > collection.equipment.length
									? `, ${collection.total - collection.equipment.length} excluded`
									: ""}
							</p>
							<div className="crew-row">
								{collection.equipment.map((item) => (
									<figure
										key={item.id}
										data-equipment-id={item.id}
										data-equipment-slot={item.slot}
										data-compact-skill={item.compactSkill}
									>
										<BotAvatar
											appearance={{ kind, equipment: { [item.slot]: item.id } }}
											pose="center"
											animated={false}
											size={size}
										/>
										<figcaption>{item.label}</figcaption>
									</figure>
								))}
							</div>
						</section>
					))}
			</div>
		</main>
	);
}
