import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import { openUrl } from "@tauri-apps/plugin-opener";

type Settings = {
  serverUrl: string;
  pollIntervalSecs: number;
  cookieHeader: string;
  basicAuthUser: string;
  basicAuthPassword: string;
  sleepGuardEnabled: boolean;
  startMonitorOnLaunch: boolean;
};

type RuntimeStatus = {
  running: boolean;
  sleepGuardActive: boolean;
  alarmActive: boolean;
  alarmMode?: string | null;
  alarmText?: string | null;
  alarmLink?: string | null;
  alarmCount: number;
  soundSuppressed: boolean;
  watchdogRestarts: number;
  mutedUntil?: string | null;
  lastCheck?: string | null;
  lastSuccess?: string | null;
  lastError?: string | null;
};

type LogEntry = {
  at: string;
  level: string;
  message: string;
};

type AppSnapshot = {
  settings: Settings;
  status: RuntimeStatus;
  logs: LogEntry[];
};

const $ = <T extends HTMLElement>(selector: string) => {
  const el = document.querySelector<T>(selector);
  if (!el) throw new Error(`Missing element ${selector}`);
  return el;
};

const els = {
  statusPill: $("#status-pill"),
  alarmState: $("#alarm-state"),
  alarmText: $("#alarm-text"),
  startBtn: $("#start-btn") as HTMLButtonElement,
  stopBtn: $("#stop-btn") as HTMLButtonElement,
  muteBtn: $("#mute-btn") as HTMLButtonElement,
  stopAlarmBtn: $("#stop-alarm-btn") as HTMLButtonElement,
  alarmLinkBtn: $("#alarm-link-btn") as HTMLButtonElement,
  lastCheck: $("#last-check"),
  lastSuccess: $("#last-success"),
  sleepState: $("#sleep-state"),
  mutedUntil: $("#muted-until"),
  watchdogRestarts: $("#watchdog-restarts"),
  form: $("#settings-form") as HTMLFormElement,
  serverUrl: $("#server-url") as HTMLInputElement,
  pollInterval: $("#poll-interval") as HTMLInputElement,
  cookieHeader: $("#cookie-header") as HTMLTextAreaElement,
  basicUser: $("#basic-user") as HTMLInputElement,
  basicPass: $("#basic-pass") as HTMLInputElement,
  sleepGuard: $("#sleep-guard") as HTMLInputElement,
  startOnLaunch: $("#start-on-launch") as HTMLInputElement,
  autostart: $("#autostart") as HTMLInputElement,
  openPortalBtn: $("#open-portal-btn") as HTMLButtonElement,
  testAlarmBtn: $("#test-alarm-btn") as HTMLButtonElement,
  testGentleBtn: $("#test-gentle-btn") as HTMLButtonElement,
  lastError: $("#last-error"),
  logBox: $("#log-box"),
};

let current: AppSnapshot | null = null;

