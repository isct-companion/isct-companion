import type { ReactNode } from "react";
import {
  Group,
  Panel,
  Separator,
  useDefaultLayout,
} from "react-resizable-panels";
import { t } from "../i18n";
import "./resizable-workspace.css";

/** Layout stays mounted when a compact view takes over, preserving readers and editors. */
export function ResizableWorkspace({
  id,
  enabled,
  children,
  firstSize,
  secondSize,
  firstMin = "20%",
  secondMin = "30%",
  firstMax = "75%",
  secondMax = "80%",
}: {
  id: string;
  enabled: boolean;
  children: [ReactNode, ReactNode];
  firstSize?: number | string;
  secondSize?: number | string;
  firstMin?: number | string;
  secondMin?: number | string;
  firstMax?: number | string;
  secondMax?: number | string;
}) {
  const { defaultLayout, onLayoutChanged } = useDefaultLayout({
    id: `workspace:${id}`,
    onlySaveAfterUserInteractions: true,
  });
  return (
    <Group
      id={`workspace-${id}`}
      className="splitWorkspace"
      data-resizable={enabled}
      disabled={!enabled}
      defaultLayout={defaultLayout}
      onLayoutChanged={(layout, meta) => {
        if (enabled) onLayoutChanged(layout, meta);
      }}
    >
      <Panel
        id={`${id}-first`}
        className="splitPane"
        defaultSize={firstSize}
        groupResizeBehavior={
          typeof firstSize === "number"
            ? "preserve-pixel-size"
            : "preserve-relative-size"
        }
        minSize={firstMin}
        maxSize={firstMax}
      >
        {children[0]}
      </Panel>
      <Separator
        className="workspaceSeparator"
        aria-label={t("Resize workspace")}
        title={t("Drag to resize · Double-click to reset")}
      />
      <Panel
        id={`${id}-second`}
        className="splitPane"
        defaultSize={secondSize}
        groupResizeBehavior={
          typeof secondSize === "number"
            ? "preserve-pixel-size"
            : "preserve-relative-size"
        }
        minSize={secondMin}
        maxSize={secondMax}
      >
        {children[1]}
      </Panel>
    </Group>
  );
}
