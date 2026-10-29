import { ArrowLeft, ChevronLeft, ChevronRight, RotateCw } from "lucide-react";
import { ResizableWorkspace } from "../../components/ResizableWorkspace";
import { useEffect, useRef, useState, type FormEvent } from "react";
import { UiButton, UiInput } from "../../components/ui";
import { lookup, t } from "../../i18n";
import { nativeOs, type SchoolSite } from "../../platform/desktop-bridge";
import { serviceTiers } from "./services";
import {
  boundsOf,
  followBounds,
  SchoolWebNavigation,
  SchoolWebNotices,
  useSchoolWebPage,
} from "../../features/school-web";
import type { Workspace } from "../../lib/model";
import {
  claimSchoolView,
  desktopBridge,
  schoolWebExit,
  official,
} from "../../app/native";
import { errorText, type Platform } from "../../app/shared";
import { withRecords } from "../../platform/records";

const clamp = (value: number, minimum: number, maximum: number) =>
  Math.min(maximum, Math.max(minimum, value));

/** This page's id as a presenter of the school views (`SchoolWebEntry`). */
const surface = "web";

const normalizeAddress = (raw: string) => {
  const trimmed = raw.trim();
  if (!trimmed) throw new Error(t("web.enterASchoolURL"));
  const url = new URL(
    /^[a-z][a-z\d+.-]*:/i.test(trimmed) ? trimmed : `https://${trimmed}`,
  );
  if (url.protocol !== "https:" && url.protocol !== "http:")
    throw new Error(t("web.enterAnHTTPOrHTTPS"));
  return url.toString();
};

