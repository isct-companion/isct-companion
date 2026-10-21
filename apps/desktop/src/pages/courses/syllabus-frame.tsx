import { ExternalLink } from "../../components/ExternalLink";
import { t } from "../../i18n";

/**
 * The school's syllabus page of one offering, inside the app. Core builds
 * `url` (offering id and content language); it is shown as given.
 */
export function SyllabusFrame({ url }: { url?: string | null }) {
  if (!url)
    return (
      <p className="syllabusStatus" role="status">
        {t("No syllabus is linked to this course.")}
      </p>
    );
  return (
    <section className="syllabusFrame" aria-label={t("Official syllabus")}>
      <iframe
        key={url}
        title={t("Official syllabus")}
        src={url}
        referrerPolicy="no-referrer"
      />
      <ExternalLink className="syllabusOutside" href={url}>
        {t("Open in browser")} ↗
      </ExternalLink>
    </section>
  );
}
