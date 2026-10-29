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
        {t("courses.noSyllabusIsLinkedTo")}
      </p>
    );
  return (
    <section
      className="syllabusFrame"
      aria-label={t("courses.officialSyllabus")}
    >
      <iframe
        key={url}
        title={t("courses.officialSyllabus")}
        src={url}
        referrerPolicy="no-referrer"
      />
      <ExternalLink className="syllabusOutside" href={url}>
        {t("common.openInBrowser")} ↗
      </ExternalLink>
    </section>
  );
}