function WebBody({
  platform,
  data,
  update,
  notify,
  schoolWebAvailable,
  requestedUrl,
  request,
  returnLabel,
  onReturn,
  active = true,
}: {
  /** False while another page is shown; the page stays mounted (keep alive). */
  active?: boolean;
  platform: Platform;
  data: Workspace;
  update: (f: (d: Workspace) => Workspace) => void;
  notify: (s: string) => void;
  schoolWebAvailable: boolean;
  requestedUrl?: string;
  request?: number;
  returnLabel?: string;
  onReturn?: () => void;
}) {
  const [site, setSite] = useState<SchoolSite>(() => {
    if (requestedUrl) return "science-lms";
    const active = sessionStorage.getItem("isct-companion-active-school-site");
    if (active && active in official) return active as SchoolSite;
    const saved = data.settings.activeSchoolSite;
    return saved && saved in official
      ? (saved as SchoolSite)
      : "science-portal";
  });
  const initialUrl = requestedUrl ?? official[site].url;
  const [status, setStatus] = useState("");
  /** The last file a school page saved, offered for Reveal for a while. */
  const [savedFile, setSavedFile] = useState<{ name: string; path: string }>();
  const notifyRef = useRef(notify);
  notifyRef.current = notify;
  /** The address outside the embedded view (preview, separate window). */
  const [currentUrl, setCurrentUrl] = useState(initialUrl);
  const [address, setAddress] = useState(initialUrl);
  const zoom = data.settings.schoolWebZoom ?? 1;
  const setZoom = (value: number) =>
    update((draft) => {
      draft.settings.schoolWebZoom = value;
      return draft;
    });
  const resolvedZoom = clamp(zoom, 0.5, 2);
  const host = useRef<HTMLDivElement>(null);
  const owner = useRef(Symbol("web-page"));
  const addressInput = useRef<HTMLInputElement>(null);
  const editingAddress = useRef(false);
  const openedRequest = useRef<string | undefined>(undefined);
  const requestKey = requestedUrl
    ? JSON.stringify([requestedUrl, request])
    : undefined;
  const embedded =
    Boolean(desktopBridge?.setSchoolWebBounds) && schoolWebAvailable;
  const page = useSchoolWebPage(embedded ? site : undefined);
  const shownUrl = page?.url;
  const failed = (error: unknown) =>
    setStatus(errorText(error) ?? t("web.unavailable"));
  /** The page customization the person allowed for `destination`. */
  const customization = (destination: SchoolSite) => {
    const customizationSite = official[destination].customization;
    const custom = data.settings.customization?.sites[customizationSite];
    const permitted =
      platform === "native-desktop" &&
      (data.settings.siteConsents?.[customizationSite] ?? false);
    return {
      css: permitted ? (custom?.css ?? "") : "",
      js: permitted ? (custom?.js ?? "") : "",
    };
  };

  useEffect(() => {
    if (!desktopBridge || !schoolWebAvailable) return;
    void desktopBridge.configureSchoolWebPolicy({
      restoreAcrossRestarts: true,
      idleMinutes: 10,
    });
  }, [schoolWebAvailable]);

  useEffect(() => {
    if (shownUrl && !editingAddress.current) setAddress(shownUrl);
  }, [shownUrl]);

  useEffect(() => {
    if (!editingAddress.current && addressInput.current)
      addressInput.current.scrollLeft = 0;
  }, [address]);

  useEffect(() => {
    let clear: number | undefined;
    const dispose = desktopBridge?.onSchoolLoginStatus?.((event) => {
      window.clearTimeout(clear);
      if (event.state === "progress") setStatus(t("common.signingIn"));
      else if (event.state === "connected") {
        setStatus(t("common.signedIn"));
        clear = window.setTimeout(() => setStatus(""), 2500);
      } else setStatus(lookup(event.message) ?? event.message);
    });
    return () => {
      dispose?.();
      window.clearTimeout(clear);
    };
  }, []);

  useEffect(() => {
    let hide: number | undefined;
    const dispose = desktopBridge?.onSchoolDownload?.((event) => {
      window.clearTimeout(hide);
      setSavedFile(undefined);
      if (event.state === "started")
        setStatus(t("web.downloadingName", { name: event.name }));
      else if (event.state === "done") {
        setStatus("");
        notifyRef.current(`Saved ${event.name}`);
        if (event.path) {
          setSavedFile({ name: event.name, path: event.path });
          hide = window.setTimeout(() => setSavedFile(undefined), 30_000);
        }
      } else setStatus(t("web.downloadFailedName", { name: event.name }));
    });
    return () => {
      dispose?.();
      window.clearTimeout(hide);
    };
  }, []);

  const revealSaved = async (path: string) => {
    try {
      const shown = await desktopBridge?.revealSchoolDownload?.(path);
      if (!shown) setStatus(t("web.thatFileCanNoLonger"));
    } catch (error) {
      setStatus(errorText(error) ?? t("web.unavailable"));
    }
  };

  useEffect(() => {
    if (!embedded || !desktopBridge || !active) return;
    const target =
      requestedUrl && openedRequest.current !== requestKey
        ? requestedUrl
        : undefined;
    // Settings → "Show page" asks for a site while this page is kept alive.
    const requested = sessionStorage.getItem(
      "isct-companion-active-school-site",
    );
    if (!target && requested && requested in official && requested !== site) {
      setSite(requested as SchoolSite);
      return;
    }
    let cancelled = false;
    let ready = false;
    const { css, js } = customization(site);
    setStatus(t("web.opening"));
    void schoolWebExit
      .settled()
      .then(() => {
        if (cancelled) return undefined;
        claimSchoolView(owner.current);
        return (
          target
            ? desktopBridge!.resolveSchoolUrl(target, site)
            : Promise.resolve({ site, url: undefined })
        ).then((resolved) => {
          if (resolved.site !== site) {
            setSite(resolved.site);
            return undefined;
          }
          return desktopBridge!.openSchoolSite(
            resolved.site,
            css,
            js,
            boundsOf(host.current),
            surface,
            resolved.url
              ? { kind: "go", url: resolved.url }
              : { kind: "resume" },
          );
        });
      })
      .then(async (state) => {
        if (cancelled || !state) return;
        ready = true;
        openedRequest.current = requestKey;
        if (!editingAddress.current) setAddress(state.url);
        setStatus("");
        await desktopBridge?.setSchoolWebZoom?.(site, resolvedZoom);
      })
      .catch((error) => {
        if (!cancelled) setStatus(errorText(error) ?? t("web.unavailable"));
      });
    const stop = followBounds(
      () => host.current,
      () => ready,
    );
    return () => {
      cancelled = true;
      stop();
    };
    // Site changes intentionally reuse the resident view and its shared session.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [site, embedded, active, requestKey]);

  const choose = (next: SchoolSite) => {
    if (schoolWebExit.pending) return;
    // Choosing the site already shown returns its view to the start page, as
    // an app link: Core closes the popups over it first. The open effect only
    // runs when `site` changes, so this opens it itself.
    if (next === site && embedded && desktopBridge) {
      const { css, js } = customization(next);
      const start = official[next].url;
      setAddress(start);
      void desktopBridge
        .openSchoolSite(next, css, js, boundsOf(host.current), surface, {
          kind: "go",
          url: start,
        })
        .catch(failed);
      return;
    }
    sessionStorage.setItem("isct-companion-active-school-site", next);
    update((draft) => {
      draft.settings.activeSchoolSite = next;
      return draft;
    });
    const nextUrl = official[next].url;
    setCurrentUrl(nextUrl);
    setAddress(nextUrl);
    setSite(next);
  };

  const navigate = async (event: FormEvent) => {
    event.preventDefault();
    try {
      const target = normalizeAddress(address);
      if (!desktopBridge)
        throw new Error(t("web.schoolNavigationNeedsTheInstalled"));
      const resolved = await desktopBridge.resolveSchoolUrl(target, site);
      await openTarget(resolved.url, resolved.site);
    } catch (error) {
      setStatus(errorText(error) ?? t("web.invalidAddress"));
    }
  };

  /** Shows `target` in its service's view and switches the picker to it. */
  const openTarget = async (target: string, destination: SchoolSite) => {
    setAddress(target);
    setStatus(t("web.opening"));
    if (embedded && desktopBridge?.navigateSchoolWeb) {
      const navigated = await desktopBridge.navigateSchoolWeb(
        destination,
        target,
      );
      if (!navigated) {
        const { css, js } = customization(destination);
        await desktopBridge.openSchoolSite(
          destination,
          css,
          js,
          boundsOf(host.current),
          surface,
          { kind: "go", url: target },
        );
      }
    }
    if (destination !== site) {
      sessionStorage.setItem("isct-companion-active-school-site", destination);
      update((draft) => {
        draft.settings.activeSchoolSite = destination;
        return draft;
      });
      setSite(destination);
    }
    setCurrentUrl(target);
    setStatus("");
  };
  const setPageZoom = (next: number) => {
    const value = Math.round(clamp(next, 0.5, 2) * 10) / 10;
    setZoom(value);
    if (embedded)
      void desktopBridge
        ?.setSchoolWebZoom?.(site, value)
        .catch(() => setStatus(t("web.zoomCouldNotBeChanged")));
  };

  const targetUrl = shownUrl || currentUrl || official[site].url;
  const openOutside = () => {
    if (desktopBridge && schoolWebAvailable && !embedded) {
      const { css, js } = customization(site);
      void desktopBridge
        .openSchoolSite(site, css, js, undefined, surface, {
          kind: "go",
          url: targetUrl,
        })
        .catch(() => notify(t("web.theSchoolWindowCouldNot")));
    } else if (desktopBridge && platform !== "browser-preview")
      void desktopBridge
        .openExternal(targetUrl)
        .catch(() => notify(t("web.couldNotOpenTheSystem")));
    else window.open(targetUrl, "_blank", "noopener,noreferrer");
  };

  const frameable = site !== "science-lms";
  return (
    <div className="page web servicesPage">
      <div className="servicesBody">
        <ResizableWorkspace
          id="web"
          enabled
          firstSize={168}
          firstMin={104}
          firstMax={360}
          secondMin="40%"
          secondMax="100%"
        >
          <nav className="servicePicker" aria-label={t("web.schoolServices")}>
            {serviceTiers.map((tier, tierIndex) => (
              <div className="serviceTier" key={tier.frequency}>
                {tierIndex > 0 && <div className="serviceDivider" />}
                {tier.sites.map((key) => (
                  <UiButton
                    type="button"
                    className={site === key ? "active" : ""}
                    data-frequency={tier.frequency}
                    title={t(official[key].name)}
                    onClick={() => choose(key)}
                    key={key}
                  >
                    <i aria-hidden="true" />
                    <span>{t(official[key].name)}</span>
                  </UiButton>
                ))}
              </div>
            ))}
          </nav>
          <section
            className="workspacePane webWorkspace"
            aria-label={t(official[site].name)}
          >
            <form
              className="webToolbar"
              onSubmit={(event) => void navigate(event)}
            >
              <div className="webAddress">
                {onReturn && (
                  <UiButton
                    type="button"
                    className="iconButton"
                    title={t("web.backToPage", { page: returnLabel })}
                    aria-label={t("web.backToPage", { page: returnLabel })}
                    onClick={onReturn}
                  >
                    <ArrowLeft />
                  </UiButton>
                )}
                <span className="webSecure" aria-hidden="true">
                  ⌁
                </span>
                <UiInput
                  ref={addressInput}
                  className="webAddressInput"
                  aria-label={t("web.currentURL")}
                  spellCheck={false}
                  value={address}
                  onChange={(event) => setAddress(event.currentTarget.value)}
                  onFocus={(event) => {
                    editingAddress.current = true;
                    event.currentTarget.select();
                  }}
                  onBlur={() => {
                    editingAddress.current = false;
                    if (addressInput.current)
                      addressInput.current.scrollLeft = 0;
                  }}
                />
                <span className="webStatus" role="status">
                  {status}
                </span>
                {savedFile && desktopBridge?.revealSchoolDownload && (
                  <UiButton
                    type="button"
                    title={savedFile.name}
                    onClick={() => void revealSaved(savedFile.path)}
                  >
                    {t(
                      nativeOs() === "macos"
                        ? "web.showInFinder"
                        : "web.showInFolder",
                    )}
                  </UiButton>
                )}
                {embedded && (
                  <SchoolWebNavigation
                    site={site}
                    page={page}
                    icons={{
                      back: <ChevronLeft />,
                      forward: <ChevronRight />,
                      reload: <RotateCw />,
                    }}
                    className={{
                      back: "iconButton",
                      forward: "iconButton",
                      reload: "iconButton",
                    }}
                    onError={failed}
                  />
                )}
              </div>
              <div className="webZoom" aria-label={t("web.pageZoom")}>
                <UiButton
                  type="button"
                  className="iconButton"
                  aria-label={t("common.zoomOut")}
                  title={t("common.zoomOut")}
                  disabled={resolvedZoom <= 0.5}
                  onClick={() => setPageZoom(resolvedZoom - 0.1)}
                >
                  −
                </UiButton>
                <UiButton
                  type="button"
                  className="zoomValue"
                  title={t("web.resetZoom")}
                  onClick={() => setPageZoom(1)}
                >
                  {Math.round(resolvedZoom * 100)}%
                </UiButton>
                <UiButton
                  type="button"
                  className="iconButton"
                  aria-label={t("common.zoomIn")}
                  title={t("common.zoomIn")}
                  disabled={resolvedZoom >= 2}
                  onClick={() => setPageZoom(resolvedZoom + 0.1)}
                >
                  +
                </UiButton>
              </div>
              <UiButton
                type="button"
                className="iconButton webOpenOutside"
                title={t("common.openInBrowser")}
                aria-label={t("common.openInBrowser")}
                onClick={openOutside}
              >
                ↗
              </UiButton>
            </form>
            {embedded && (
              <SchoolWebNotices site={site} page={page} onError={failed} />
            )}
            <div className="webHost" ref={host}>
              {embedded ? null : platform === "browser-preview" && frameable ? (
                <iframe
                  key={`${site}:${targetUrl}`}
                  title={t(official[site].name)}
                  src={targetUrl}
                  referrerPolicy="no-referrer"
                  style={{
                    zoom: resolvedZoom,
                    width: `${100 / resolvedZoom}%`,
                    height: `${100 / resolvedZoom}%`,
                  }}
                />
              ) : (
                <div className="webFallback">
                  <b>{t(official[site].name)}</b>
                  <p>
                    {t(
                      platform === "browser-preview"
                        ? "web.t2scholaDoesNotAllowEmbedding"
                        : desktopBridge
                          ? "web.thisBuildOpensSchoolPages"
                          : "web.schoolPagesNeedTheDesktop",
                    )}
                  </p>
                  <UiButton className="button primary" onClick={openOutside}>
                    {t("web.openSite", { site: t(official[site].name) })} ↗
                  </UiButton>
                </div>
              )}
            </div>
          </section>
        </ResizableWorkspace>
      </div>
    </div>
  );
}

/** Web with the records it shows (`platform/records`). */
export const Web = withRecords([], WebBody);
