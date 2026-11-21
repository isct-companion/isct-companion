import { useCallback, useEffect, useState } from "react";
import { Inbox, Upload } from "lucide-react";
import {
  Button,
  EmptyState,
  ErrorBanner,
  SegmentedControl,
  Skeleton,
  Tag,
} from "../../components/ui";
import { confirmAction } from "../../components/confirm";
import { lookup, t } from "../../i18n";
import type {
  ExamRequest,
  ExamRequestCursor,
  ExamRequestPage,
} from "../../generated/persistence";
import {
  Row,
  Workspace,
  subjectLabel,
  useReload,
  type OpenUpload,
  type ViewProps,
} from "./ExamList";

const requestFacts = (item: ExamRequest) =>
  [
    item.academicYear,
    item.occasion && lookup(`examShare.occasion.${item.occasion}`),
    item.wantedRole && lookup(`examShare.role.${item.wantedRole}`),
  ]
    .filter(Boolean)
    .join(" · ");

export function RequestList({
  call,
  open,
  opening,
  reading,
  pane,
  generation,
  setViewUrl,
  closeReader,
  openUpload,
}: ViewProps & { closeReader: () => void; openUpload: OpenUpload }) {
  const [mine, setMine] = useState(false);
  const [selected, setSelected] = useState<string>();
  const [page, setPage] = useState<ExamRequestPage & { mine: boolean }>();
  const [failed, setFailed] = useState(false);
  const [more, setMore] = useState(false);
  const show = useCallback(
    (personal: boolean, value: ExamRequestPage) =>
      setPage({ ...value, mine: personal }),
    [],
  );
  const load = useCallback(
    async (personal: boolean, cursor: ExamRequestCursor | null = null) => {
      setFailed(false);
      const answer = await call({ kind: "requests", mine: personal, cursor });
      if (answer?.kind !== "requests") return setFailed(!cursor);
      setPage((previous) => ({
        ...answer.value,
        mine: personal,
        items:
          cursor && previous?.mine === personal
            ? [...previous.items, ...answer.value.items]
            : answer.value.items,
      }));
    },
    [call],
  );
  useEffect(() => {
    void load(mine);
  }, [mine, load]);
  useReload(generation, () => void load(mine));
  const items = page?.mine === mine ? page.items : undefined;
  const shownUrl = page?.webUrl;
  useEffect(() => setViewUrl(shownUrl), [shownUrl, setViewUrl]);
  const chosen = items?.find((item) => item.id === selected);

  return (
    <Workspace>
      <div className="examPane">
        <SegmentedControl
          value={mine ? "mine" : "open"}
          onChange={(value) => setMine(value === "mine")}
          data={[
            { value: "open", label: t("examShare.openRequests") },
            { value: "mine", label: t("examShare.myRequests") },
          ]}
        />
        <div className="scroll">
          {failed ? (
            <ErrorBanner onRetry={() => void load(mine)}>
              {t("examShare.loadFailed")}
            </ErrorBanner>
          ) : !items ? (
            <Skeleton count={5} />
          ) : items.length ? (
            <>
              <ul className="examList">
                {items.map((item) => (
                  <Row
                    key={item.id}
                    title={
                      <>
                        {subjectLabel(item.subject)}
                        {item.status !== "open" && (
                          <Tag
                            tone={
                              item.status === "fulfilled" ? "ok" : "neutral"
                            }
                          >
                            {lookup(`examShare.requestStatus.${item.status}`) ??
                              item.status}
                          </Tag>
                        )}
                      </>
                    }
                    meta={
                      [requestFacts(item), item.description]
                        .filter(Boolean)
                        .join(" · ") || item.subject.code
                    }
                    current={selected === item.id}
                    onClick={() => {
                      setSelected(item.id);
                      closeReader();
                    }}
                  />
                ))}
              </ul>
              {page?.cursor && (
                <Button
                  variant="subtle"
                  className="examMore"
                  loading={more}
                  onClick={async () => {
                    setMore(true);
                    await load(mine, page.cursor);
                    setMore(false);
                  }}
                >
                  {t("examShare.more")}
                </Button>
              )}
            </>
          ) : (
            <EmptyState compact>
              {t(mine ? "examShare.noMyRequests" : "examShare.noOpenRequests")}
            </EmptyState>
          )}
        </div>
      </div>
      <div className="examPane examDetail">
        {pane ||
          (chosen ? (
            <RequestDetail
              item={chosen}
              opening={opening === chosen.answerExamId}
              reading={!!chosen.answerExamId && reading === chosen.answerExamId}
              view={() => chosen.answerExamId && open(chosen.answerExamId)}
              answer={() => openUpload({ kind: "request", request: chosen })}
              setOpen={async (next) => {
                const answer = await call({
                  kind: "setRequestOpen",
                  id: chosen.id,
                  open: next,
                  mine,
                });
                if (answer?.kind === "requests") show(mine, answer.value);
              }}
              remove={async () => {
                if (
                  !(await confirmAction(
                    t("examShare.deleteRequest"),
                    t("common.delete"),
                  ))
                )
                  return;
                const answer = await call({
                  kind: "deleteRequest",
                  id: chosen.id,
                  mine,
                });
                if (answer?.kind === "requests") {
                  setSelected(undefined);
                  show(mine, answer.value);
                }
              }}
            />
          ) : (
            <EmptyState icon={Inbox}>{t("examShare.pickRequest")}</EmptyState>
          ))}
      </div>
    </Workspace>
  );
}

/** One request in full: what is wanted and from whom, what it is for, and
 * what to do next: share a file, see the shared one, or manage one's own. */
function RequestDetail({
  item,
  opening,
  reading,
  view,
  answer,
  setOpen,
  remove,
}: {
  item: ExamRequest;
  opening: boolean;
  reading: boolean;
  view: () => void;
  answer: () => void;
  setOpen: (open: boolean) => Promise<void>;
  remove: () => Promise<void>;
}) {
  const facts = requestFacts(item);
  return (
    <article className="examRequest">
      <header>
        <small>{item.subject.code}</small>
        <h2>{subjectLabel(item.subject)}</h2>
        <Tag
          tone={
            item.status === "fulfilled"
              ? "ok"
              : item.status === "open"
                ? "accent"
                : "neutral"
          }
        >
          {lookup(`examShare.requestStatus.${item.status}`) ?? item.status}
        </Tag>
      </header>
      {facts && <p className="examRequestFacts">{facts}</p>}
      {item.teachers.length > 0 && (
        <p className="examRequestFacts">
          {item.teachers.map(({ name }) => name).join(" / ")}
        </p>
      )}
      {item.description && (
        <p className="examRequestText">{item.description}</p>
      )}
      <div className="examRequestActions">
        {item.answerExamId && (
          <Button onClick={view} loading={opening} disabled={reading}>
            {t("examShare.viewAnswer")}
          </Button>
        )}
        {!item.mine && item.status === "open" && (
          <Button onClick={answer}>
            <Upload size={15} strokeWidth={1.75} aria-hidden />
            {t("examShare.upload.answerRequest")}
          </Button>
        )}
        {item.mine && (
          <>
            <Button
              variant="default"
              onClick={() => void setOpen(item.status !== "open")}
            >
              {t(
                item.status === "open"
                  ? "examShare.closeRequest"
                  : "examShare.reopenRequest",
              )}
            </Button>
            <Button variant="subtle" color="red" onClick={() => void remove()}>
              {t("common.delete")}
            </Button>
          </>
        )}
      </div>
    </article>
  );
}
