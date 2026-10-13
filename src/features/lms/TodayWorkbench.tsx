import "./lms.css";
import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import type { CourseEntry } from "./LmsWorkspace";

/** Keep two recent readers briefly; preserve Today and evict idle course DOMs. */
export function TodayWorkbench({
  entry,
  active = true,
  renderWorkspace,
  children,
}: {
  entry: CourseEntry | null;
  active?: boolean;
  renderWorkspace: (entry: CourseEntry, active: boolean) => ReactNode;
  children: (compact: boolean) => ReactNode;
}) {
  const [recent, setRecent] = useState<CourseEntry[]>([]);
  const [sleeping, setSleeping] = useState(false);
  const [closing, setClosing] = useState<CourseEntry | null>(null);
  const lastEntry = useRef<CourseEntry | null>(null);
  useLayoutEffect(() => {
    if (entry) { lastEntry.current = entry; setClosing(null); return; }
    if (!lastEntry.current) return;
    setClosing(lastEntry.current);
    const delay = typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : 300; // matches --lms-slow
    const timer = setTimeout(() => { setClosing(null); lastEntry.current = null; }, delay);
    return () => clearTimeout(timer);
  }, [entry]);
  const shownEntry = entry || closing;
  const todayPane = useRef<HTMLDivElement>(null);
  const positions = useRef({ overview: 0, navigation: 0 });
  const compact = Boolean(entry);
  useLayoutEffect(() => {
    if (todayPane.current)
      todayPane.current.scrollTop =
        positions.current[compact ? "navigation" : "overview"];
  }, [compact]);
  const retained =
    sleeping && !active
      ? []
      : entry
        ? [
            ...recent.filter((item) => item.courseId !== entry.courseId),
            entry,
          ].slice(-2)
        : recent;
  useEffect(() => {
    if (entry)
      setRecent((previous) =>
        [
          ...previous.filter((item) => item.courseId !== entry.courseId),
          entry,
        ].slice(-2),
      );
  }, [entry]);
  useEffect(() => {
    if (active) setSleeping(false);
    const timer = setTimeout(() => {
      if (!active) setSleeping(true);
      setRecent((previous) =>
        active && entry
          ? previous.filter((item) => item.courseId === entry.courseId)
          : [],
      );
    }, 45000);
    return () => clearTimeout(timer);
  }, [active, entry]);
  return (
    <div className={`todayWorkbench${entry ? " working" : ""}${closing && !entry ? " leaving" : ""}`}>
      <div
        className={`todayRetained${entry ? " compact" : ""}`}
        ref={todayPane}
        onScroll={(event) => {
          positions.current[compact ? "navigation" : "overview"] =
            event.currentTarget.scrollTop;
        }}
      >
        {children(compact)}
      </div>
      <div className="todayWorkbenchContent" hidden={!shownEntry} inert={!entry}>
        {retained.map((item) => (
          <div
            className="todayCourseSlot"
            key={item.courseId}
            hidden={shownEntry?.courseId !== item.courseId}
          >
            {renderWorkspace(item, active && entry?.courseId === item.courseId)}
          </div>
        ))}
      </div>
    </div>
  );
}
