// The console's icon set: the Lucide glyphs this client actually draws, named
// once so a destination or a component asks for a name rather than importing a
// component of its own.
//
// Deep imports rather than the package barrel, because the barrel re-exports
// every icon Lucide ships and this console draws about sixty of them. The map below is total over `IconName`, so a name added to the
// union without an import stops the type check instead of rendering nothing.

import type { LucideIcon } from '@lucide/svelte';

import Activity from '@lucide/svelte/icons/activity';
import ArrowLeft from '@lucide/svelte/icons/arrow-left';
import ArrowRightLeft from '@lucide/svelte/icons/arrow-right-left';
import Bell from '@lucide/svelte/icons/bell';
import Bold from '@lucide/svelte/icons/bold';
import BookOpen from '@lucide/svelte/icons/book-open';
import Building2 from '@lucide/svelte/icons/building-2';
import Calendar from '@lucide/svelte/icons/calendar';
import CalendarClock from '@lucide/svelte/icons/calendar-clock';
import ChartLine from '@lucide/svelte/icons/chart-line';
import Check from '@lucide/svelte/icons/check';
import ChevronDown from '@lucide/svelte/icons/chevron-down';
import ChevronLeft from '@lucide/svelte/icons/chevron-left';
import ChevronRight from '@lucide/svelte/icons/chevron-right';
import CircleAlert from '@lucide/svelte/icons/circle-alert';
import CircleCheck from '@lucide/svelte/icons/circle-check';
import CirclePlus from '@lucide/svelte/icons/circle-plus';
import CircleQuestionMark from '@lucide/svelte/icons/circle-question-mark';
import CircleUser from '@lucide/svelte/icons/circle-user';
import CircleX from '@lucide/svelte/icons/circle-x';
import Clock from '@lucide/svelte/icons/clock';
import Copy from '@lucide/svelte/icons/copy';
import CreditCard from '@lucide/svelte/icons/credit-card';
import Download from '@lucide/svelte/icons/download';
import EllipsisVertical from '@lucide/svelte/icons/ellipsis-vertical';
import ExternalLink from '@lucide/svelte/icons/external-link';
import Eye from '@lucide/svelte/icons/eye';
import FileDown from '@lucide/svelte/icons/file-down';
import Files from '@lucide/svelte/icons/files';
import Filter from '@lucide/svelte/icons/filter';
import Gift from '@lucide/svelte/icons/gift';
import HeartPulse from '@lucide/svelte/icons/heart-pulse';
import Image from '@lucide/svelte/icons/image';
import Info from '@lucide/svelte/icons/info';
import Italic from '@lucide/svelte/icons/italic';
import Laptop from '@lucide/svelte/icons/laptop';
import Layers from '@lucide/svelte/icons/layers';
import LayoutDashboard from '@lucide/svelte/icons/layout-dashboard';
import LayoutList from '@lucide/svelte/icons/layout-list';
import LayoutTemplate from '@lucide/svelte/icons/layout-template';
import LibraryBig from '@lucide/svelte/icons/library-big';
import List from '@lucide/svelte/icons/list';
import Lock from '@lucide/svelte/icons/lock';
import LogOut from '@lucide/svelte/icons/log-out';
import Minus from '@lucide/svelte/icons/minus';
import Monitor from '@lucide/svelte/icons/monitor';
import Package from '@lucide/svelte/icons/package';
import Pause from '@lucide/svelte/icons/pause';
import Pencil from '@lucide/svelte/icons/pencil';
import Plus from '@lucide/svelte/icons/plus';
import RefreshCw from '@lucide/svelte/icons/refresh-cw';
import Search from '@lucide/svelte/icons/search';
import Share2 from '@lucide/svelte/icons/share-2';
import ShoppingBag from '@lucide/svelte/icons/shopping-bag';
import ShieldCheck from '@lucide/svelte/icons/shield-check';
import SlidersHorizontal from '@lucide/svelte/icons/sliders-horizontal';
import Sparkles from '@lucide/svelte/icons/sparkles';
import Smartphone from '@lucide/svelte/icons/smartphone';
import Store from '@lucide/svelte/icons/store';
import Tag from '@lucide/svelte/icons/tag';
import Trash2 from '@lucide/svelte/icons/trash-2';
import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
import Upload from '@lucide/svelte/icons/upload';
import Users from '@lucide/svelte/icons/users';
import WavesHorizontal from '@lucide/svelte/icons/waves-horizontal';
import Workflow from '@lucide/svelte/icons/workflow';
import X from '@lucide/svelte/icons/x';

export const ICONS = {
	activity: Activity,
	'arrow-left': ArrowLeft,
	'arrow-right-left': ArrowRightLeft,
	bell: Bell,
	bold: Bold,
	'book-open': BookOpen,
	'building-2': Building2,
	calendar: Calendar,
	'calendar-clock': CalendarClock,
	'chart-line': ChartLine,
	check: Check,
	'chevron-down': ChevronDown,
	'chevron-left': ChevronLeft,
	'chevron-right': ChevronRight,
	'circle-alert': CircleAlert,
	'circle-check': CircleCheck,
	'circle-plus': CirclePlus,
	'circle-question-mark': CircleQuestionMark,
	'circle-user': CircleUser,
	'circle-x': CircleX,
	clock: Clock,
	copy: Copy,
	'credit-card': CreditCard,
	download: Download,
	'ellipsis-vertical': EllipsisVertical,
	'external-link': ExternalLink,
	eye: Eye,
	'file-down': FileDown,
	files: Files,
	filter: Filter,
	gift: Gift,
	'heart-pulse': HeartPulse,
	image: Image,
	info: Info,
	italic: Italic,
	laptop: Laptop,
	layers: Layers,
	'layout-dashboard': LayoutDashboard,
	'layout-list': LayoutList,
	'layout-template': LayoutTemplate,
	'library-big': LibraryBig,
	list: List,
	lock: Lock,
	'log-out': LogOut,
	minus: Minus,
	monitor: Monitor,
	package: Package,
	pause: Pause,
	pencil: Pencil,
	plus: Plus,
	'refresh-cw': RefreshCw,
	search: Search,
	'share-2': Share2,
	'shopping-bag': ShoppingBag,
	'shield-check': ShieldCheck,
	'sliders-horizontal': SlidersHorizontal,
	sparkles: Sparkles,
	smartphone: Smartphone,
	store: Store,
	tag: Tag,
	'trash-2': Trash2,
	'triangle-alert': TriangleAlert,
	upload: Upload,
	users: Users,
	'waves-horizontal': WavesHorizontal,
	workflow: Workflow,
	x: X
} as const satisfies Record<string, LucideIcon>;

export type IconName = keyof typeof ICONS;

export const ICON_NAMES: readonly IconName[] = Object.keys(ICONS) as IconName[];