function fmtDate(value?: string | null) {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return "—";
  return new Intl.DateTimeFormat("ru-RU", {
    day: "2-digit",
    month: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(date);
}

function setBusy(on: boolean) {
  [
    els.startBtn,
    els.stopBtn,
    els.muteBtn,
    els.stopAlarmBtn,
    els.testAlarmBtn,
    els.testGentleBtn,
  ].forEach((button) => {
    button.disabled = on;
  });
}

function formToSettings(): Settings {
  return {
    serverUrl: els.serverUrl.value.trim(),
    pollIntervalSecs: Math.max(1, Number.parseInt(els.pollInterval.value, 10) || 2),
    cookieHeader: els.cookieHeader.value.trim(),
    basicAuthUser: els.basicUser.value.trim(),
    basicAuthPassword: els.basicPass.value,
    sleepGuardEnabled: els.sleepGuard.checked,
    startMonitorOnLaunch: els.startOnLaunch.checked,
  };
}

function fillForm(settings: Settings) {
  if (document.activeElement && els.form.contains(document.activeElement)) return;

  els.serverUrl.value = settings.serverUrl;
  els.pollInterval.value = String(settings.pollIntervalSecs);
  els.cookieHeader.value = settings.cookieHeader;
  els.basicUser.value = settings.basicAuthUser;
  els.basicPass.value = settings.basicAuthPassword;
  els.sleepGuard.checked = settings.sleepGuardEnabled;
  els.startOnLaunch.checked = settings.startMonitorOnLaunch;
}

function render(snapshot: AppSnapshot) {
  current = snapshot;
  const { settings, status, logs } = snapshot;

  fillForm(settings);

  const activeClass = status.alarmActive ? "alarm" : status.running ? "running" : "stopped";
  els.statusPill.className = `status-pill ${activeClass}`;
  els.statusPill.textContent = status.alarmActive
    ? "Alarm active"
    : status.running
      ? "Monitoring"
      : "Stopped";

  els.alarmState.textContent = status.alarmActive
    ? `${(status.alarmMode || "alarm").toUpperCase()} · ${status.alarmCount}`
    : "No active alarm";
  els.alarmText.textContent =
    (status.soundSuppressed && status.alarmText
      ? `${status.alarmText} (sound stopped manually)`
      : status.alarmText) ||
    (status.running ? "Waiting for server events." : "Monitoring is not running yet.");
  els.alarmLinkBtn.hidden = !status.alarmLink;

  els.lastCheck.textContent = fmtDate(status.lastCheck);
  els.lastSuccess.textContent = fmtDate(status.lastSuccess);
  els.sleepState.textContent = status.sleepGuardActive ? "Active" : "Off";
  els.mutedUntil.textContent = fmtDate(status.mutedUntil);
  els.watchdogRestarts.textContent = String(status.watchdogRestarts);

  els.startBtn.disabled = status.running;
  els.stopBtn.disabled = !status.running;
  els.muteBtn.disabled = !status.alarmActive;
  els.stopAlarmBtn.disabled = !status.alarmActive || status.soundSuppressed;

  els.lastError.textContent = status.lastError || "No errors.";
  els.lastError.classList.toggle("has-error", Boolean(status.lastError));

  els.logBox.innerHTML = logs
    .slice(-120)
    .map(
      (entry) =>
        `<div class="log-line ${entry.level}"><time>${fmtDate(entry.at)}</time><span>${escapeHtml(
          entry.level,
        )}</span><p>${escapeHtml(entry.message)}</p></div>`,
    )
    .join("");
  els.logBox.scrollTop = els.logBox.scrollHeight;
}

function escapeHtml(value: string) {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

async function call(command: string, args?: Record<string, unknown>) {
  setBusy(true);
  try {
    const snapshot = await invoke<AppSnapshot>(command, args);
    render(snapshot);
  } catch (error) {
    els.lastError.textContent = String(error);
    els.lastError.classList.add("has-error");
    console.error(`Command ${command} failed`, error);
  } finally {
    setBusy(false);
  }
}

async function syncAutostartToggle() {
  try {
    els.autostart.checked = await isEnabled();
  } catch (error) {
    console.warn("Autostart state unavailable", error);
  }
}

window.addEventListener("DOMContentLoaded", async () => {
  await listen<AppSnapshot>("kds-state", (event) => render(event.payload));

  render(await invoke<AppSnapshot>("load_snapshot"));
  await syncAutostartToggle();

  els.form.addEventListener("submit", async (event) => {
    event.preventDefault();
    await call("save_settings", { settings: formToSettings() });
  });

  els.startBtn.addEventListener("click", () => call("start_monitor"));
  els.stopBtn.addEventListener("click", () => call("stop_monitor"));
  els.stopAlarmBtn.addEventListener("click", () => call("stop_alarm"));
  els.alarmLinkBtn.addEventListener("click", async () => {
    const url = current?.status.alarmLink;
    if (url) await openUrl(url);
  });
  els.muteBtn.addEventListener("click", () => call("mute_for_minutes", { minutes: 30 }));
  els.testAlarmBtn.addEventListener("click", () => call("test_alarm", { mode: "alarm" }));
  els.testGentleBtn.addEventListener("click", () => call("test_alarm", { mode: "gentle" }));

  els.sleepGuard.addEventListener("change", () =>
    call("set_sleep_guard", { enabled: els.sleepGuard.checked }),
  );

  els.autostart.addEventListener("change", async () => {
    if (els.autostart.checked) await enable();
    else await disable();
    await syncAutostartToggle();
  });

  els.openPortalBtn.addEventListener("click", async () => {
    const url = current?.settings.serverUrl || els.serverUrl.value.trim();
    if (url) await openUrl(url);
  });
});
