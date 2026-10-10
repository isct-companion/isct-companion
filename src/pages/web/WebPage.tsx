import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type FormEvent,
  type PointerEvent,
} from "react";
import { UiButton, UiInput } from "../../components/mantine";
import { type SchoolSite } from "../../platform/desktop-bridge";
import { schoolWebBounds } from "../../platform/school-web-bounds";
import type { Workspace } from "../../core/model";
import {
  claimSchoolView,
  desktopBridge,
  schoolWebExit,
  official,
} from "../../app/native";
import { type Platform } from "../../app/shared";

const serviceTiers: Array<{
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

const clamp = (value: number, minimum: number, maximum: number) =>
  Math.min(maximum, Math.max(minimum, value));

const normalizeAddress = (raw: string) => {
  const trimmed = raw.trim();
  if (!trimmed) throw new Error("Enter a school URL.");
  const url = new URL(
    /^[a-z][a-z\d+.-]*:/i.test(trimmed) ? trimmed : `https://${trimmed}`,
  );
  if (url.protocol !== "https:")
    throw new Error("School pages must use HTTPS.");
  return url.toString();
};

export function Web({
  platform,
  data,
  update,
  notify,
  schoolWebAvailable,
  requestedUrl,
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
  const [currentUrl, setCurrentUrl] = useState(initialUrl);
  const [address, setAddress] = useState(initialUrl);
  const serviceWidth = data.settings.schoolServiceWidth ?? 168;
  const zoom = data.settings.schoolWebZoom ?? 1;
  const setServiceWidth = (value: number) =>
    update((draft) => {
      draft.settings.schoolServiceWidth = value;
      return draft;
    });
  const setZoom = (value: number) =>
    update((draft) => {
      draft.settings.schoolWebZoom = value;
      return draft;
    });
  const resolvedServiceWidth = clamp(serviceWidth, 132, 360);
  const resolvedZoom = clamp(zoom, 0.5, 2);
  const host = useRef<HTMLDivElement>(null);
  const owner = useRef(Symbol("web-page"));
  const addressInput = useRef<HTMLInputElement>(null);
  const editingAddress = useRef(false);
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
      }),
    [site],
  );

  useEffect(() => {
    if (!editingAddress.current && addressInput.current)
      addressInput.current.scrollLeft = 0;
  }, [address]);

  useEffect(() => {
    let clear: number | undefined;
    const dispose = desktopBridge?.onSchoolLoginStatus?.((event) => {
      window.clearTimeout(clear);
      if (event.state === "progress") setStatus("Signing in…");
      else if (event.state === "connected") {
        setStatus("Signed in");
        clear = window.setTimeout(() => setStatus(""), 2500);
      } else setStatus(event.message);
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
      if (event.state === "started") setStatus(`Downloading ${event.name}…`);
      else if (event.state === "done") {
        setStatus("");
        notifyRef.current(`Saved ${event.name}`);
        if (event.path) {
          setSavedFile({ name: event.name, path: event.path });
          hide = window.setTimeout(() => setSavedFile(undefined), 30_000);
        }
      } else setStatus(`Download failed: ${event.name}`);
    });
    return () => {
      dispose?.();
      window.clearTimeout(hide);
    };
  }, []);

  const revealSaved = async (path: string) => {
    try {
      const shown = await desktopBridge?.revealSchoolDownload?.(path);
      if (!shown) setStatus("That file can no longer be shown.");
    } catch (error) {
      setStatus(error instanceof Error ? error.message : "Unavailable");
    }
  };

  useEffect(() => {
    if (!embedded || !desktopBridge || !active) return;
    // Settings → "Show page" asks for a site while this page is kept alive.
    const requested = sessionStorage.getItem(
      "isct-companion-active-school-site",
    );
    if (
      !requestedUrl &&
      requested &&
      requested in official &&
      requested !== site
    ) {
      setSite(requested as SchoolSite);
      return;
    }
    let cancelled = false;
    const customizationSite = official[site].customization;
    const custom = data.settings.customization?.sites[customizationSite];
    const permitted =
      platform === "native-desktop" &&
      (data.settings.siteConsents?.[customizationSite] ?? false);
    setStatus("Opening…");
    void schoolWebExit
      .settled()
      .then(() => {
        if (cancelled) return undefined;
        claimSchoolView(owner.current);
        return (
          requestedUrl
            ? desktopBridge!.resolveSchoolUrl(requestedUrl)
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
        setCurrentUrl(state.url);
        if (!editingAddress.current) setAddress(state.url);
        setStatus("");
        await desktopBridge?.setSchoolWebZoom?.(site, resolvedZoom);
      })
      .catch((error) => {
        if (!cancelled)
          setStatus(error instanceof Error ? error.message : "Unavailable");
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
  }, [site, embedded, active]);

  const choose = (next: SchoolSite) => {
    if (schoolWebExit.pending) return;
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
        throw new Error("School navigation needs the installed app.");
      const resolved = await desktopBridge.resolveSchoolUrl(target);
      await openTarget(resolved.url, resolved.site);
    } catch (error) {
      setStatus(error instanceof Error ? error.message : "Invalid address");
    }
  };

  /** Shows `target` in its service's view and switches the picker to it. */
  const openTarget = async (target: string, destination: SchoolSite) => {
    setAddress(target);
    setStatus("Opening…");
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
  const openTargetRef = useRef(openTarget);
  openTargetRef.current = openTarget;

  // `target="_blank"` links to another service (e.g. 教務Web → LMS) switch
  // the picker, like a browser opening a new tab.
  useEffect(
    () =>
      desktopBridge?.onSchoolWebOpen?.(({ site: destination, url }) => {
        if (schoolWebExit.pending || !(destination in official)) return;
        openTargetRef
          .current(url, destination)
          .catch((error) =>
            setStatus(error instanceof Error ? error.message : "Unavailable"),
          );
      }),
    [],
  );

  const goBack = async () => {
    try {
      const moved = await desktopBridge?.schoolWebBack?.(site);
      setStatus(moved ? "" : "No previous page.");
    } catch (error) {
      setStatus(error instanceof Error ? error.message : "Unavailable");
    }
  };

  const setPageZoom = (next: number) => {
    const value = Math.round(clamp(next, 0.5, 2) * 10) / 10;
    setZoom(value);
    if (embedded)
      void desktopBridge
        ?.setSchoolWebZoom?.(site, value)
        .catch(() => setStatus("Zoom could not be changed."));
  };

  const startResize = (event: PointerEvent<HTMLDivElement>) => {
    if (event.pointerType === "mouse" && event.button !== 0) return;
    const startX = event.clientX;
    const startWidth = resolvedServiceWidth;
    const move = (moveEvent: globalThis.PointerEvent) =>
      setServiceWidth(clamp(startWidth + moveEvent.clientX - startX, 132, 360));
    const stop = () => {
      removeEventListener("pointermove", move);
      removeEventListener("pointerup", stop);
    };
    addEventListener("pointermove", move);
    addEventListener("pointerup", stop, { once: true });
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
        .catch(() => notify("The school window could not be opened."));
    } else if (desktopBridge && platform !== "browser-preview")
      void desktopBridge
        .openExternal(targetUrl)
        .catch(() => notify("Could not open the system browser."));
    else window.open(targetUrl, "_blank", "noopener,noreferrer");
  };

  const frameable = site !== "science-lms";
  return (
    <div className="page web servicesPage">
      <form className="webToolbar" onSubmit={(event) => void navigate(event)}>
        <div className="webAddress">
          {onReturn && (
            <UiButton
              type="button"
              className="iconButton"
              title={`Back to ${returnLabel}`}
              aria-label={`Back to ${returnLabel}`}
              onClick={onReturn}
            >
              ←
            </UiButton>
          )}
          <span className="webSecure" aria-hidden="true">
            ⌁
          </span>
          <UiInput
            ref={addressInput}
            className="webAddressInput"
            aria-label="Current URL"
            spellCheck={false}
            value={address}
            onChange={(event) => setAddress(event.currentTarget.value)}
            onFocus={(event) => {
              editingAddress.current = true;
              event.currentTarget.select();
            }}
            onBlur={() => {
              editingAddress.current = false;
              if (addressInput.current) addressInput.current.scrollLeft = 0;
            }}
          />
          <span className="webStatus" role="status">
            {status}
          </span>
          {savedFile && desktopBridge?.revealSchoolDownload && (
            <UiButton
              type="button"
              title={`Show ${savedFile.name} in Finder`}
              onClick={() => void revealSaved(savedFile.path)}
            >
              Reveal
            </UiButton>
          )}
          {embedded && desktopBridge?.schoolWebBack && (
            <UiButton
              type="button"
              className="iconButton"
              title="Previous page"
              aria-label="Previous page"
              onClick={() => void goBack()}
            >
              ←
            </UiButton>
          )}
          {embedded && desktopBridge?.reloadSchoolWeb && (
            <UiButton
              type="button"
              className="iconButton"
              title="Reload"
              aria-label="Reload"
              onClick={() => void desktopBridge?.reloadSchoolWeb?.(site)}
            >
              ↻
            </UiButton>
          )}
          {embedded && site === "kyomu" && (
            <UiButton
              type="button"
              title="教務Web top page"
              onClick={() =>
                void openTarget(official.kyomu.url, "kyomu").catch((error) =>
                  setStatus(
                    error instanceof Error ? error.message : "Unavailable",
                  ),
                )
              }
            >
              Top
            </UiButton>
          )}
        </div>
        <div className="webZoom" aria-label="Page zoom">
          <UiButton
            type="button"
            className="iconButton"
            aria-label="Zoom out"
            title="Zoom out"
            disabled={resolvedZoom <= 0.5}
            onClick={() => setPageZoom(resolvedZoom - 0.1)}
          >
            −
          </UiButton>
          <UiButton
            type="button"
            className="zoomValue"
            title="Reset zoom"
            onClick={() => setPageZoom(1)}
          >
            {Math.round(resolvedZoom * 100)}%
          </UiButton>
          <UiButton
            type="button"
            className="iconButton"
            aria-label="Zoom in"
            title="Zoom in"
            disabled={resolvedZoom >= 2}
            onClick={() => setPageZoom(resolvedZoom + 0.1)}
          >
            +
          </UiButton>
        </div>
        <UiButton
          type="button"
          className="iconButton webOpenOutside"
          title="Open in browser"
          aria-label="Open in browser"
          onClick={openOutside}
        >
          ↗
        </UiButton>
      </form>

      <div
        className="servicesBody"
        style={
          {
            "--service-width": `${resolvedServiceWidth}px`,
          } as CSSProperties
        }
      >
        <nav className="servicePicker" aria-label="School services">
          {serviceTiers.map((tier, tierIndex) => (
            <div className="serviceTier" key={tier.frequency}>
              {tierIndex > 0 && <div className="serviceDivider" />}
              {tier.sites.map((key) => (
                <UiButton
                  type="button"
                  className={site === key ? "active" : ""}
                  data-frequency={tier.frequency}
                  title={`${official[key].name} · ${tier.frequency} frequency`}
                  onClick={() => choose(key)}
                  key={key}
                >
                  <i aria-hidden="true" />
                  <span>{official[key].name}</span>
                </UiButton>
              ))}
            </div>
          ))}
        </nav>
        <div
          className="serviceResizeHandle"
          role="separator"
          aria-label="Resize service list"
          aria-orientation="vertical"
          aria-valuemin={132}
          aria-valuemax={360}
          aria-valuenow={resolvedServiceWidth}
          tabIndex={0}
          onPointerDown={startResize}
          onKeyDown={(event) => {
            if (event.key === "ArrowLeft")
              setServiceWidth(clamp(resolvedServiceWidth - 12, 132, 360));
            if (event.key === "ArrowRight")
              setServiceWidth(clamp(resolvedServiceWidth + 12, 132, 360));
          }}
        />
        <div className="webHost" ref={host}>
          {embedded ? null : platform === "browser-preview" && frameable ? (
            <iframe
              key={`${site}:${targetUrl}`}
              title={official[site].name}
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
              <b>{official[site].name}</b>
              <p>
                {platform === "browser-preview"
                  ? "T2SCHOLA does not allow embedding in a browser."
                  : desktopBridge
                    ? "This build opens school pages in a separate window."
                    : "School pages need the desktop app."}
              </p>
              <UiButton className="button primary" onClick={openOutside}>
                Open {official[site].name} ↗
              </UiButton>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
