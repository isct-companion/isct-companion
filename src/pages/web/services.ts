import type { SchoolSite } from "../../platform/desktop-bridge";

/** School services by how often students open them. */
export const serviceTiers: Array<{
  frequency: "high" | "medium" | "low";
  sites: SchoolSite[];
}> = [
  { frequency: "high", sites: ["science-lms", "kyomu"] },
  {
    frequency: "medium",
    sites: ["science-portal", "tokyo-portal", "tokyo-mail", "syllabus"],
  },
  { frequency: "low", sites: ["library", "tsubame"] },
];
