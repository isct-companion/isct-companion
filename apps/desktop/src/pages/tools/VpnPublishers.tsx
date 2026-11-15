import { Pencil, X } from "lucide-react";
import { useState, type FormEvent } from "react";
import { t } from "../../i18n";
import { Popover, UiButton, UiInput } from "../../components/ui";
import type { VpnStatus } from "../../platform/desktop-bridge";

/**
 * The publisher sites the campus VPN carries, edited from a pencil beside
 * the card's route line. Core normalizes what is typed (a pasted article
 * address becomes its host), so an entry that does not show up afterwards
 * is one it could not read or already had.
 */
export function VpnPublishers({
  publishers,
  isDefault,
  onChange,
}: {
  publishers: string[];
  isDefault: boolean;
  onChange: (list: string[] | undefined) => Promise<VpnStatus | undefined>;
}) {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState("");
  const [invalid, setInvalid] = useState(false);
  const [busy, setBusy] = useState(false);

  const change = async (list: string[] | undefined) => {
    setBusy(true);
    try {
      return await onChange(list);
    } finally {
      setBusy(false);
    }
  };

  const add = async (event: FormEvent) => {
    event.preventDefault();
    if (!draft.trim()) {
      setInvalid(true);
      return;
    }
    const next = await change([...publishers, draft]);
    if (next && (next.publishers?.length ?? 0) > publishers.length) {
      setDraft("");
      setInvalid(false);
    } else setInvalid(true);
  };

  return (
    <Popover opened={open} onChange={setOpen} position="bottom-end" width={300}>
      <Popover.Target>
        <button
          type="button"
          className="toolCardEdit"
          aria-label={t("tools.campusWifi.vpnEditPublishers")}
        >
          <Pencil aria-hidden size={13} strokeWidth={1.75} />
        </button>
      </Popover.Target>
      <Popover.Dropdown className="vpnPublishers">
        <h4>{t("tools.campusWifi.vpnPublishersTitle")}</h4>
        <p>{t("tools.campusWifi.vpnPublishersHint")}</p>
        <ul>
          {publishers.map((host) => (
            <li key={host} className="uiTag">
              {host}
              <button
                type="button"
                disabled={busy}
                aria-label={t("tools.campusWifi.vpnPublisherRemove", { host })}
                onClick={() =>
                  void change(publishers.filter((entry) => entry !== host))
                }
              >
                <X aria-hidden size={12} strokeWidth={2} />
              </button>
            </li>
          ))}
        </ul>
        <form onSubmit={(event) => void add(event)}>
          <UiInput
            value={draft}
            placeholder="nature.com"
            aria-invalid={invalid || undefined}
            onChange={(event) => {
              setDraft(event.currentTarget.value);
              setInvalid(false);
            }}
          />
          <UiButton type="submit" className="button small" disabled={busy}>
            {t("tools.campusWifi.vpnPublisherAdd")}
          </UiButton>
        </form>
        {invalid && (
          <p className="vpnPublishersError" role="alert">
            {t("tools.campusWifi.vpnPublisherInvalid")}
          </p>
        )}
        {!isDefault && (
          <button
            type="button"
            className="vpnPublishersReset"
            disabled={busy}
            onClick={() => void change(undefined)}
          >
            {t("tools.campusWifi.vpnPublishersReset")}
          </button>
        )}
      </Popover.Dropdown>
    </Popover>
  );
}
