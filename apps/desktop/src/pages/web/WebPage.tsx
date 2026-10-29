import { ArrowLeft, ChevronLeft, ChevronRight, RotateCw } from "lucide-react";
import { ResizableWorkspace } from "../../components/ResizableWorkspace";
import { useEffect, useRef, useState, type FormEvent } from "react";
import { UiButton, UiInput } from "../../components/ui";
import { lookup, t, type MessageKey } from "../../i18n";
import {
  nativeOs,
  type SchoolLoadFailure,
  type SchoolSite,
  type SchoolWebNotice,
} from "../../platform/desktop-bridge";
import { serviceTiers } from "./services";
import { schoolWebBounds } from "../../platform/school-web-bounds";
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

/** What the notice under the address says when a page did not load. */
const failureText: Record<SchoolLoadFailure, MessageKey> = {
  offline: "web.thisPageCouldNotBe",
  "host-not-found": "web.thisPageCouldNotBeOpened",
  unreachable: "web.thisPageCouldNotBeOpenedIts",
  insecure: "web.thisPageCouldNotBeOpenedThe",
  cleartext: "web.thisPageUsesAnUnencrypted",
  other: "web.thisPageCouldNotBeOpened2",
};

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
  /** Popups open over each site's view; the toolbar acts on the front one. */
  const [popups, setPopups] = useState<Partial<Record<SchoolSite, number>>>({});
  const [notice, setNotice] = useState<SchoolWebNotice>();
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
  const bounds = () => {
    const rect = host.current?.getBoundingClientRect();
    return rect ? schoolWebBounds(rect, window) : undefined;
  };

  useEffect(() => {
    if (!desktopBridge || !schoolWebAvailable) return;
    void desktopBridge.configureSchoolWebPolicy({
      restoreAcrossRestarts: true,
      idleMinutes: 10,
    });
  }, [schoolWebAvailable]);

  useEffect(
    () =>
      desktopBridge?.onSchoolWebUrl?.((request) => {
        if (request.site !== site) return;
        const { url } = request;
        setCurrentUrl(url);
        if (!editingAddress.current) setAddress(url);
        // A page loaded: an earlier failure no longer describes it.
        setNotice((shown) =>
          shown?.kind === "load-failed" ? undefined : shown,
        );
      }),
    [site],
  );

  useEffect(
    () =>
      desktopBridge?.onSchoolWebPopups?.(({ site: popupSite, open }) =>
        setPopups((current) => ({ ...current, [popupSite]: open })),
      ),
    [],
  );

  useEffect(() => {
    setNotice(undefined);
    let clear: number | undefined;
    const dispose = desktopBridge?.onSchoolWebNotice?.((next) => {
      if (next.site !== site) return;
      window.clearTimeout(clear);
      setNotice(next);
      if (next.kind === "reopened")
        clear = window.setTimeout(() => setNotice(undefined), 6000);
    });
    return () => {
      dispose?.();
      window.clearTimeout(clear);
    };
  }, [site]);

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
    const customizationSite = official[site].customization;
    const custom = data.settings.customization?.sites[customizationSite];
    const permitted =
      platform === "native-desktop" &&
      (data.settings.siteConsents?.[customizationSite] ?? false);
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
            permitted ? (custom?.css ?? "") : "",
            permitted ? (custom?.js ?? "") : "",
            bounds(),
            resolved.url,
          );
        });
      })
      .then(async (state) => {
        if (cancelled || !state) return;
        openedRequest.current = requestKey;
        setCurrentUrl(state.url);
        if (!editingAddress.current) setAddress(state.url);
        setStatus("");
        await desktopBridge?.setSchoolWebZoom?.(site, resolvedZoom);
      })
      .catch((error) => {
        if (!cancelled) setStatus(errorText(error) ?? t("web.unavailable"));
      });
    const observer = new ResizeObserver(() => {
      const next = bounds();
      if (next && !schoolWebExit.pending)
        void desktopBridge?.setSchoolWebBounds?.(next).catch(() => undefined);
    });
    if (host.current) observer.observe(host.current);
    const onResize = () => {
      const next = bounds();
      if (next && !schoolWebExit.pending)
        void desktopBridge?.setSchoolWebBounds?.(next).catch(() => undefined);
    };
    addEventListener("resize", onResize);
    window.visualViewport?.addEventListener("resize", onResize);
    return () => {
      cancelled = true;
      observer.disconnect();
      removeEventListener("resize", onResize);
      window.visualViewport?.removeEventListener("resize", onResize);
    };
    // Site changes intentionally reuse the resident view and its shared session.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [site, embedded, active, requestKey]);

  const choose = (next: SchoolSite) => {
    if (schoolWebExit.pending) return;
    // Choosing the site already shown returns its view to the start page. The
    // open effect only runs when `site` changes, so this has to navigate itself.
    // Popups over the view close first: the address bar acts on the front one.
    if (next === site && embedded) {
      void (async () => {
        for (let open = 0; open < 8; open++)
          if (!(await desktopBridge?.closeSchoolWebPopup?.(site))) break;
        await openTarget(official[next].url, next);
      })().catch((error) =>
        setStatus(errorText(error) ?? t("web.unavailable")),
      );
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
        const customizationSite = official[destination].customization;
        const custom = data.settings.customization?.sites[customizationSite];
        const permitted =
          platform === "native-desktop" &&
          (data.settings.siteConsents?.[customizationSite] ?? false);
        await desktopBridge.openSchoolSite(
          destination,
          permitted ? (custom?.css ?? "") : "",
          permitted ? (custom?.js ?? "") : "",
          bounds(),
          target,
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
  const goBack = async () => {
    try {
      await desktopBridge?.schoolWebBack?.(site);
    } catch (error) {
      setStatus(errorText(error) ?? t("web.unavailable"));
    }
  };
  const goForward = async () => {
    try {
      await desktopBridge?.schoolWebForward?.(site);
    } catch (error) {
      setStatus(errorText(error) ?? t("web.unavailable"));
    }
  };
  const closePopup = () =>
    void desktopBridge
      ?.closeSchoolWebPopup?.(site)
      .catch((error) => setStatus(errorText(error) ?? t("web.unavailable")));
  const retry = (url: string) => {
    setNotice(undefined);
    void desktopBridge
      ?.navigateSchoolWeb?.(site, url)
      .catch((error) => setStatus(errorText(error) ?? t("web.unavailable")));
  };
  const openInBrowser = (url: string) =>
    void desktopBridge
      ?.openExternal(url)
      .catch(() => notify(t("web.couldNotOpenTheSystem")));
  const popupCount = popups[site] ?? 0;

  const setPageZoom = (next: number) => {
    const value = Math.round(clamp(next, 0.5, 2) * 10) / 10;
    setZoom(value);
    if (embedded)
      void desktopBridge
        ?.setSchoolWebZoom?.(site, value)
        .catch(() => setStatus(t("web.zoomCouldNotBeChanged")));
  };

  const targetUrl = currentUrl || official[site].url;
  const openOutside = () => {
    if (desktopBridge && schoolWebAvailable && !embedded) {
      const customizationSite = official[site].customization;
      const custom = data.settings.customization?.sites[customizationSite];
      const permitted =
        data.settings.siteConsents?.[customizationSite] ?? false;
      void desktopBridge
        .openSchoolSite(
          site,
          permitted ? (custom?.css ?? "") : "",
          permitted ? (custom?.js ?? "") : "",
          undefined,
          targetUrl,
        )
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
                {embedded && desktopBridge?.schoolWebBack && (
                  <UiButton
                    type="button"
                    className="iconButton"
                    title={t("common.previousPage")}
                    aria-label={t("common.previousPage")}
                    onClick={() => void goBack()}
                  >
                    <ChevronLeft />
                  </UiButton>
                )}
                {embedded && desktopBridge?.schoolWebForward && (
                  <UiButton
                    type="button"
                    className="iconButton"
                    title={t("common.nextPage")}
                    aria-label={t("common.nextPage")}
                    onClick={() => void goForward()}
                  >
                    <ChevronRight />
                  </UiButton>
                )}
                {embedded && desktopBridge?.reloadSchoolWeb && (
                  <UiButton
                    type="button"
                    className="iconButton"
                    title={t("common.reload")}
                    aria-label={t("common.reload")}
                    onClick={() => void desktopBridge?.reloadSchoolWeb?.(site)}
                  >
                    <RotateCw />
                  </UiButton>
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
            {embedded && popupCount > 0 && (
              <div className="webNotice" role="status">
                <span>{t("web.popupWindow")}</span>
                <UiButton type="button" onClick={closePopup}>
                  {t("web.closeWindow")}
                </UiButton>
              </div>
            )}
            {embedded && notice?.kind === "load-failed" && (
              <div className="webNotice warning" role="alert">
                <span>{t(failureText[notice.reason])}</span>
                {notice.url && notice.reason !== "cleartext" && (
                  <UiButton type="button" onClick={() => retry(notice.url!)}>
                    {t("common.retry")}
                  </UiButton>
                )}
                {notice.url && (
                  <UiButton
                    type="button"
                    onClick={() => openInBrowser(notice.url!)}
                  >
                    {t("common.openInBrowser")}
                  </UiButton>
                )}
              </div>
            )}
            {embedded && notice?.kind === "reopened" && (
              <div className="webNotice" role="status">
                <span>{t("web.reopenedThisPageWithoutSending")}</span>
              </div>
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
