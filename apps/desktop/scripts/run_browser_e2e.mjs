#!/usr/bin/env node

import { execFileSync, spawnSync } from "node:child_process";
import { constants } from "node:fs";
import {
  access,
  chmod,
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const wdioCli = join(desktopRoot, "node_modules", "@wdio", "cli", "bin", "wdio.js");
const chromeForTestingOrigin = "https://storage.googleapis.com";
const maxChromeDriverArchiveBytes = 32 * 1024 * 1024;

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
    const response = await fetch(url, { redirect: "error" });
    if (!response.ok) {
      throw new Error(`ChromeDriver download failed (${response.status} ${response.statusText})`);
    }
    const responseUrl = new URL(response.url);
    if (responseUrl.origin !== chromeForTestingOrigin || responseUrl.href !== url) {
      throw new Error(`ChromeDriver download returned an unexpected URL: ${responseUrl.href}`);
    }
    const declaredLength = Number(response.headers.get("content-length"));
    if (Number.isFinite(declaredLength) && declaredLength > maxChromeDriverArchiveBytes) {
      throw new Error(`ChromeDriver archive is unexpectedly large (${declaredLength} bytes)`);
    }
    const bytes = Buffer.from(await response.arrayBuffer());
    if (bytes.length === 0 || bytes.length > maxChromeDriverArchiveBytes) {
      throw new Error(`ChromeDriver archive has an invalid size (${bytes.length} bytes)`);
    }
    if (bytes[0] !== 0x50 || bytes[1] !== 0x4b) {
      throw new Error("ChromeDriver download is not a ZIP archive");
    }
    await writeFile(archive, bytes, { mode: 0o600 });

    const expectedEntry = `chromedriver-${platform}/chromedriver`;
    const extractedRoot = join(scratch, "extracted");
    await mkdir(extractedRoot);
    const unzip = spawnSync("unzip", ["-q", archive, expectedEntry, "-d", extractedRoot], {
      stdio: "inherit",
    });
    if (unzip.status !== 0) {
      throw new Error(`unzip failed with exit code ${unzip.status ?? "unknown"}`);
    }
    const extractedDriver = join(extractedRoot, expectedEntry);
    const extractedStat = await lstat(extractedDriver);
    if (!extractedStat.isFile() || extractedStat.isSymbolicLink()) {
      throw new Error("ChromeDriver archive did not contain the expected regular file");
    }
    await mkdir(dirname(driver), { recursive: true });
    const stagedDriver = `${driver}.tmp-${process.pid}`;
    await copyFile(extractedDriver, stagedDriver);
    await chmod(stagedDriver, 0o755);
    await rename(stagedDriver, driver);
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
