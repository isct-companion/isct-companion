import { useEffect, useRef, useState } from "react";

// The app icon as a toy, the same one isctcomp.app shows: a timetable board
// balanced on its stand. Tap a cell to add a class and the board leans toward
// the heavier side; drag it, or hover over it with a mouse, to rock it. Until
// someone touches it, one cell glows and the board nudges itself now and then.
// The drawing is resources/design/icon.svg, split into the board that rocks
// around the stand's tip and the line it rests on.
//
// The swing is a small damped spring stepped per frame. The app has no
// animation library, and one page does not justify adding one for ~40 lines.

/** The icon's skew, applied to the board and to the line alike. */
const SKEW = "translate(-31,-37.9) matrix(1,0.08749,0,1,0,-54.86)";
/** The stand's tip after that skew: the board rocks around it. */
const PIVOT_X = 651;
const PIVOT_Y = 871.9;
/** The same tip before the skew, where the cells' `x` are measured. */
const PIVOT_INNER_X = 682;
const VIEW_X = 170;
const VIEW_W = 920;
const LEAN = 0.022; // degrees per unit of torque
const MAX_LEAN = 12;
const HOVER_TILT = 7; // degrees toward the mouse at the edge of the toy
const MAX_DRAG = 16;
// Under-damped on purpose, so the board swings back and forth before resting.
const STIFFNESS = 170;
const DAMPING = 4.5;
const NUDGES = 3; // self-nudges while nobody has touched it
const STORAGE_KEY = "isct-companion-about-toy";

const SILHOUETTE =
  "M264.7,679 Q264,633 303.6,609.6 L728,358.6 Q790,322 854.5,354 L1010.8,431.6 Q1052,452 1051.1,498 L1047.9,661 Q1047,707 1002.1,717.2 L312.9,873.8 Q268,884 267.3,838 Z";

/** The twelve cells, two rows of six; `x` is each column's centre. */
const CELLS = [
  [
    379.8,
    "M335,683 Q335,669 348.6,665.8 L411,651 Q424.6,647.7 424.6,661.7 L424.6,689.7 Q424.6,703.7 411,706.8 L348.6,721 Q335,724.1 335,710.1 Z",
  ],
  [
    490.9,
    "M446.1,656.7 Q446.1,642.7 459.7,639.4 L522.1,624.6 Q535.7,621.4 535.7,635.4 L535.7,664.3 Q535.7,678.3 522,681.4 L459.7,695.7 Q446.1,698.8 446.1,684.8 Z",
  ],
  [
    602.0,
    "M557.2,630.3 Q557.2,616.3 570.8,613.1 L633.1,598.3 Q646.8,595 646.8,609 L646.8,638.9 Q646.8,652.9 633.1,656.1 L570.8,670.3 Q557.2,673.4 557.2,659.4 Z",
  ],
  [
    713.0,
    "M668.2,604 Q668.2,590 681.9,586.7 L744.2,571.9 Q757.8,568.7 757.8,582.7 L757.8,613.6 Q757.8,627.6 744.2,630.7 L681.9,644.9 Q668.2,648.1 668.2,634.1 Z",
  ],
  [
    824.1,
    "M779.3,577.6 Q779.3,563.6 792.9,560.4 L855.3,545.6 Q868.9,542.3 868.9,556.3 L868.9,588.2 Q868.9,602.2 855.3,605.3 L793,619.6 Q779.3,622.7 779.3,608.7 Z",
  ],
  [
    935.2,
    "M890.4,551.3 Q890.4,537.3 904,534 L966.4,519.2 Q980,516 980,530 L980,562.9 Q980,576.9 966.4,580 L904,594.2 Q890.4,597.3 890.4,583.3 Z",
  ],
  [
    379.8,
    "M335,762.9 Q335,748.9 348.7,745.8 L411,731.8 Q424.6,728.8 424.6,742.8 L424.6,770.7 Q424.6,784.7 410.9,787.6 L348.7,801.1 Q335,804 335,790 Z",
  ],
  [
    490.9,
    "M446.1,737.9 Q446.1,723.9 459.7,720.9 L522,706.9 Q535.7,703.8 535.7,717.8 L535.7,746.7 Q535.7,760.7 522,763.7 L459.8,777.1 Q446.1,780.1 446.1,766.1 Z",
  ],
  [
    602.0,
    "M557.2,713 Q557.2,699 570.8,696 L633.1,682 Q646.8,678.9 646.8,692.9 L646.8,722.8 Q646.8,736.8 633.1,739.8 L570.8,753.2 Q557.2,756.1 557.2,742.1 Z",
  ],
  [
    713.0,
    "M668.2,688.1 Q668.2,674.1 681.9,671 L744.2,657 Q757.8,654 757.8,668 L757.8,698.9 Q757.8,712.9 744.2,715.8 L681.9,729.2 Q668.2,732.2 668.2,718.2 Z",
  ],
  [
    824.1,
    "M779.3,663.2 Q779.3,649.2 793,646.1 L855.3,632.1 Q868.9,629.1 868.9,643.1 L868.9,674.9 Q868.9,688.9 855.2,691.9 L793,705.3 Q779.3,708.3 779.3,694.3 Z",
  ],
  [
    935.2,
    "M890.4,638.2 Q890.4,624.2 904,621.2 L966.3,607.2 Q980,604.1 980,618.1 L980,651 Q980,665 966.3,667.9 L904.1,681.4 Q890.4,684.3 890.4,670.3 Z",
  ],
] as const;
/** The cells lit in the icon as shipped: its rest position. */
const SHIPPED = [2, 10];
/** The empty cell that glows until the first touch. */
const HINT = 4;

