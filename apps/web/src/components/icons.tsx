/**
 * Iconos SVG inline — set consistente del rediseño V1.
 *
 * - viewBox 16×16
 * - stroke="currentColor", strokeWidth=1.5, linecap/linejoin="round"
 * - El color lo controla la clase CSS del padre.
 * - El tamaño visible lo controla CSS (los SVG no traen width/height fijos).
 */

import type { SVGProps } from "react";

type IconProps = SVGProps<SVGSVGElement>;

function IconBase({ children, ...rest }: { children: React.ReactNode } & IconProps) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
      {...rest}
    >
      {children}
    </svg>
  );
}

export function PlusIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path d="M8 3v10M3 8h10" />
    </IconBase>
  );
}

export function RowEditIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path d="M11.2 2.6l2.2 2.2-8 8H3.2v-2.2l8-8z" />
    </IconBase>
  );
}

export function RowTrashIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path d="M3 4.5h10" />
      <path d="M5.5 4.5V3a1 1 0 011-1h3a1 1 0 011 1v1.5" />
      <path d="M4.5 4.5l.8 8.5a1 1 0 001 .9h3.4a1 1 0 001-.9l.8-8.5" />
    </IconBase>
  );
}

export function GearIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <circle cx={8} cy={8} r={1.8} />
      <path d="M13.4 9.6a1 1 0 00.2 1.1l.04.04a1.2 1.2 0 11-1.7 1.7l-.04-.04a1 1 0 00-1.1-.2 1 1 0 00-.6.9V13a1.2 1.2 0 11-2.4 0v-.05a1 1 0 00-.6-.9 1 1 0 00-1.1.2l-.04.04a1.2 1.2 0 11-1.7-1.7l.04-.04a1 1 0 00.2-1.1 1 1 0 00-.9-.6H3a1.2 1.2 0 110-2.4h.05a1 1 0 00.9-.6 1 1 0 00-.2-1.1l-.04-.04a1.2 1.2 0 111.7-1.7l.04.04a1 1 0 001.1.2h.05a1 1 0 00.6-.9V3a1.2 1.2 0 112.4 0v.05a1 1 0 00.6.9 1 1 0 001.1-.2l.04-.04a1.2 1.2 0 111.7 1.7l-.04.04a1 1 0 00-.2 1.1v.05a1 1 0 00.9.6H13a1.2 1.2 0 110 2.4h-.05a1 1 0 00-.9.6z" />
    </IconBase>
  );
}

/* ───────────────── Set extendido del rediseño ───────────────── */

export function XIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path d="M3.5 3.5l9 9M12.5 3.5l-9 9" />
    </IconBase>
  );
}

export function CheckIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path d="M3 8.4l3.2 3.1L13 4.6" />
    </IconBase>
  );
}

export function MoreIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <circle cx={3.5} cy={8} r={1.1} fill="currentColor" stroke="none" />
      <circle cx={8} cy={8} r={1.1} fill="currentColor" stroke="none" />
      <circle cx={12.5} cy={8} r={1.1} fill="currentColor" stroke="none" />
    </IconBase>
  );
}

export function ChevronIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path d="M6 3.5L10.5 8 6 12.5" />
    </IconBase>
  );
}

export function ChevronLeftIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path d="M10 3.5L5.5 8 10 12.5" />
    </IconBase>
  );
}

export function ChevronDownIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path d="M3.5 6L8 10.5 12.5 6" />
    </IconBase>
  );
}

export function MenuIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path d="M3 5h10M3 8h10M3 11h10" />
    </IconBase>
  );
}

export function UserIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <circle cx={8} cy={6} r={2.4} />
      <path d="M3 13c1-2.4 3-3.4 5-3.4s4 1 5 3.4" />
    </IconBase>
  );
}

export function DragIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <circle cx={6} cy={4} r={0.9} fill="currentColor" stroke="none" />
      <circle cx={10} cy={4} r={0.9} fill="currentColor" stroke="none" />
      <circle cx={6} cy={8} r={0.9} fill="currentColor" stroke="none" />
      <circle cx={10} cy={8} r={0.9} fill="currentColor" stroke="none" />
      <circle cx={6} cy={12} r={0.9} fill="currentColor" stroke="none" />
      <circle cx={10} cy={12} r={0.9} fill="currentColor" stroke="none" />
    </IconBase>
  );
}

export function DownloadIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path d="M8 2.5v7.6m0 0l-2.8-2.8m2.8 2.8l2.8-2.8M3 13.5h10" />
    </IconBase>
  );
}

export function CalendarIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <rect x={2.5} y={3.5} width={11} height={10} rx={1.5} />
      <path d="M5 2v3M11 2v3M2.5 6.5h11" />
    </IconBase>
  );
}

export function FilterIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path d="M2.5 4h11M5 8h6M7 12h2" />
    </IconBase>
  );
}

export function SortIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path d="M5 3v9m0 0l-2-2m2 2l2-2M11 13V4m0 0l-2 2m2-2l2 2" />
    </IconBase>
  );
}

export function LinkIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path d="M7 9l2-2" />
      <path d="M9.5 6.5l1-1a2.1 2.1 0 013 3l-1.5 1.5" />
      <path d="M6.5 9.5l-1 1a2.1 2.1 0 01-3-3L4 6" />
    </IconBase>
  );
}

export function RefreshIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path d="M3 8a5 5 0 018.5-3.5L13 6" />
      <path d="M13 2.5V6h-3.5" />
      <path d="M13 8a5 5 0 01-8.5 3.5L3 10" />
      <path d="M3 13.5V10h3.5" />
    </IconBase>
  );
}

export function EyeIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path d="M1.5 8s2.4-4.5 6.5-4.5S14.5 8 14.5 8 12.1 12.5 8 12.5 1.5 8 1.5 8z" />
      <circle cx={8} cy={8} r={1.8} />
    </IconBase>
  );
}

export function SearchIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <circle cx={7} cy={7} r={3.6} />
      <path d="M9.7 9.7l3 3" />
    </IconBase>
  );
}

export function ArrowUpIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path d="M8 13V3m-3 3l3-3 3 3" />
    </IconBase>
  );
}

export function ArrowDownIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path d="M8 3v10m-3-3l3 3 3-3" />
    </IconBase>
  );
}

export function DuplicateIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <rect x={3} y={5} width={7} height={8} rx={1.2} />
      <path d="M5.5 5V4a1 1 0 011-1h5a1 1 0 011 1v6a1 1 0 01-1 1H10" />
    </IconBase>
  );
}

export function UploadIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path d="M8 10.6V2.9m0 0L5.2 5.7M8 2.9l2.8 2.8M3 13h10" />
    </IconBase>
  );
}
