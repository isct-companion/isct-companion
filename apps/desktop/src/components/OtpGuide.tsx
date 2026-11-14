import { useState } from "react";
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
          <Stack gap="sm" align="center">
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
            <Text size="sm" className="otpGuideCaption">
              {t(step.caption as MessageKey)}
            </Text>
            <Group justify="space-between" style={{ width: "100%" }}>
              <Text size="xs" c="dimmed">
                {t("otpGuide.stepOf", {
                  current: String(index + 1),
                  total: String(steps.length),
                })}
              </Text>
              <Group gap={6}>
                <Button
                  variant="default"
                  size="xs"
                  disabled={index === 0}
                  onClick={() => setIndex(index - 1)}
                >
                  {t("common.back")}
                </Button>
                <Button
                  size="xs"
                  onClick={() => setIndex(last ? null : index + 1)}
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
