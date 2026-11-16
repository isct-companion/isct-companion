import { memo, useEffect, useMemo, useRef, useState } from "react";
import { t } from "../../i18n";
import { safeContent, safeUrl } from "./content";
import { LmsIcon } from "./LmsIcon";
import { onSignedIn, thumbnailUrl } from "./thumbnails";

/** The course whose files a page's pictures come from. */
export type FileAccess = { courseId: string };

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
