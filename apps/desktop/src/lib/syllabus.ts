import type { Meeting } from "./model";
import type { SyllabusContent } from "./syllabus-content";

export type SyllabusCourse = {
  externalId: string;
  year: number;
  quarter: "1Q" | "2Q" | "3Q" | "4Q";
  quarters?: ("1Q" | "2Q" | "3Q" | "4Q")[];
  code: string;
  title: string;
  instructor: string;
  credits?: number;
  language?: string;
  meetings?: Meeting[];
  url: string;
  updated?: string;
  content?: SyllabusContent | null;
};
