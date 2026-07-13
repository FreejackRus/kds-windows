# KDS Guard for Windows

Desktop companion for KDS built with Tauri.

## What works in the first MVP

- Polls `https://staff.tkgse.ru/rest/staff/alarm` or a configured server URL.
- Supports manual `Cookie` header and Basic Auth credentials.
- Keeps Windows awake while the guard is enabled using `SetThreadExecutionState`.
- Plays looping alarm or gentle alarm sounds from bundled `.ogg` files.
- Continues running while the Windows session is locked, as long as the user remains logged in.
- Offers mute for 30 minutes, stop sound, and test sound actions.
- Can be registered for autostart through the Tauri autostart plugin.
- Keeps running in the system tray when the main window is closed.
- Restarts the monitor worker through a watchdog if it panics.
- Shows alarm links returned by the API in the main window.

## Run on Windows

Install prerequisites:

- Node.js 20+
- Rust toolchain from `https://rustup.rs`
- Microsoft C++ Build Tools
- WebView2 Runtime

Then run:

```powershell
npm install
npm run tauri dev
```

Build installer:

```powershell
npm run tauri build
```

## Current limitations

- Drupal login UI is not implemented yet. For now, paste a valid cookie header or use Basic Auth for dev servers.
- The app is a user-session app, not a Windows service. It can sound while the workstation is locked, but not after user logoff.
