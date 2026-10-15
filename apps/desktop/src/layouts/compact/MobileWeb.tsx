import { UiButton } from "../../components/mantine";
import { official } from "../../app/native";
import { serviceTiers } from "../../pages/web/services";
import type { SchoolSite } from "../../platform/desktop-bridge";
import { t } from "../../i18n";

const frequent = serviceTiers
  .filter((tier) => tier.frequency === "high")
  .flatMap((tier) => tier.sites);
const more = serviceTiers
  .filter((tier) => tier.frequency !== "high")
  .flatMap((tier) => tier.sites);

/** One school service per row; the caller decides how a service opens. */
export function MobileWebList({
  open,
  current,
}: {
  open: (site: SchoolSite) => void;
  /** The service last browsed, marked so returning to it is one tap. */
  current?: SchoolSite;
}) {
  const row = (site: SchoolSite) => (
    <li key={site}>
      <UiButton
        className="mobileWebRow"
        data-site={site}
        aria-current={current === site ? "true" : undefined}
        onClick={() => open(site)}
      >
        <i aria-hidden="true">{official[site].name.slice(0, 1)}</i>
        <span>
          <b>{official[site].name}</b>
          <small>{new URL(official[site].url).host}</small>
        </span>
        <em aria-hidden="true">›</em>
      </UiButton>
    </li>
  );
  return (
    <div className="page mobileWeb">
      <h1>{t("Web")}</h1>
      <ul className="mobileWebList">{frequent.map(row)}</ul>
      <h2>{t("More services")}</h2>
      <ul className="mobileWebList">{more.map(row)}</ul>
    </div>
  );
}
