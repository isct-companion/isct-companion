import type { Quarter, TermId } from "./model";
import { JST, tokyoDate } from "./civil-date";
export { JST } from "./civil-date";
export function termForDate(date: Date): TermId {
  const [year, month] = tokyoDate(date).split("-").map(Number);
  const academic = month < 4 ? year - 1 : year;
  const q: Quarter =
    month >= 4 && month < 6
      ? "1Q"
      : month >= 6 && month < 10
        ? "2Q"
        : month >= 10 && month < 12
          ? "3Q"
          : "4Q";
  return `${academic}-${q}`;
}
