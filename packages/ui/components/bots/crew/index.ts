import { clothingBySeries } from "../equipment/clothing";
import { handPropsBySeries } from "../equipment/hand-props";
import { headwearBySeries } from "../equipment/headwear";
import type {
	BotEquipmentDefinition,
	BotPackDefinition,
	BotSeriesDefinition,
} from "../types";
import { adventureShapes } from "./adventure";
import { arcadeShapes } from "./arcade";
import { arcaneShapes } from "./arcane";
import { basicsShapes } from "./basics";
import { celebrationsShapes } from "./celebrations";
import { cosmosShapes } from "./cosmos";
import { fantasyShapes } from "./fantasy";
import { flowShapes } from "./flow";
import { gardenShapes } from "./garden";
import { healthcareEquipment, healthcareShapes } from "./healthcare";
import { musicShapes } from "./music";
import { natureShapes } from "./nature";
import { oceanShapes } from "./ocean";
import { officeShapes } from "./office";
import { petsShapes } from "./pets";
import { piratesShapes } from "./pirates";
import { proShapes } from "./pro";
import { serviceEquipment, serviceShapes } from "./service";
import { snacksShapes } from "./snacks";
import { techShapes } from "./tech";
import { workplaceEquipment, workplaceShapes } from "./workplace";
import { workshopShapes } from "./workshop";
import { zooShapes } from "./zoo";

export const crewSeries: readonly BotSeriesDefinition[] = [
	{
		id: "flow",
		label: "Flow",
		description: "Our home crew, inspired by Flow-Like.",
	},
	{
		id: "basics",
		label: "Basics",
		description: "Friendly shapes that fit anywhere.",
	},
	{
		id: "nature",
		label: "Nature",
		description: "Plants, water, and a little warmth.",
	},
	{
		id: "pets",
		label: "Furry friends",
		description: "Companions from home and the forest.",
	},
	{
		id: "zoo",
		label: "Zoo",
		description: "From the pond to the bamboo grove.",
	},
	{ id: "tech", label: "Tech", description: "Little helpers for big ideas." },
	{
		id: "snacks",
		label: "Snacks",
		description: "A handful of favorite treats.",
	},
	{
		id: "cosmos",
		label: "Cosmos",
		description: "Planets, stars, and space travelers.",
	},
	{
		id: "pro",
		label: "Pro",
		description: "Quiet colors and understated characters.",
	},
	{
		id: "office",
		label: "Office",
		description: "Paperclips, pencils, a mouse, and other desk companions.",
	},
	{
		id: "workshop",
		label: "Workshop",
		description: "Tools and craft supplies with projects of their own.",
	},
	{
		id: "ocean",
		label: "Ocean",
		description: "Sea creatures from the reef and the open water.",
	},
	{
		id: "fantasy",
		label: "Fairy tales",
		description: "A dragon, a wandering ghost, and objects full of magic.",
	},
	{
		id: "garden",
		label: "Garden",
		description: "Plants and small companions from the garden.",
	},
	{
		id: "music",
		label: "Music",
		description: "Instruments and music players ready for a jam session.",
	},
	{
		id: "adventure",
		label: "Adventure",
		description: "Trail companions for camping trips and distant shores.",
	},
	{
		id: "arcade",
		label: "Arcade",
		description: "Pocket games and playful pieces for the next round.",
	},
	{
		id: "pirates",
		label: "Pirates",
		description: "Treasure, nautical tools, and a parrot from the high seas.",
	},
	{
		id: "arcane",
		label: "Magic school",
		description:
			"A wizard hat, a flying broom, and objects from the potion classroom.",
	},
	{
		id: "celebrations",
		label: "Celebrations",
		description:
			"Christmas, Halloween, Easter, New Year, Lunar New Year, and Diwali.",
	},
	{
		id: "service",
		label: "On duty",
		description:
			"Objects from rescue crews, patrols, and field teams, with uniforms to try on.",
	},
	{
		id: "workplace",
		label: "Workday",
		description:
			"Desk companions from reception to the meeting room, with office outfits.",
	},
	{
		id: "healthcare",
		label: "Healthcare",
		description:
			"Medical objects from the clinic and pharmacy, with coats, tunics, and scrubs.",
	},
];

export const crewShapes = [
	...flowShapes,
	...basicsShapes,
	...natureShapes,
	...petsShapes,
	...zooShapes,
	...techShapes,
	...snacksShapes,
	...cosmosShapes,
	...proShapes,
	...officeShapes,
	...workshopShapes,
	...oceanShapes,
	...fantasyShapes,
	...gardenShapes,
	...musicShapes,
	...adventureShapes,
	...arcadeShapes,
	...piratesShapes,
	...arcaneShapes,
	...celebrationsShapes,
	...serviceShapes,
	...workplaceShapes,
	...healthcareShapes,
];

const uniformEquipment: Readonly<
	Record<string, readonly BotEquipmentDefinition[]>
> = {
	service: serviceEquipment,
	workplace: workplaceEquipment,
	healthcare: healthcareEquipment,
};

const equipmentBySeries = Object.fromEntries(
	crewSeries.map(({ id }) => [
		id,
		[
			...(uniformEquipment[id] ?? []),
			...(headwearBySeries[id] ?? []),
			...(clothingBySeries[id] ?? []),
			...(handPropsBySeries[id] ?? []),
		],
	]),
);

export const crewEquipment = Object.values(equipmentBySeries).flat();

export const crewPacks: readonly BotPackDefinition[] = crewSeries.map(
	(series) => {
		const shapeIds = crewShapes
			.filter((shape) => shape.series === series.id)
			.map((shape) => shape.id);
		const equipment = Object.hasOwn(equipmentBySeries, series.id)
			? equipmentBySeries[series.id as keyof typeof equipmentBySeries]
			: [];
		return {
			id: `${series.id}-crew`,
			label: series.label,
			description: series.description,
			coverKind: shapeIds[0],
			shapeIds,
			...(equipment.length
				? { equipmentIds: equipment.map((item) => item.id) }
				: {}),
		};
	},
);
