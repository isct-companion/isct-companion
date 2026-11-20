import {
  memo,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import { t } from "../../i18n";
import { safeContent, safeUrl } from "./content";
import { LmsIcon } from "./LmsIcon";
import { onSignedIn, thumbnailUrl } from "./thumbnails";

/** The course whose files a page's pictures come from. */
export type FileAccess = { courseId: string };

/**
 * Links that are folders in the outline (a ZIP): `of` names the folder a
 * link's address is, `render` draws it. Such a link's card is replaced in
 * place by that folder.
 */
export type LinkedFolders = {
  of: (url: string) => string | undefined;
  render: (url: string, title: string) => ReactNode;
};

export const ImageThumbnail = memo(function ImageThumbnail({
  url,
  files,
}: {
  url: string;
  files: FileAccess;
}) {
  const element = useRef<HTMLSpanElement>(null);
  const [src, setSrc] = useState<string | undefined>(() =>
    thumbnailUrl(files.courseId, url),
  );
  const [loaded, setLoaded] = useState(false);
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  useEffect(
    () =>
      onSignedIn(() => {
        setFailed(false);
        setAttempt((value) => value + 1);
      }),
    [],
  );
  useEffect(() => {
    if (src || failed) return;
    let disposed = false;
    const load = () => {
      if (!disposed) setSrc(thumbnailUrl(files.courseId, url));
    };
    const observer =
      typeof IntersectionObserver === "undefined"
        ? undefined
        : new IntersectionObserver(
            (entries) => {
              if (entries.some((entry) => entry.isIntersecting)) {
                observer!.disconnect();
                load();
              }
            },
            { rootMargin: "200px" },
          );
    if (observer && element.current) observer.observe(element.current);
    else load();
    return () => {
      disposed = true;
      observer?.disconnect();
    };
  }, [url, src, failed, attempt, files]);
  return (
    <span
      ref={element}
      className={`lmsFileTile lmsThumbnail${loaded ? " loaded" : ""}`}
      data-family="image"
      aria-hidden="true"
    >
      <LmsIcon name="image" size={18} />
      {src && (
        <img
          src={src}
          alt=""
          decoding="async"
          onLoad={() => setLoaded(true)}
          onError={() => {
            setSrc(undefined);
            setLoaded(false);
            setFailed(true);
          }}
        />
      )}
    </span>
  );
});

export const RichContent = memo(function RichContent({
  html,
  language,
  openLink,
  files,
  folders,
}: {
  files: FileAccess;
  html: string;
  language?: "ja" | "en";
  openLink: (url: string, title: string, anchor?: Element) => void;
  folders?: LinkedFolders;
}) {
  const content = useMemo(() => safeContent(html, language), [html, language]);
  const renderedHtml = useMemo(() => ({ __html: content }), [content]);
  const element = useRef<HTMLDivElement>(null);
  const foldersNow = useRef(folders);
  foldersNow.current = folders;
  /** Where each folder link's row renders. The text is set as HTML, so the
   * rows render there through portals. */
  const [hosts, setHosts] = useState<
    { host: HTMLElement; url: string; title: string }[]
  >([]);
  useEffect(() => {
    const root = element.current;
    const of = foldersNow.current?.of;
    if (!root || !of) return;
    for (const link of root.querySelectorAll<HTMLAnchorElement>("a[href]")) {
      const folder = of(safeUrl(link.getAttribute("href")) ?? "");
      if (!folder) continue;
      const host = document.createElement("div");
      host.className = "lmsLinkedFolder";
      host.dataset.url = folder;
      host.dataset.title =
        link.querySelector(".lmsAttachmentName")?.textContent ||
        link.textContent ||
        folder;
      link.replaceWith(host);
    }
    // Hosts placed before (a remount keeps the DOM) are taken as they are.
    setHosts(
      [...root.querySelectorAll<HTMLElement>(".lmsLinkedFolder")].map(
        (host) => ({
          host,
          url: host.dataset.url!,
          title: host.dataset.title!,
        }),
      ),
    );
  }, [content]);
  useEffect(() => {
    const load = (placeholder: HTMLElement) => {
      placeholder.dataset.state = "loading";
      const image = document.createElement("img");
      image.alt = placeholder.dataset.imageAlt || "";
      image.decoding = "async";
      image.className = "lmsInlineThumbnail";
      image.onload = () => {
        placeholder.dataset.state = "ready";
      };
      image.onerror = () => {
        image.remove();
        placeholder.dataset.state = "unavailable";
      };
      image.src = thumbnailUrl(files.courseId, placeholder.dataset.imageSrc!);
      placeholder.replaceChildren(image);
    };
    const placeholders =
      element.current?.querySelectorAll<HTMLElement>("[data-image-src]") || [];
    const observer =
      typeof IntersectionObserver === "undefined"
        ? null
        : new IntersectionObserver(
            (entries) => {
              for (const entry of entries)
                if (entry.isIntersecting) {
                  observer!.unobserve(entry.target);
                  load(entry.target as HTMLElement);
                }
            },
            { rootMargin: "80px" },
          );
    for (const placeholder of placeholders) {
      if (observer) observer.observe(placeholder);
      else load(placeholder);
    }
    const stop = onSignedIn(() => {
      for (const placeholder of placeholders)
        if (placeholder.dataset.state === "unavailable") load(placeholder);
    });
    return () => {
      observer?.disconnect();
      stop();
    };
  }, [content, files]);
  return (
    <>
      {folders &&
        hosts.map(({ host, url, title }, index) =>
          createPortal(folders.render(url, title), host, `${index}:${url}`),
        )}
      <div
        ref={element}
        className="lmsRichContent"
        dangerouslySetInnerHTML={renderedHtml}
        onClick={(event) => {
          const link = (event.target as Element).closest("a");
          if (!link) return;
          event.preventDefault();
          const url = safeUrl(link.getAttribute("href"));
          const name =
            link.querySelector<HTMLElement>("[data-image-alt]")?.dataset
              .imageAlt ||
            link.querySelector(".lmsAttachmentName")?.textContent;
          if (url)
            openLink(url, name || link.textContent || t("lms.link"), link);
        }}
      />
    </>
  );
});
