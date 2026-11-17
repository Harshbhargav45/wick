import path from "node:path";
import type { NextConfig } from "next";

const root = path.resolve(process.cwd());

const nextConfig: NextConfig = {
  // Pin the workspace root here so Next does not walk up past the repo and pick
  // up an unrelated package-lock.json in a parent directory.
  turbopack: { root },
  outputFileTracingRoot: root,
};

export default nextConfig;
