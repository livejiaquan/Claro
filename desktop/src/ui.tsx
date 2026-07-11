import { Children, cloneElement, isValidElement, useId, type ReactElement, type ReactNode } from "react";

/* ── 共用小元件與 icons ──────────────────────────────────────────────── */

export function Section({ title, children }: { title?: string; children: ReactNode }) {
  const titleId = useId();
  return (
    <section className="mb-6" aria-labelledby={title ? titleId : undefined}>
      {title && <h2 id={titleId} className="section-title">{title}</h2>}
      <div className="card">{children}</div>
    </section>
  );
}

function isNativeFormControl(child: ReactNode): child is ReactElement<Record<string, unknown>> {
  return (
    isValidElement<Record<string, unknown>>(child) &&
    typeof child.type === "string" &&
    ["input", "select", "textarea"].includes(child.type)
  );
}

export function Row({
  label,
  sub,
  children,
  alignTop,
}: {
  label: string;
  sub?: ReactNode;
  children?: ReactNode;
  alignTop?: boolean;
}) {
  const labelId = useId();
  const childList = Children.toArray(children);
  const hasSingleDirectControl = childList.length === 1 && isNativeFormControl(childList[0]);
  const labelledChildren = childList.map((child) => {
    if (!isNativeFormControl(child)) return child;
    if (child.props["aria-label"] || child.props["aria-labelledby"]) return child;
    return cloneElement(child, { "aria-labelledby": labelId });
  });

  return (
    <div className="row" style={alignTop ? { alignItems: "flex-start" } : undefined}>
      <div className="flex-1 min-w-0">
        <div className="row-label" id={labelId}>{label}</div>
        {sub && <div className="row-sub">{sub}</div>}
      </div>
      <div
        className="row-control"
        role={hasSingleDirectControl ? undefined : "group"}
        aria-labelledby={hasSingleDirectControl ? undefined : labelId}
      >
        {labelledChildren}
      </div>
    </div>
  );
}

export function Kbd({ k }: { k: string }) {
  return <kbd className="keycap">{k}</kbd>;
}

const KEY_SYMBOLS: Record<string, string> = {
  opt: "⌥",
  option: "⌥",
  alt: "⌥",
  optright: "右⌥",
  optleft: "左⌥",
  shift: "⇧",
  cmd: "⌘",
  command: "⌘",
  cmdright: "右⌘",
  cmdleft: "左⌘",
  ctrl: "⌃",
  control: "⌃",
  fn: "fn",
  space: "␣",
};

/** 渲染快捷鍵組合（handy-keys 字串格式，如 "Opt+Shift+C"、"CmdRight"） */
export function Hotkey({ combo = "Opt+Shift+C" }: { combo?: string }) {
  const parts = combo.split("+").map((p) => p.trim()).filter(Boolean);
  return (
    <span className="inline-flex gap-1 align-middle mx-0.5">
      {parts.map((p, i) => (
        <Kbd key={i} k={KEY_SYMBOLS[p.toLowerCase()] ?? p.toUpperCase()} />
      ))}
    </span>
  );
}

export function LevelBar({ level, active }: { level: number; active: boolean }) {
  const pct = active ? Math.min(100, level * 400) : 0;
  const stateText = !active ? "麥克風測試未啟動" : level > 0.01 ? "收音正常" : "音量偏低";
  return (
    <>
      <div
        className="level-track"
        role="meter"
        aria-label="麥克風音量"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(pct)}
        aria-valuetext={stateText}
      >
        {/* 靜音門檻 0.01 → 4% */}
        <div className="level-thresh" style={{ left: "4%" }} />
        <div
          className={`level-fill ${level > 0.01 ? "hot" : ""}`}
          style={{ width: `${active ? Math.max(pct, 1.5) : 0}%` }}
        />
      </div>
      <span className="sr-only" role="status" aria-live="polite">{stateText}</span>
    </>
  );
}

