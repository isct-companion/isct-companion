import { useLocalStorage } from "@mantine/hooks";
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
import { desktopBridge, schoolWebExit, official } from "../../app/native";
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
  { frequency: "low", sites: ["library"] },
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

const siteForAddress = (raw: string): SchoolSite | undefined => {
  const host = new URL(raw).hostname;
  if (host === "lms.s.isct.ac.jp" || host === "t2schola.titech.ac.jp")
    return "science-lms";
  if (host === "isct.ex-tic.com" || host === "portal.isct.ac.jp")
    return "science-portal";
  if (host === "portal.titech.ac.jp" || host === "portal.nap.gsic.titech.ac.jp")
    return "tokyo-portal";
  if (host === "mailv4.m.titech.ac.jp") return "tokyo-mail";
  if (host === "kyomu0.gakumu.titech.ac.jp") return "kyomu";
  if (host === "request.libra.titech.ac.jp") return "library";
  if (host === "syllabus.s.isct.ac.jp") return "syllabus";
  return undefined;
};

export function Web({
  platform,
  data,
  notify,
  schoolWebAvailable,
  requestedUrl,
  returnLabel,
  onReturn,
}: {
  platform: Platform;
  data: Workspace;
  notify: (s: string) => void;
  schoolWebAvailable: boolean;
  requestedUrl?: string;
  returnLabel?: string;
  onReturn?: () => void;
}) {
  const [site, setSite] = useState<SchoolSite>(() => {
    if (requestedUrl) return "science-lms";
    const active = sessionStorage.getItem("stws-active-school-site");
    if (active && active in official) return active as SchoolSite;
    if (localStorage.getItem("stws-restore-school-page") !== "true")
      return "science-portal";
    const saved = localStorage.getItem("stws-active-school-site");
    return saved && saved in official
      ? (saved as SchoolSite)
      : "science-portal";
  });
  const initialUrl = requestedUrl ?? official[site].url;
  const [status, setStatus] = useState("");
  const [currentUrl, setCurrentUrl] = useState(initialUrl);
  const [address, setAddress] = useState(initialUrl);
  const [serviceWidth, setServiceWidth] = useLocalStorage<number>({
    key: "stws-school-service-width",
    defaultValue: 168,
  });
  const [zoom, setZoom] = useLocalStorage<number>({
    key: "stws-school-web-zoom",
    defaultValue: 1,
  });
  const resolvedServiceWidth = clamp(serviceWidth, 132, 360);
  const resolvedZoom = clamp(zoom, 0.5, 2);
  const host = useRef<HTMLDivElement>(null);
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
    if (localStorage.getItem("stws-restore-school-page") !== "true")
      localStorage.removeItem("stws-active-school-site");
    void desktopBridge.configureSchoolWebPolicy({
      restoreAcrossRestarts:
        localStorage.getItem("stws-restore-school-page") === "true",
      idleMinutes: 10,
    });
  }, [schoolWebAvailable]);

  useEffect(
    () =>
      desktopBridge?.onSchoolWebUrl?.((url) => {
        setCurrentUrl(url);
        if (!editingAddress.current) setAddress(url);
      }),
    [],
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
    if (!embedded || !desktopBridge) return;
    let cancelled = false;
    const customizationSite = official[site].customization;
    const custom = data.settings.customization?.sites[customizationSite];
    const permitted =
      platform === "native-desktop" &&
      localStorage.getItem(`stws-site-consent-${customizationSite}`) === "true";
    setStatus("Opening…");
    void desktopBridge
      .openSchoolSite(
        site,
        permitted ? (custom?.css ?? "") : "",
        permitted ? (custom?.js ?? "") : "",
        bounds(),
        site === "science-lms" ? requestedUrl : undefined,
      )
      .then(async (state) => {
        if (cancelled) return;
        setCurrentUrl(state.url);
        if (!editingAddress.current) setAddress(state.url);
        setStatus(
          state.phase === "reconnectRequired" ? "Sign-in required" : "",
        );
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
  }, [site, embedded]);

  const choose = (next: SchoolSite) => {
    if (schoolWebExit.pending) return;
    sessionStorage.setItem("stws-active-school-site", next);
    if (localStorage.getItem("stws-restore-school-page") === "true")
      localStorage.setItem("stws-active-school-site", next);
    const nextUrl = official[next].url;
    setCurrentUrl(nextUrl);
    setAddress(nextUrl);
    setSite(next);
  };

  const navigate = async (event: FormEvent) => {
    event.preventDefault();
    try {
      const target = normalizeAddress(address);
      const destination = siteForAddress(target);
      if (!destination)
        throw new Error("Use an official Science Tokyo school URL.");
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
            localStorage.getItem(`stws-site-consent-${customizationSite}`) ===
              "true";
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
        sessionStorage.setItem("stws-active-school-site", destination);
        if (localStorage.getItem("stws-restore-school-page") === "true")
          localStorage.setItem("stws-active-school-site", destination);
        setSite(destination);
      }
      setCurrentUrl(target);
      setStatus("");
    } catch (error) {
      setStatus(error instanceof Error ? error.message : "Invalid address");
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
        localStorage.getItem(`stws-site-consent-${customizationSite}`) ===
        "true";
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
