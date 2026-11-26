import type { ComponentProps } from "react";
import { Alert } from "../ui";

export function Banner({
  role = "status",
  ...props
}: ComponentProps<typeof Alert>) {
  return <Alert role={role} {...props} />;
}
