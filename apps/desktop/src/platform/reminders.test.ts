import { expect, it, vi } from "vitest";
import { bootstrapReminders } from "./reminders";

it("moves the legacy log only after Core has persisted it", async () => {
  const order: string[] = [];
  const fired = { "class:abc": 123 };
  const storage = {
    getItem: () => JSON.stringify(fired),
    removeItem: vi.fn(() => {
      order.push("remove");
    }),
  };
  const bridge = {
    bootstrapReminders: vi.fn(async (value) => {
      expect(value).toEqual(fired);
      order.push("persist");
      return true;
    }),
  };
  await bootstrapReminders(bridge, storage);
  expect(order).toEqual(["persist", "remove"]);
  expect(storage.removeItem).toHaveBeenCalledWith("isct-reminders-fired");
});
it("keeps the legacy record when native persistence fails", async () => {
  const storage = { getItem: () => '{"class:abc":123}', removeItem: vi.fn() };
  await expect(
    bootstrapReminders(
      {
        bootstrapReminders: async () => {
          throw new Error("disk full");
        },
      },
      storage,
    ),
  ).rejects.toThrow("disk full");
  expect(storage.removeItem).not.toHaveBeenCalled();
});
it("transfers an empty log when the legacy browser storage is invalid", async () => {
  const bootstrap = vi.fn(async () => true);
  await bootstrapReminders(
    { bootstrapReminders: bootstrap },
    { getItem: () => "invalid", removeItem: () => undefined },
  );
  expect(bootstrap).toHaveBeenCalledWith({});
});
