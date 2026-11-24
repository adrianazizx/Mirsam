#!/usr/bin/env bash
# A virtual X display for a Linux CI job, set up the way a desktop login sets
# up a session: starts Xvfb on :99, gives the job's later steps DISPLAY, and
# hands the display to the services the session bus starts on demand.
#
# That last part is what keeps the app's start-up at about a second. GitHub's
# Ubuntu runners have a systemd user session whose bus can start
# xdg-desktop-portal, but that session has no display, so the portal's GTK
# backend exits ("cannot open display") and the portal never answers. The app
# asks it twice at start-up, on its main thread, before the window shows:
# GTK 3's GtkApplication for the Inhibit portal (no session manager runs), and
# gives up after 25 s; then tao for the colour scheme, and gives up after 5 s.
# Measured on 2026-11-24 (ADR 0060's amendment): every launch took 30 s from
# run() to a usable window, about 1 s once the display is handed over. A
# desktop session hands its display over when it starts (portals need it), so
# desktops are not affected.
#
# Usage: scripts/ci-virtual-display.sh   (CI only; GITHUB_ENV must be set)
set -euo pipefail

display=":99"
Xvfb "$display" -screen 0 1600x1200x24 &
echo "DISPLAY=$display" >>"$GITHUB_ENV"

# Both the bus's own activation environment and systemd's user manager, which
# starts the portal (its D-Bus service names a SystemdService).
if ! dbus-update-activation-environment --systemd "DISPLAY=$display"; then
  echo "::warning::could not give the session bus the display; the app's start-up may wait on D-Bus timeouts"
fi
