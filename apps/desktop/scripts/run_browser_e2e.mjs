#!/usr/bin/env node

import { execFileSync, spawnSync } from "node:child_process";
import { constants } from "node:fs";
import { access, chmod, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const wdioCli = join(desktopRoot, "node_modules", "@wdio", "cli", "bin", "wdio.js");

async function isExecutable(path) {
  try {
    await access(path, constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

function installedChromeVersion() {
  const candidates = [
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Google Chrome Beta.app/Contents/MacOS/Google Chrome Beta",
    "/Applications/Google Chrome Dev.app/Contents/MacOS/Google Chrome Dev",
  ];
  for (const chrome of candidates) {
    try {
      const output = execFileSync(chrome, ["--version"], { encoding: "utf8" });
      const version = output.match(/\d+\.\d+\.\d+\.\d+/)?.[0];
      if (version) return version;
    } catch {
      // Try the next installed Chrome channel.
    }
  }
  throw new Error("Google Chrome is required for the desktop browser E2E suite");
}

async function prepareMacChromedriver() {
  if (process.env.CHROMEDRIVER_PATH) return process.env.CHROMEDRIVER_PATH;

  const version = installedChromeVersion();
  const platform = process.arch === "arm64" ? "mac-arm64" : "mac-x64";
  const installRoot = join(
    homedir(),
    ".cache",
    "thinclaw-webdriver",
    "manual",
    version,
  );
  const driver = join(installRoot, `chromedriver-${platform}`, "chromedriver");
  if (await isExecutable(driver)) return driver;

  await mkdir(installRoot, { recursive: true });
  const scratch = await mkdtemp(join(tmpdir(), "thinclaw-chromedriver-"));
  try {
    const archive = join(scratch, "chromedriver.zip");
    const url = `https://storage.googleapis.com/chrome-for-testing-public/${version}/${platform}/chromedriver-${platform}.zip`;
    const response = await fetch(url, { redirect: "follow" });
    if (!response.ok) {
      throw new Error(`ChromeDriver download failed (${response.status} ${response.statusText})`);
    }
    await writeFile(archive, Buffer.from(await response.arrayBuffer()));
    const unzip = spawnSync("unzip", ["-q", "-o", archive, "-d", installRoot], {
      stdio: "inherit",
    });
    if (unzip.status !== 0) {
      throw new Error(`unzip failed with exit code ${unzip.status ?? "unknown"}`);
    }
    await chmod(driver, 0o755);
    if (!(await isExecutable(driver))) {
      throw new Error(`ChromeDriver was not installed at ${driver}`);
    }
    return driver;
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
}

const env = { ...process.env };
if (process.platform === "darwin") {
  env.CHROMEDRIVER_PATH = await prepareMacChromedriver();
}

const result = spawnSync(process.execPath, [wdioCli, "run", "./wdio.browser.conf.ts"], {
  cwd: desktopRoot,
  env,
  stdio: "inherit",
});
if (result.error) throw result.error;
process.exitCode = result.status ?? 1;
