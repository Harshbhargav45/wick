import { HermesClient } from "@pythnetwork/hermes-client";
import { config } from "./config.mjs";

const client = new HermesClient(config.hermesBaseUrl);

export async function fetchLatestVaa() {
  const feed = `0x${config.feedId}`;
  const updates = await client.getLatestPriceUpdates([feed], {
    encoding: "base64",
  });
  if (!updates || !updates.parsed || updates.parsed.length === 0) {
    throw new Error("hermes returned no price updates");
  }
  const p = updates.parsed[0];
  const vaa = updates.binary?.data?.[0];
  if (!vaa) {
    throw new Error("hermes returned no VAA binary (legacy endpoint requires base64 encoding)");
  }
  return {
    vaa,
    feedId: p.id,
    price: p.price.price,
    expo: p.price.expo,
    publishTime: p.price.publish_time,
  };
}
