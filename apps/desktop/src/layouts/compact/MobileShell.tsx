import "./mobile.css";
import { useEffect, useState, type ReactElement, type ReactNode } from "react";
import { Drawer } from "@mantine/core";
import { UiButton } from "../../components/mantine";
import { PullRing } from "../../components/PullRing";
import type { SchoolSessionState } from "../../platform/school-session";
import type { Page } from "../../app/shared";
import { useEdgeSwipeBack } from "./edge-swipe";
import { t } from "../../i18n";

const tabs: Page[] = ["home", "timetable", "lectures", "web", "settings"];

export type MobileAccountStatus = {
  profile: "science" | "tokyo-tech";
  name: string;
  state: SchoolSessionState | undefined;
  label: string;
};

export type MobileStatus = {
  accounts: MobileAccountStatus[];
  lmsPhase: string;
  /** 0–100 while a pull runs, null when idle. */
  progress: number | null;
  refresh?: () => void;
  openAccounts: () => void;
};

/** One colour for the whole connection: problems first, then progress. */
function overallState(status: MobileStatus) {
  if (status.accounts.some((account) => account.state === "failed"))
    return "failed";
  if (status.progress !== null) return "pulling";
  if (status.accounts.some((account) => account.state === "active"))
    return "active";
  return "idle";
}

/**
 * The phone and portrait-tablet shell: one page at a time, tabs at the
 * bottom, and no fixed status bar. Connection state is a dot on Home that
 * opens a sheet; only a problem earns a banner.
 */
export function MobileShell({
  page,
  density,
  labels,
  icons,
  openPage,
  openRoot,
  pages,
  overlays,
  status,
  back,
  webMenu,
}: {
  page: Page;
  density: string;
  labels: Record<Page, string>;
  icons: Record<Page, ReactElement>;
  openPage: (page: Page) => void;
  /** Tapping the current tab again returns to that tab's root screen. */
  openRoot: (page: Page) => void;
  pages: Record<Page, ReactNode>;
  overlays: ReactNode;
  status: MobileStatus;
  /** Closes the screen pushed over a tab's root; undefined at the root. */
  back?: () => void;
  /** While browsing a school service, tapping Web again opens this menu. */
  webMenu?: () => void;
}) {
  useEdgeSwipeBack(back);
  const [sheet, setSheet] = useState(false);
  // Dialogs render in a portal outside the shell; this lets them go full screen.
  useEffect(() => {
    document.documentElement.dataset.shell = "mobile";
    return () => {
      delete document.documentElement.dataset.shell;
    };
  }, []);
  const state = overallState(status);
  const failed = status.accounts.filter(
    (account) => account.state === "failed",
  );

  return (
    <div className="app mobileShell" data-density={density} data-page={page}>
      <main className="mobileMain">
        {page === "home" && (
          <UiButton
            className="mobileStatusDot"
            data-state={state}
            aria-label={t("Connection status")}
            onClick={() => setSheet(true)}
          >
            {state === "pulling" ? (
              <PullRing value={status.progress ?? 0} size={16} />
            ) : (
              <i aria-hidden="true" />
            )}
          </UiButton>
        )}
        {page === "home" && failed.length > 0 && (
          <div className="mobileAttention" role="status">
            <span>
              {t("{{names}} needs signing in again.", {
                names: failed.map((account) => account.name).join(" · "),
              })}
            </span>
            <UiButton className="textButton" onClick={status.openAccounts}>
              {t("Sign in")}
            </UiButton>
          </div>
        )}
        {tabs.map((name) => pages[name])}
      </main>
      <nav className="mobileTabs" aria-label={t("Pages")}>
        {tabs.map((name) => {
          const menu = name === "web" && page === "web" ? webMenu : undefined;
          return (
            <UiButton
              key={name}
              className={page === name ? "active" : ""}
              aria-current={page === name ? "page" : undefined}
              aria-haspopup={menu ? "dialog" : undefined}
              aria-label={menu ? t("School services") : undefined}
              onClick={() =>
                menu ? menu() : name === page ? openRoot(name) : openPage(name)
              }
            >
              <span className="mobileTabIcon">
                {icons[name]}
                {menu && (
                  // A drawer mark: tapping Web again opens the services.
                  <svg
                    className="mobileTabMenuMark"
                    viewBox="0 0 12 12"
                    aria-hidden="true"
                  >
                    <rect x="1.5" y="1.5" width="9" height="9" rx="2.5" />
                    <path d="M4 5h4M4 7.5h4" />
                  </svg>
                )}
              </span>
              <span>{labels[name]}</span>
            </UiButton>
          );
        })}
      </nav>
      <Drawer
        opened={sheet}
        onClose={() => setSheet(false)}
        position="bottom"
        size="auto"
        title={t("Connection")}
        classNames={{ content: "mobileSheet" }}
      >
        <ul className="mobileStatusList">
          {status.accounts.map((account) => (
            <li key={account.profile} data-state={account.state ?? "checking"}>
              <i aria-hidden="true" />
              <b>{account.name}</b>
              <small>{account.label}</small>
            </li>
          ))}
          <li data-state={status.progress === null ? "idle" : "pulling"}>
            <i aria-hidden="true" />
            <b>LMS</b>
            <small>
              {status.lmsPhase}
              {status.progress !== null && ` · ${Math.round(status.progress)}%`}
            </small>
          </li>
        </ul>
        <div className="mobileSheetActions">
          {status.refresh && (
            <UiButton
              className="button primary"
              disabled={status.progress !== null}
              onClick={() => {
                status.refresh?.();
                setSheet(false);
              }}
            >
              {t("Refresh")}
            </UiButton>
          )}
          <UiButton
            className="button"
            onClick={() => {
              setSheet(false);
              status.openAccounts();
            }}
          >
            {t("School accounts")}
          </UiButton>
        </div>
      </Drawer>
      {overlays}
    </div>
  );
}
