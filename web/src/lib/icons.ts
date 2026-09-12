// The console's icon set: the Lucide glyphs this client actually draws, named
// once so a destination or a component asks for a name rather than importing a
// component of its own.
//
// Deep imports rather than the package barrel, because the barrel re-exports
// every icon Lucide ships and this console draws about forty of them. The map below is total over `IconName`, so a name added to the
// union without an import stops the type check instead of rendering nothing.

import type { LucideIcon } from '@lucide/svelte';

import Activity from '@lucide/svelte/icons/activity';
import ArrowLeft from '@lucide/svelte/icons/arrow-left';
import ArrowRightLeft from '@lucide/svelte/icons/arrow-right-left';
import Bell from '@lucide/svelte/icons/bell';
import BookOpen from '@lucide/svelte/icons/book-open';
import Building2 from '@lucide/svelte/icons/building-2';
import CalendarClock from '@lucide/svelte/icons/calendar-clock';
import ChartLine from '@lucide/svelte/icons/chart-line';
import Check from '@lucide/svelte/icons/check';
import ChevronDown from '@lucide/svelte/icons/chevron-down';
import CircleAlert from '@lucide/svelte/icons/circle-alert';
import CircleCheck from '@lucide/svelte/icons/circle-check';
import CirclePlus from '@lucide/svelte/icons/circle-plus';
import CircleQuestionMark from '@lucide/svelte/icons/circle-question-mark';
import CircleUser from '@lucide/svelte/icons/circle-user';
import CircleX from '@lucide/svelte/icons/circle-x';
import Copy from '@lucide/svelte/icons/copy';
import CreditCard from '@lucide/svelte/icons/credit-card';
import Download from '@lucide/svelte/icons/download';
import EllipsisVertical from '@lucide/svelte/icons/ellipsis-vertical';
import Eye from '@lucide/svelte/icons/eye';
import FileDown from '@lucide/svelte/icons/file-down';
import HeartPulse from '@lucide/svelte/icons/heart-pulse';
import Image from '@lucide/svelte/icons/image';
import Info from '@lucide/svelte/icons/info';
import Laptop from '@lucide/svelte/icons/laptop';
import Layers from '@lucide/svelte/icons/layers';
import LayoutDashboard from '@lucide/svelte/icons/layout-dashboard';
import LayoutList from '@lucide/svelte/icons/layout-list';
import LayoutTemplate from '@lucide/svelte/icons/layout-template';
import LibraryBig from '@lucide/svelte/icons/library-big';
import LogOut from '@lucide/svelte/icons/log-out';
import Minus from '@lucide/svelte/icons/minus';
import Monitor from '@lucide/svelte/icons/monitor';
import Package from '@lucide/svelte/icons/package';
import Pause from '@lucide/svelte/icons/pause';
import Plus from '@lucide/svelte/icons/plus';
import RefreshCw from '@lucide/svelte/icons/refresh-cw';
import Search from '@lucide/svelte/icons/search';
import Share2 from '@lucide/svelte/icons/share-2';
import ShoppingBag from '@lucide/svelte/icons/shopping-bag';
import ShieldCheck from '@lucide/svelte/icons/shield-check';
import SlidersHorizontal from '@lucide/svelte/icons/sliders-horizontal';
import Smartphone from '@lucide/svelte/icons/smartphone';
import Store from '@lucide/svelte/icons/store';
import Tag from '@lucide/svelte/icons/tag';
import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
import Users from '@lucide/svelte/icons/users';
import WavesHorizontal from '@lucide/svelte/icons/waves-horizontal';
import Workflow from '@lucide/svelte/icons/workflow';
import X from '@lucide/svelte/icons/x';

export const ICONS = {
	activity: Activity,
	'arrow-left': ArrowLeft,
	'arrow-right-left': ArrowRightLeft,
	bell: Bell,
	'book-open': BookOpen,
	'building-2': Building2,
	'calendar-clock': CalendarClock,
	'chart-line': ChartLine,
	check: Check,
	'chevron-down': ChevronDown,
	'circle-alert': CircleAlert,
	'circle-check': CircleCheck,
	'circle-plus': CirclePlus,
	'circle-question-mark': CircleQuestionMark,
	'circle-user': CircleUser,
	'circle-x': CircleX,
	copy: Copy,
	'credit-card': CreditCard,
	download: Download,
	'ellipsis-vertical': EllipsisVertical,
	eye: Eye,
	'file-down': FileDown,
	'heart-pulse': HeartPulse,
	image: Image,
	info: Info,
	laptop: Laptop,
	layers: Layers,
	'layout-dashboard': LayoutDashboard,
	'layout-list': LayoutList,
	'layout-template': LayoutTemplate,
	'library-big': LibraryBig,
	'log-out': LogOut,
	minus: Minus,
	monitor: Monitor,
	package: Package,
	pause: Pause,
	plus: Plus,
	'refresh-cw': RefreshCw,
	search: Search,
	'share-2': Share2,
	'shopping-bag': ShoppingBag,
	'shield-check': ShieldCheck,
	'sliders-horizontal': SlidersHorizontal,
	smartphone: Smartphone,
	store: Store,
	tag: Tag,
	'triangle-alert': TriangleAlert,
	users: Users,
	'waves-horizontal': WavesHorizontal,
	workflow: Workflow,
	x: X
} as const satisfies Record<string, LucideIcon>;

export type IconName = keyof typeof ICONS;

export const ICON_NAMES: readonly IconName[] = Object.keys(ICONS) as IconName[];
