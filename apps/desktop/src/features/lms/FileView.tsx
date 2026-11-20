import { lazy, Suspense, useEffect, useState, type ReactNode } from "react";
import type { Viewer } from "../../generated/persistence";
import { t } from "../../i18n";
import { previewOf } from "./file-types";
import { localFileUrl } from "./files";
import { ImageView } from "./ImageView";
import { ReaderTools } from "./ReaderPane";
import { useReleaseSource } from "./useReleaseSource";

const PdfDocument = lazy(() =>
  import("./PdfDocument").then((m) => ({ default: m.PdfDocument })),
);
const TextDocument = lazy(() =>
  import("./TextDocument").then((m) => ({ default: m.TextDocument })),
);
const ReadableMarkdown = lazy(() =>
  import("./ReadableMarkdown").then((m) => ({ default: m.ReadableMarkdown })),
);

const loading = (
  <p className="lmsQuiet lmsLoading">{t("lms.loadingPreview")}</p>
);

/**
 * A downloaded file in the reader the way Core says (`Viewer`), for every
 * reader that shows one: course files and past exams alike. `toolbar` sits
 * in the reader's tool slot, or with the PDF's own tools. A viewer the
 * reader does not draw shows `fallback`; so does a file that cannot be read.
 */
export function FileView({
  viewer,
  path,
  name,
  title,
  text,
  toolbar,
  fallback,
  onBroken,
}: {
  viewer: Viewer;
  path: string;
  /** The file's name, which picks a text file's highlighting. */
  name: string;
  title: string;
  /** The file's text when Core already read it; read here otherwise. */
  text?: string;
  toolbar: ReactNode;
  fallback: ReactNode;
  /** An image the webview cannot decode. */
  onBroken?: () => void;
}) {
  const src = localFileUrl(path);
  if (viewer === "pdf")
    return (
      <Suspense fallback={loading}>
        <PdfDocument src={src} title={title} fileToolbar={toolbar} />
      </Suspense>
    );
  if (!previewOf(viewer)) return <>{fallback}</>;
  return (
    <div className="lmsAttachmentPreview">
      <ReaderTools>{toolbar}</ReaderTools>
      {viewer === "image" ? (
        <ImageView src={src} title={title} onBroken={onBroken} />
      ) : viewer === "video" ? (
        <div className="lmsImageStage">
          <MediaView kind="video" src={src} />
        </div>
      ) : viewer === "audio" ? (
        <div className="lmsAudioStage">
          <MediaView kind="audio" src={src} />
        </div>
      ) : (
        <TextFile src={src} text={text} fallback={fallback}>
          {(value) => (
            <Suspense fallback={loading}>
              {viewer === "markdown" ? (
                <ReadableMarkdown text={value} />
              ) : (
                <TextDocument value={value} name={name} />
              )}
            </Suspense>
          )}
        </TextFile>
      )}
    </div>
  );
}

/** A text file's contents: given, or read whole from its local URL (Core
 * keeps text in the reader under its size limit). */
function TextFile({
  src,
  text,
  fallback,
  children,
}: {
  src: string;
  text?: string;
  fallback: ReactNode;
  children: (text: string) => ReactNode;
}) {
  const [read, setRead] = useState<string | null | undefined>(text);
  useEffect(() => {
    if (text !== undefined) return setRead(text);
    setRead(undefined);
    const controller = new AbortController();
    void fetch(src, { signal: controller.signal })
      .then((response) => {
        if (!response.ok) throw new Error(String(response.status));
        return response.text();
      })
      .then(setRead, () => {
        if (!controller.signal.aborted) setRead(null);
      });
    return () => controller.abort();
  }, [src, text]);
  if (read === undefined) return loading;
  if (read === null) return <>{fallback}</>;
  return <>{children(read)}</>;
}

function MediaView({ kind, src }: { kind: "video" | "audio"; src: string }) {
  const media = useReleaseSource<HTMLMediaElement>(src);
  return kind === "video" ? (
    <video
      ref={media as React.RefObject<HTMLVideoElement>}
      className="lmsMediaPreview"
      src={src}
      controls
      preload="metadata"
    />
  ) : (
    <audio
      ref={media as React.RefObject<HTMLAudioElement>}
      src={src}
      controls
      preload="metadata"
    />
  );
}