export function StatCard({
  icon,
  value,
  unit,
  label,
}: {
  icon: ReactNode;
  value: string;
  unit?: string;
  label: string;
}) {
  return (
    <div className="card px-4 py-4">
      <div className="stat-icon">{icon}</div>
      <div>
        <span className="stat-num">{value}</span>
        {unit && <span className="stat-unit">{unit}</span>}
      </div>
      <div className="stat-label">{label}</div>
    </div>
  );
}

/* icons：1.6px stroke 手繪風 */
const S = { fill: "none", stroke: "currentColor", strokeWidth: 1.6, strokeLinecap: "round", strokeLinejoin: "round" } as const;

export const IconHome = () => (
  <svg viewBox="0 0 20 20" {...S}>
    <path d="M3.5 8.5 10 3l6.5 5.5V16a1 1 0 0 1-1 1h-3.6v-4.4H8.1V17H4.5a1 1 0 0 1-1-1V8.5Z" />
  </svg>
);
export const IconHistory = () => (
  <svg viewBox="0 0 20 20" {...S}>
    <circle cx="10" cy="10" r="6.7" />
    <path d="M10 6.6V10l2.4 1.6" />
  </svg>
);
export const IconSetup = () => (
  <svg viewBox="0 0 20 20" {...S}>
    <path d="M5 4.2h10a1.3 1.3 0 0 1 1.3 1.3v9a1.3 1.3 0 0 1-1.3 1.3H5a1.3 1.3 0 0 1-1.3-1.3v-9A1.3 1.3 0 0 1 5 4.2Z" />
    <path d="m6.4 8.1 1.2 1.2 2-2.1M11.4 8.3h2.3M6.4 12.4h7.3" />
  </svg>
);
export const IconSettings = () => (
  <svg viewBox="0 0 20 20" {...S}>
    <path d="M8.4 3.3h3.2l.5 1.9 1.7 1 1.9-.6 1.6 2.7-1.5 1.4v1.9l1.5 1.4-1.6 2.7-1.9-.6-1.7 1-.5 1.9H8.4l-.5-1.9-1.7-1-1.9.6-1.6-2.7 1.5-1.4V9.7L2.7 8.3l1.6-2.7 1.9.6 1.7-1 .5-1.9Z" transform="scale(0.92) translate(0.9,0.9)" />
    <circle cx="10" cy="10" r="2.4" />
  </svg>
);
export const IconMic = () => (
  <svg viewBox="0 0 20 20" {...S}>
    <rect x="7.2" y="2.8" width="5.6" height="9" rx="2.8" />
    <path d="M4.6 9.6a5.4 5.4 0 0 0 10.8 0M10 15v2.4" />
  </svg>
);
export const IconChars = () => (
  <svg viewBox="0 0 20 20" {...S}>
    <path d="M4 5.2h12M10 5.2V16M6.5 16h7" />
  </svg>
);
export const IconSpeed = () => (
  <svg viewBox="0 0 20 20" {...S}>
    <path d="M3.6 12.8a6.8 6.8 0 1 1 12.8 0" />
    <path d="M10 12.5l3.2-3.6" />
    <circle cx="10" cy="12.8" r="1.3" />
  </svg>
);
export const IconStack = () => (
  <svg viewBox="0 0 20 20" {...S}>
    <path d="m10 3 7 3.6L10 10 3 6.6 10 3ZM3 10.2l7 3.4 7-3.4M3 13.8l7 3.4 7-3.4" />
  </svg>
);
export const IconCopy = () => (
  <svg viewBox="0 0 20 20" width="15" height="15" {...S}>
    <rect x="7" y="7" width="9" height="9.5" rx="2" />
    <path d="M13 7V5.5A1.5 1.5 0 0 0 11.5 4H5.6A1.6 1.6 0 0 0 4 5.6v6A1.4 1.4 0 0 0 5.4 13H7" />
  </svg>
);
