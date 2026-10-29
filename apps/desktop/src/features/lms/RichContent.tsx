import { memo, useEffect, useMemo, useRef, useState } from "react";
import { t } from "../../i18n";
import { safeContent, safeUrl } from "./content";
import { LmsIcon } from "./LmsIcon";
import {
  cachedThumbnail,
  dropThumbnail,
  loadThumbnail,
  onSignedIn,
} from "./thumbnails";
import type { CourseFolder } from "./files";

export type FileAccess = { context: CourseFolder; limit: number };

export const ImageThumbnail = memo(function ImageThumbnail({
  url,
  files,
}: {
  url: string;
  files: FileAccess;
}) {
  const element = useRef<HTMLSpanElement>(null);
  const [src, setSrc] = useState<string | undefined>(() =>
    cachedThumbnail(files.context, url),
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
    const load = () =>
      void loadThumbnail(files.context, url, files.limit)
        .then((value) => {
          if (!disposed && value) setSrc(value);
        })
        .catch(() => undefined);
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
            dropThumbnail(url);
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
}: {
  files: FileAccess;
  html: string;
  language?: "ja" | "en";
  openLink: (url: string, title: string, anchor?: Element) => void;
}) {
  const content = useMemo(() => safeContent(html, language), [html, language]);
  const renderedHtml = useMemo(() => ({ __html: content }), [content]);
  const element = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let disposed = false;
    const load = async (placeholder: HTMLElement) => {
      const url = placeholder.dataset.imageSrc!;
      placeholder.dataset.state = "loading";
      try {
        const src = await loadThumbnail(files.context, url, files.limit);
        if (disposed) return;
        if (!src) {
          placeholder.dataset.state = "unavailable";
          return;
        }
        const image = document.createElement("img");
        image.alt = placeholder.dataset.imageAlt || "";
        image.decoding = "async";
        image.className = "lmsInlineThumbnail";
        image.onload = () => {
          placeholder.dataset.state = "ready";
        };
        image.onerror = () => {
          dropThumbnail(url);
          image.remove();
          placeholder.dataset.state = "unavailable";
        };
        image.src = src;
        placeholder.replaceChildren(image);
      } catch (error) {
        if (!disposed) {
          placeholder.dataset.state = "unavailable";
          placeholder.title = String(error);
        }
      }
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
                  void load(entry.target as HTMLElement);
                }
            },
            { rootMargin: "80px" },
          );
    for (const placeholder of placeholders) {
      if (observer) observer.observe(placeholder);
      else void load(placeholder);
    }
    const stop = onSignedIn(() => {
      for (const placeholder of placeholders)
        if (placeholder.dataset.state === "unavailable") void load(placeholder);
    });
    return () => {
      disposed = true;
      observer?.disconnect();
      stop();
    };
  }, [content, files]);
  return (
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
            .imageAlt || link.querySelector(".lmsAttachmentName")?.textContent;
        if (url) openLink(url, name || link.textContent || t("lms.link"), link);
      }}
    />
  );
});
