import type { CSSProperties } from "react";

interface IconProps {
  size?: number;
  className?: string;
  style?: CSSProperties;
}

const base = (size: number) => ({
  width: size,
  height: size,
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 2,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
});

export const IconChevronRight = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <polyline points="9 6 15 12 9 18" />
  </svg>
);

export const IconSearch = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <circle cx="11" cy="11" r="7" />
    <line x1="21" y1="21" x2="16.65" y2="16.65" />
  </svg>
);

export const IconPlus = ({ size = 16, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <line x1="12" y1="5" x2="12" y2="19" />
    <line x1="5" y1="12" x2="19" y2="12" />
  </svg>
);

export const IconUpload = ({ size = 16, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
    <polyline points="17 8 12 3 7 8" />
    <line x1="12" y1="3" x2="12" y2="15" />
  </svg>
);

export const IconUndo = ({ size = 16, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <path d="M9 7 4 12l5 5" />
    <path d="M4 12h9a7 7 0 0 1 7 7" />
  </svg>
);

export const IconHistory = ({ size = 16, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <path d="M3 12a9 9 0 1 0 3-6.7L3 8" />
    <path d="M3 3v5h5" />
    <path d="M12 7v5l3 2" />
  </svg>
);

export const IconRadar = ({ size = 16, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <circle cx="12" cy="12" r="9" />
    <circle cx="12" cy="12" r="4" />
    <line x1="12" y1="12" x2="18.5" y2="5.5" />
    <circle cx="12" cy="12" r="1" fill="currentColor" stroke="none" />
  </svg>
);

export const IconSun = ({ size = 16, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <circle cx="12" cy="12" r="4" />
    <path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41" />
  </svg>
);

export const IconMoon = ({ size = 16, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
  </svg>
);

export const IconSettings = ({ size = 16, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
  </svg>
);

export const IconEditor = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <path d="M12 20h9" />
    <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z" />
  </svg>
);

export const IconGraph = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <circle cx="12" cy="5" r="2" />
    <circle cx="5" cy="19" r="2" />
    <circle cx="19" cy="19" r="2" />
    <line x1="12" y1="7" x2="6" y2="17" />
    <line x1="12" y1="7" x2="18" y2="17" />
    <line x1="7" y1="19" x2="17" y2="19" />
  </svg>
);

export const IconClose = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <line x1="18" y1="6" x2="6" y2="18" />
    <line x1="6" y1="6" x2="18" y2="18" />
  </svg>
);

export const IconChevronLeft = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <polyline points="15 6 9 12 15 18" />
  </svg>
);

export const IconChevronDown = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <polyline points="6 9 12 15 18 9" />
  </svg>
);

export const IconFolder = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
  </svg>
);

export const IconFolderPlus = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <path d="M3 19V5a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2v3" />
    <path d="M16 15v6M13 18h6" />
  </svg>
);

export const IconFile = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
    <polyline points="14 2 14 8 20 8" />
  </svg>
);

export const IconCode = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <polyline points="16 18 22 12 16 6" />
    <polyline points="8 6 2 12 8 18" />
  </svg>
);

export const IconImage = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <rect x="3" y="3" width="18" height="18" rx="2" ry="2" />
    <circle cx="8.5" cy="8.5" r="1.5" />
    <polyline points="21 15 16 10 5 21" />
  </svg>
);

export const IconInbox = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <polyline points="22 12 16 12 14 15 10 15 8 12 2 12" />
    <path d="M5.45 5.11L2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z" />
  </svg>
);

export const IconBook = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20" />
    <path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z" />
  </svg>
);

export const IconBriefcase = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <rect x="2" y="7" width="20" height="14" rx="2" ry="2" />
    <path d="M16 21V5a2 2 0 0 0-2-2h-4a2 2 0 0 0-2 2v16" />
  </svg>
);

export const IconLink = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" />
    <path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" />
  </svg>
);

export const IconTrash = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <polyline points="3 6 5 6 21 6" />
    <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
  </svg>
);

export const IconCopy = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
    <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
  </svg>
);

export const IconRename = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <path d="M12 20h9" />
    <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z" />
  </svg>
);

export const IconCheck = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <polyline points="20 6 9 17 4 12" />
  </svg>
);

export const IconAlert = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
    <line x1="12" y1="9" x2="12" y2="13" />
    <line x1="12" y1="17" x2="12.01" y2="17" />
  </svg>
);

export const IconInfo = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <circle cx="12" cy="12" r="10" />
    <line x1="12" y1="16" x2="12" y2="12" />
    <line x1="12" y1="8" x2="12.01" y2="8" />
  </svg>
);

export const IconRobot = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <rect x="4" y="8" width="16" height="12" rx="2" />
    <circle cx="9" cy="14" r="1" />
    <circle cx="15" cy="14" r="1" />
    <line x1="12" y1="4" x2="12" y2="8" />
    <circle cx="12" cy="3" r="1" />
  </svg>
);

export const IconUser = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2" />
    <circle cx="12" cy="7" r="4" />
  </svg>
);

export const IconTag = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <path d="M20.59 13.41l-7.17 7.17a2 2 0 0 1-2.83 0L2 12V2h10l8.59 8.59a2 2 0 0 1 0 2.82z" />
    <line x1="7" y1="7" x2="7.01" y2="7" />
  </svg>
);

export const IconClock = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <circle cx="12" cy="12" r="10" />
    <polyline points="12 6 12 12 16 14" />
  </svg>
);

export const IconHash = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <line x1="4" y1="9" x2="20" y2="9" />
    <line x1="4" y1="15" x2="20" y2="15" />
    <line x1="10" y1="3" x2="8" y2="21" />
    <line x1="16" y1="3" x2="14" y2="21" />
  </svg>
);

export const IconDownload = ({ size = 14, className, style }: IconProps) => (
  <svg {...base(size)} className={className} style={style}>
    <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
    <polyline points="7 10 12 15 17 10" />
    <line x1="12" y1="15" x2="12" y2="3" />
  </svg>
);

// ===== 对象类型相关 =====
export const objTypeMeta: Record<
  string,
  { label: string; icon: typeof IconBook; color: string }
> = {
  concept: { label: "概念", icon: IconBook, color: "var(--blue)" },
  project: { label: "项目", icon: IconBriefcase, color: "var(--orange)" },
  code: { label: "代码", icon: IconCode, color: "var(--green)" },
  document: { label: "文档", icon: IconFile, color: "var(--yellow)" },
  inbox: { label: "收件箱", icon: IconInbox, color: "var(--pink)" },
  image: { label: "图片", icon: IconImage, color: "var(--purple, var(--accent))" },
};
