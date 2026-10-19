type NetworkInformation = {
  type?: string;
  saveData?: boolean;
  metered?: boolean;
  addEventListener?: (type: "change", listener: () => void) => void;
  removeEventListener?: (type: "change", listener: () => void) => void;
};

type SubscriptionNetwork = "offline" | "wifi" | "cellular";

function networkInformation(): NetworkInformation | undefined {
  return (navigator as Navigator & { connection?: NetworkInformation })
    .connection;
}

/** Unknown links are treated as unmetered. */
export function subscriptionNetwork(): SubscriptionNetwork {
  if (!navigator.onLine) return "offline";
  const connection = networkInformation();
  if (
    connection?.type === "cellular" ||
    connection?.saveData === true ||
    connection?.metered === true
  )
    return "cellular";
  return "wifi";
}
