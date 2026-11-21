import { useRef, useState } from "react";
import { Button, Group, Modal, Stack, Text } from "./ui";
import { t, type MessageKey } from "../i18n";
import guides from "../../../../resources/otp-guide/otp-guide.json";
import "./otp-guide.css";

/**
 * Step-by-step OTP setup guides (slotledger `approach/otp-setup-guide`): the
 * same screenshots and step list as the phone apps, from
 * `resources/otp-guide`. Each step is a screenshot, where to tap on it (as a
 * fraction of its width and height) or `screenshot` when the person should
 * capture that screen, and a caption key.
 */
export type OtpGuideId = keyof typeof guides;
type Step = {
  image: string;
  tap?: { x: number; y: number };
  action?: "screenshot";
  caption: string;
};

const IMAGES = import.meta.glob<string>(
  "../../../../resources/otp-guide/*.webp",
  {
    eager: true,
    query: "?url",
    import: "default",
  },
);
const image = (name: string) =>
  IMAGES[`../../../../resources/otp-guide/${name}`];

const TITLES: Record<OtpGuideId, MessageKey> = {
  google: "otpGuide.google.title",
  titechApp: "otpGuide.titechApp.title",
};

/** "Show steps" for an import guide. */
export function OtpGuideButton({ guide }: { guide: OtpGuideId }) {
  const steps = guides[guide] as Step[];
  const [index, setIndex] = useState<number | null>(null);
  const step = index === null ? null : steps[index];
  const last = index === steps.length - 1;
  const next = useRef<HTMLButtonElement>(null);
  const go = (to: number) => {
    // Back disables itself on the first step; focus moves to Next rather
    // than falling out of the dialog.
    if (to === 0) next.current?.focus();
    setIndex(to);
  };
  return (
    <>
      <Button variant="subtle" size="xs" onClick={() => setIndex(0)}>
        {t("otpGuide.showSteps")}
      </Button>
      {step && index !== null && (
        <Modal
          opened
          centered
          onClose={() => setIndex(null)}
          title={t(TITLES[guide])}
        >
          {/* The arrow keys page like the phone's swipe; Next holds focus, so
              Space and Return step through without the pointer. */}
          <Stack
            gap="space-5"
            align="center"
            onKeyDown={(event) => {
              if (event.key === "ArrowRight" && !last) go(index + 1);
              else if (event.key === "ArrowLeft" && index > 0) go(index - 1);
              else return;
              event.preventDefault();
            }}
          >
            <div className="otpGuideShot" data-action={step.action}>
              <img src={image(step.image)} alt="" />
              {step.action === "screenshot" && (
                <img
                  className="otpGuideThumb"
                  src={image(step.image)}
                  alt=""
                  aria-hidden
                />
              )}
              {step.tap && (
                <span
                  className="otpGuideTap"
                  aria-hidden
                  style={{
                    left: `${step.tap.x * 100}%`,
                    top: `${step.tap.y * 100}%`,
                  }}
                />
              )}
            </div>
            {/* Every caption sits in one cell, so the tallest sets the height
                and the buttons stay put from step to step. */}
            <div className="otpGuideCaptions">
              {steps.map((each, at) => (
                <Text
                  key={at}
                  size="sm"
                  aria-hidden={at !== index || undefined}
                  data-current={at === index || undefined}
                >
                  {t(each.caption as MessageKey)}
                </Text>
              ))}
            </div>
            <Group justify="space-between" style={{ width: "100%" }}>
              <Text size="xs" c="dimmed">
                {t("otpGuide.stepOf", {
                  current: String(index + 1),
                  total: String(steps.length),
                })}
              </Text>
              <Group gap="space-3" className="otpGuideNav">
                <Button
                  variant="default"
                  size="xs"
                  disabled={index === 0}
                  onClick={() => go(index - 1)}
                >
                  {t("common.back")}
                </Button>
                <Button
                  ref={next}
                  size="xs"
                  autoFocus
                  onClick={() => (last ? setIndex(null) : go(index + 1))}
                >
                  {t(last ? "common.done" : "common.next")}
                </Button>
              </Group>
            </Group>
          </Stack>
        </Modal>
      )}
    </>
  );
}
