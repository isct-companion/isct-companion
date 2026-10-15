import { useEffect, useId, useRef, useState } from "react";
import type { TermId } from "../../lib/model";
import { t } from "../../i18n";

/** One expanded year; hovering changes navigation, never the displayed schedule. */
export function YearQuarterPicker({ terms, selected, current, onSelect, onAdd }: {
  terms: TermId[];
  selected: TermId;
  current: TermId;
  onSelect: (term: TermId) => void;
  onAdd?: () => void;
}) {
  const years = [...new Set(terms.map(term => term.split("-")[0]))].sort();
  const [expanded, setExpanded] = useState(current.split("-")[0]);
  const previous = useRef(selected);
  const id = useId();
  useEffect(() => {
    if (selected !== previous.current) setExpanded(selected.split("-")[0]);
    previous.current = selected;
  }, [selected]);
  return <div className="yearQuarterPicker" aria-label={t("Quarter")}>
    {years.map(year => {
      const open = expanded === year;
      const selectedYear = selected.startsWith(`${year}-`);
      const groupTerms = terms.filter(term => term.startsWith(`${year}-`));
      return <div key={year} className={`yearQuarterGroup${open ? " expanded" : ""}${selectedYear ? " selectedYear" : ""}`}
        onPointerEnter={event => { if (event.pointerType === "mouse") setExpanded(year); }}>
        <button className="yearQuarterYear" type="button" aria-expanded={open} aria-controls={`${id}-${year}`} onClick={() => setExpanded(year)}
          onKeyDown={event => { if (event.key === "ArrowRight" || event.key === "ArrowDown") { event.preventDefault(); setExpanded(year); } }}>
          <span>{year}</span>{selectedYear && !open && <small>{selected.split("-")[1]}</small>}
        </button>
        <div className="yearQuarterDrawer" id={`${id}-${year}`} aria-hidden={!open} inert={!open}>
          <div className="yearQuarterDrawerInner">
            <span className="yearQuarterDivider" aria-hidden="true" />
            <div className="yearQuarterQuarters" role="tablist" aria-label={`${year} · ${t("Quarter")}`}>
              {groupTerms.map((term, index) => <span className="yearQuarterStep" key={term}>
                {index > 0 && <span className="yearQuarterChevron" aria-hidden="true">›</span>}
                <button type="button" role="tab" aria-label={term.replace("-", " ")} aria-selected={term === selected}
                  className={term === selected ? "active" : ""} onClick={() => onSelect(term)}>
                  {term.split("-")[1]}{term === current && <i title={t("Current quarter")} />}
                </button>
              </span>)}
            </div>
          </div>
        </div>
      </div>;
    })}
    {onAdd && <button type="button" className="yearQuarterAdd" aria-label={t("Add quarter")} onClick={onAdd}>＋</button>}
  </div>;
}
