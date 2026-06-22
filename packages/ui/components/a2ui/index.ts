// Core
export { A2UIRenderer, useA2UIState } from "./A2UIRenderer";
export { SurfaceManager, useSurfaceManager } from "./SurfaceManager";
export {
	DataProvider,
	useData,
	useDataValue,
	useResolvedValue,
} from "./DataContext";
export {
	ActionProvider,
	useActions,
	useComponentActionTrigger,
	useExecuteAction,
	useIsComponentTriggering,
	useMarkComponentTriggering,
	useOnAction,
} from "./ActionHandler";
export {
	WidgetActionProvider,
	useWidgetActions,
	useWidgetAction,
} from "./WidgetActionHandler";
export { getComponentRenderer, registerComponent } from "./ComponentRegistry";
export { resolveStyle, resolveInlineStyle, mergeStyles } from "./StyleResolver";
export { useElementGatherer, createElementPayload } from "./ElementGatherer";
export {
	RouteDialogProvider,
	useRouteDialog,
	useRouteDialogSafe,
} from "./RouteDialogProvider";
export { WidgetRefsProvider, useWidgetRefs } from "./WidgetRefsContext";

// Hooks
export {
	useDataBinding,
	useBoundValue,
	useDataPath,
	useSetDataPath,
	useSurface,
	useSurfaceComponent,
	useAction,
	useActionCallback,
} from "./hooks";

// Types
export type {
	// Core types
	Surface,
	SurfaceComponent,
	A2UIComponent,
	A2UIClientMessage,
	A2UIServerMessage,
	BoundValue,
	Style,
	Action,
	Widget,
	Page,
	DataEntry,
	Children,
	ChildrenTemplate,
	// Widget action types
	WidgetAction,
	WidgetActionContextField,
	ActionBinding,
	WorkflowBinding,
	CommandBinding,
	WidgetRef,
	// Component types
	RowComponent,
	ColumnComponent,
	StackComponent,
	GridComponent,
	ScrollAreaComponent,
	AspectRatioComponent,
	TextComponent,
	ImageComponent,
	IconComponent,
	VideoComponent,
	MarkdownComponent,
	DividerComponent,
	BadgeComponent,
	AvatarComponent,
	UserProfileComponent,
	ProgressComponent,
	SpinnerComponent,
	SkeletonComponent,
	ButtonComponent,
	TextFieldComponent,
	SelectComponent,
	SliderComponent,
	CheckboxComponent,
	SwitchComponent,
	RadioGroupComponent,
	DateTimeInputComponent,
	LinkComponent,
	CardComponent,
	ModalComponent,
	TabsComponent,
	AccordionComponent,
	DrawerComponent,
	TooltipComponent,
	PopoverComponent,
	// New layout types
	OverlayComponent,
	OverlayItem,
	AbsoluteComponent,
	// Display types
	LottieComponent,
	// Game component types
	Canvas2DComponent,
	SpriteComponent,
	ShapeComponent,
	Scene3DComponent,
	Model3DComponent,
	DialogueComponent,
	CharacterPortraitComponent,
	ChoiceMenuComponent,
	ChoiceComponent,
	InventoryGridComponent,
	InventoryItemDef,
	HealthBarComponent,
	MiniMapComponent,
	MapMarkerDef,
	GeoMapComponent,
	GeoMapMarkerDef,
	GeoMapRouteDef,
	GeoMapViewport,
	GeoCoordinate,
	GeoBoundingBox,
	GeoRouteGeometry,
	GeoRouteStep,
	GeoRouteLeg,
	GeoRouteResult,
	GeoTripWaypoint,
	GeoSearchResult,
} from "./types";

// Layout components
export { A2UIRow } from "./layout/Row";
export { A2UIColumn } from "./layout/Column";
export { A2UIStack } from "./layout/Stack";
export { A2UIGrid } from "./layout/Grid";
export { A2UIScrollArea } from "./layout/ScrollArea";
export { A2UIAspectRatio } from "./layout/AspectRatio";
export { A2UIOverlay } from "./layout/Overlay";
export { A2UIAbsolute } from "./layout/Absolute";

// Display components
export { A2UIText } from "./display/Text";
export { A2UIImage } from "./display/Image";
export { A2UIIcon } from "./display/Icon";
export { A2UIVideo } from "./display/Video";
export { A2UIMarkdown } from "./display/Markdown";
export { A2UIDivider } from "./display/Divider";
export { A2UIBadge } from "./display/Badge";
export { A2UIAvatar } from "./display/Avatar";
export { A2UIUserProfile } from "./display/UserProfile";
export { A2UIProgress } from "./display/Progress";
export { A2UISpinner } from "./display/Spinner";
export { A2UISkeleton } from "./display/Skeleton";
export { A2UILottie } from "./display/Lottie";
export { A2UIGeoMap } from "./display/GeoMap";

// Geo conversion utilities
export {
	routeResultToRouteDef,
	routeResultsToRouteDefs,
	searchResultToMarkerDef,
	searchResultsToMarkerDefs,
	tripWaypointToMarkerDef,
	tripWaypointsToMarkerDefs,
	routeStepsToMarkerDefs,
	searchResultToViewport,
} from "./geoConversions";

// Interactive components
export { A2UIButton } from "./interactive/Button";
export { A2UITextField } from "./interactive/TextField";
export { A2UISelect } from "./interactive/Select";
export { A2UISlider } from "./interactive/Slider";
export { A2UICheckbox } from "./interactive/Checkbox";
export { A2UISwitch } from "./interactive/Switch";
export { A2UIRadioGroup } from "./interactive/RadioGroup";
export { A2UIDateTimeInput } from "./interactive/DateTimeInput";

// Container components
export { A2UICard } from "./container/Card";
export { A2UIModal } from "./container/Modal";
export { A2UITabs } from "./container/Tabs";
export { A2UIAccordion } from "./container/Accordion";
export { A2UIDrawer } from "./container/Drawer";
export { A2UITooltip } from "./container/Tooltip";
export { A2UIPopover } from "./container/Popover";

// Game components
export { A2UICanvas2D } from "./game/Canvas2D";
export { A2UISprite } from "./game/Sprite";
export { A2UIShape } from "./game/Shape";
export { A2UIScene3D, type ControlMode, type FixedView } from "./game/Scene3D";
export { A2UIModel3D } from "./game/Model3D";
export { A2UIDialogue } from "./game/Dialogue";
export { A2UICharacterPortrait } from "./game/CharacterPortrait";
export { A2UIChoiceMenu } from "./game/ChoiceMenu";
export { A2UIInventoryGrid } from "./game/InventoryGrid";
export { A2UIHealthBar } from "./game/HealthBar";
export { A2UIMiniMap } from "./game/MiniMap";