const torqueOf = (on: readonly number[]) =>
  on.reduce((sum, i) => sum + CELLS[i][0] - PIVOT_INNER_X, 0);
const REST_TORQUE = torqueOf(SHIPPED);
const leanOf = (on: readonly number[]) =>
  Math.max(-MAX_LEAN, Math.min(MAX_LEAN, (torqueOf(on) - REST_TORQUE) * LEAN));

/** The lit cells from the last visit; a convenience for this device only, so
 * a missing or unreadable entry is the icon as shipped. */
function storedCells(): number[] {
  try {
    const raw = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "null");
    if (
      Array.isArray(raw) &&
      raw.every((i) => Number.isInteger(i) && i >= 0 && i < CELLS.length)
    )
      return raw as number[];
  } catch {
    // Unreadable storage: start from the icon as shipped.
  }
  return SHIPPED;
}

export function IconToy({ className = "" }: { className?: string }) {
  const [on, setOn] = useState(storedCells);
  const [idle, setIdle] = useState(true);
  const [dragging, setDragging] = useState(false);
  const figure = useRef<HTMLElement>(null);
  const svg = useRef<SVGSVGElement>(null);
  const board = useRef<SVGGElement>(null);
  /** The motion lives outside React state: it changes every frame. */
  const motion = useRef({
    angle: 0,
    velocity: 0,
    lean: leanOf(on),
    hover: 0,
    frame: 0,
    last: 0,
    drag: null as null | {
      id: number;
      x: number;
      from: number;
      moved: boolean;
      t: number;
    },
    still: false,
    run: () => {},
    push: (_velocity: number) => {},
    stopNudges: () => {},
  });

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(on));
    } catch {
      // Without storage the board simply starts as shipped next time.
    }
    motion.current.lean = leanOf(on);
    motion.current.run();
  }, [on]);

  useEffect(() => {
    const m = motion.current;
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)");
    m.still = reduced.matches;
    const target = () => m.lean + m.hover;
    const draw = () =>
      board.current?.setAttribute(
        "transform",
        `rotate(${m.angle.toFixed(2)} ${PIVOT_X} ${PIVOT_Y})`,
      );
    const step = (now: number) => {
      const dt = Math.min(0.032, (now - m.last) / 1000 || 0.016);
      m.last = now;
      if (!m.drag?.moved) {
        m.velocity +=
          (-STIFFNESS * (m.angle - target()) - DAMPING * m.velocity) * dt;
        m.angle += m.velocity * dt;
      }
      draw();
      if (
        m.drag?.moved ||
        Math.abs(m.angle - target()) > 0.02 ||
        Math.abs(m.velocity) > 0.05
      ) {
        m.frame = requestAnimationFrame(step);
      } else {
        m.angle = target();
        m.velocity = 0;
        draw();
        m.frame = 0;
      }
    };
    m.run = () => {
      if (m.still) {
        m.angle = target();
        m.velocity = 0;
        draw();
        return;
      }
      if (!m.frame) {
        m.last = performance.now();
        m.frame = requestAnimationFrame(step);
      }
    };
    m.push = (velocity) => {
      m.velocity += velocity;
      m.run();
    };

    // A swing as the page appears, then a few nudges until the first touch.
    let nudges = 0;
    let timer = 0;
    const nudge = () => {
      if (m.still || nudges >= NUDGES) return;
      timer = window.setTimeout(() => {
        nudges++;
        m.push(nudges % 2 ? 45 : -45);
        nudge();
      }, 4500);
    };
    m.angle = m.still ? target() : target() - 6;
    m.run();
    nudge();
    m.stopNudges = () => window.clearTimeout(timer);
    return () => {
      m.stopNudges = () => {};
      window.clearTimeout(timer);
      cancelAnimationFrame(m.frame);
      m.frame = 0;
      m.drag = null;
      m.hover = 0;
    };
  }, []);

  const engage = () => {
    if (!idle) return;
    setIdle(false);
    motion.current.stopNudges();
  };

  /** A position in the drawing's units, measured inside the toy only. */
  const svgX = (clientX: number) => {
    const box = svg.current!.getBoundingClientRect();
    return VIEW_X + (clientX - box.left) * (VIEW_W / box.width);
  };

  const endDrag = () => {
    const m = motion.current;
    if (!m.drag) return;
    m.drag = null;
    setDragging(false);
    m.run();
  };

  return (
    <figure
      ref={figure}
      className={`iconToy${idle ? " idle" : ""}${dragging ? " dragging" : ""} ${className}`}
      aria-hidden
      onPointerEnter={engage}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        engage();
        const m = motion.current;
        m.drag = {
          id: e.pointerId,
          x: svgX(e.clientX),
          from: m.angle,
          moved: false,
          t: performance.now(),
        };
      }}
      onPointerMove={(e) => {
        const m = motion.current;
        const drag = m.drag;
        if (drag && e.pointerId === drag.id) {
          const dx = svgX(e.clientX) - drag.x;
          if (!drag.moved && Math.abs(dx) < 12) return;
          if (!drag.moved) {
            drag.moved = true;
            setDragging(true);
          }
          if (m.still) return;
          const next = Math.max(
            -MAX_DRAG,
            Math.min(MAX_DRAG, drag.from + dx * 0.06),
          );
          const now = performance.now();
          m.velocity =
            ((next - m.angle) / Math.max(0.008, (now - drag.t) / 1000)) * 0.6;
          drag.t = now;
          m.angle = next;
          m.run();
          return;
        }
        // A mouse hovering over the toy tips the board toward it.
        if (e.pointerType !== "mouse") return;
        const box = figure.current!.getBoundingClientRect();
        const x = ((e.clientX - box.left) / box.width) * 2 - 1;
        m.hover = Math.max(-1, Math.min(1, x)) * HOVER_TILT;
        m.run();
      }}
      onPointerUp={(e) => {
        const m = motion.current;
        if (!m.drag || e.pointerId !== m.drag.id) return;
        if (m.drag.moved) return endDrag();
        m.drag = null;
        const cell = (e.target as Element).closest<SVGPathElement>(
          "[data-cell]",
        );
        if (cell) {
          const index = Number(cell.dataset.cell);
          setOn((current) =>
            current.includes(index)
              ? current.filter((i) => i !== index)
              : [...current, index],
          );
        } else if (svg.current?.contains(e.target as Node)) {
          // A tap on the board pushes the side that was tapped.
          m.push((svgX(e.clientX) > PIVOT_X ? 1 : -1) * 70);
        }
      }}
      // Leaving the toy ends tracking: the board lets go and swings back.
      onPointerLeave={(e) => {
        const m = motion.current;
        m.hover = 0;
        if (m.drag && e.pointerId === m.drag.id) endDrag();
        else m.run();
      }}
      onPointerCancel={() => {
        motion.current.hover = 0;
        endDrag();
      }}
    >
      <svg ref={svg} viewBox="170 250 920 780">
        <defs>
          <clipPath id="iconToy-sil">
            <path d={SILHOUETTE} />
          </clipPath>
          <clipPath id="iconToy-in">
            <path d="M282.6,671.2 Q282.2,643.2 306.3,628.9 L744.3,370 Q790.8,342.5 839.1,366.5 L1008.7,450.6 Q1033.8,463.1 1033.2,491 L1029.8,664.6 Q1029.3,692.6 1002,698.8 L312.9,855.3 Q285.6,861.5 285.2,833.5 Z" />
          </clipPath>
          <clipPath id="iconToy-half">
            <path d="M-3634.5,1528.5 L4950.5,-443.5 L4055,-4341.9 L-4529.9,-2370 Z" />
          </clipPath>
        </defs>
        <g ref={board}>
          <g transform={SKEW}>
            <path
              className="iconToyStand"
              d="M522,832 L682,905 L795,770"
              fill="none"
              strokeWidth="34"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
            <path className="iconToyPanel" d={SILHOUETTE} />
            <path
              className="iconToyTop"
              d={SILHOUETTE}
              clipPath="url(#iconToy-half)"
            />
            <g clipPath="url(#iconToy-in)">
              {CELLS.map(([, d], index) => (
                <path
                  key={index}
                  data-cell={index}
                  className={`iconToyCell${on.includes(index) ? " on" : ""}${index === HINT ? " hint" : ""}`}
                  d={d}
                />
              ))}
            </g>
            <line
              className="iconToyInk"
              x1="-28.4"
              y1="700.2"
              x2="1344.4"
              y2="384.8"
              strokeWidth="30"
              clipPath="url(#iconToy-sil)"
            />
            <path
              className="iconToyOutline"
              d="M246.9,686.8 Q245.8,622.8 300.9,590.3 L711.8,347.3 Q789.2,301.5 869.9,341.5 L1012.9,412.5 Q1070.2,440.9 1069,504.9 L1066,657.4 Q1064.7,721.4 1002.3,735.6 L312.8,892.3 Q250.4,906.5 249.3,842.5 Z M282.6,671.2 Q282.2,643.2 306.3,628.9 L744.3,370 Q790.8,342.5 839.1,366.5 L1008.7,450.6 Q1033.8,463.1 1033.2,491 L1029.8,664.6 Q1029.3,692.6 1002,698.8 L312.9,855.3 Q285.6,861.5 285.2,833.5 Z"
              fillRule="evenodd"
            />
          </g>
        </g>
        <g transform={SKEW}>
          <path
            className="iconToyStand"
            d="M324,995 L1045,832"
            strokeWidth="48"
            strokeLinecap="round"
          />
        </g>
      </svg>
    </figure>
  );
}
