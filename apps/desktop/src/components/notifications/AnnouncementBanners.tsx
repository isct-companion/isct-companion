import type { AnnouncementView } from "../../generated/persistence";
import { t } from "../../i18n";
import { ExternalLink } from "../ExternalLink";
import { Stack, Text } from "../ui";
import { Banner } from "./Banner";
import "./announcement-banners.css";

function AnnouncementBanner({ item }: { item: AnnouncementView }) {
  return (
    <Banner
      className="announcementBanner"
      color={item.level === "important" ? "red" : undefined}
    >
      <Stack gap="space-3">
        <Text fw={600}>{item.title}</Text>
        <Text size="sm" style={{ whiteSpace: "pre-wrap" }}>
          {item.body}
        </Text>
        {item.link && (
          <ExternalLink href={item.link.url}>{item.link.label}</ExternalLink>
        )}
      </Stack>
    </Banner>
  );
}

export function AnnouncementBanners({
  items,
}: {
  items?: readonly AnnouncementView[];
}) {
  const [first, ...others] = items ?? [];
  if (!first) return null;
  return (
    <Stack gap="space-3">
      <AnnouncementBanner item={first} />
      {others.length > 0 && (
        <details>
          <summary>{t("announcements.more", { count: others.length })}</summary>
          <Stack gap="space-3" mt="space-3">
            {others.map((item) => (
              <AnnouncementBanner key={item.id} item={item} />
            ))}
          </Stack>
        </details>
      )}
    </Stack>
  );
}
