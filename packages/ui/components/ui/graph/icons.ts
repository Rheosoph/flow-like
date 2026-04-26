import {
	Activity,
	AlertTriangle,
	Bike,
	Briefcase,
	Building2,
	Calendar,
	Car,
	Cpu,
	CreditCard,
	Database,
	FileSignature,
	FileText,
	Globe,
	Laptop,
	Link,
	type LucideIcon,
	MapPin,
	Network,
	Package,
	Plane,
	Router,
	Server,
	Shield,
	Ship,
	Smartphone,
	Tag,
	Truck,
	User,
	UserCog,
	Users,
	Wallet,
} from "lucide-react";

export const GRAPH_ICONS: Record<string, LucideIcon> = {
	// people
	user: User,
	users: Users,
	shield: Shield,
	userCog: UserCog,
	// vehicles
	car: Car,
	truck: Truck,
	ship: Ship,
	plane: Plane,
	bike: Bike,
	// devices
	laptop: Laptop,
	smartphone: Smartphone,
	cpu: Cpu,
	server: Server,
	router: Router,
	// places
	mapPin: MapPin,
	building: Building2,
	globe: Globe,
	// entities
	briefcase: Briefcase,
	fileText: FileText,
	fileSignature: FileSignature,
	// events / time
	calendar: Calendar,
	activity: Activity,
	alertTriangle: AlertTriangle,
	// commerce
	creditCard: CreditCard,
	package: Package,
	wallet: Wallet,
	// misc
	database: Database,
	network: Network,
	link: Link,
	tag: Tag,
};

export type IconKey = keyof typeof GRAPH_ICONS;

export function getGraphIcon(key: string): LucideIcon {
	return GRAPH_ICONS[key] ?? Database;
}
