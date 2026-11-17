// Launcher mod for herdr-agents-status.
// Opened with mod.open from keys.json (e.g. "ctrl-t o": "mod.open:agents-status").
// It launches herdr-agents-status (running it again toggles the overlay off)
// and then closes its own 1x1 popup immediately.
import { View, div } from "gpui-kit";
import { exec, props, closeOverlay, checking, notify } from "herdr";

// Command and extra arguments (e.g. ["--config", "/path/to/config.toml"])
const BIN = "herdr-agents-status";
const ARGS = [];

// Single-quote for a POSIX shell
const quote = (s) => `'${s.replaceAll("'", `'\\''`)}'`;

export default class AgentsStatusLauncher extends View {
  init(_props, _cx) {
    if (checking()) return;
    // herdr-app's own environment is the desktop session's, which may lack PATH entries set up in the shell's
    // rc file (e.g. mise, where herdr may live). Run it through the user's shell as an interactive shell
    // ($SHELL -ic), like herdr-core starts terminals, so it sees the same environment as a terminal.
    // setsid -f detaches it; redirect stdio so exec() doesn't wait for the GUI to exit (it would hold the pipes open)
    const command = ["setsid", "-f", BIN, ...ARGS].map(quote).join(" ");
    exec("sh", ["-c", '"${SHELL:-/bin/sh}" -ic "$0" >/dev/null 2>&1 </dev/null', command])
      .catch((e) => notify({ title: "agents-status", body: `Could not launch ${BIN}: ${e}`, level: "error" }));
    closeOverlay(props().overlay);
  }

  render() {
    return div();
  }
}
