// The emulator frame: a device drawn into a canvas, its buttons below it,
// touch where it has a touch screen, driven by the page around it through
// postMessage (see docs/web-emulator.md). Runs in an iframe of its own
// origin; it answers the window that embeds it only.
//
// A panic in the module stops it for good (a trap): the frame then makes
// a fresh instance, starts the device again as it was last started, with
// the data plugins it had, and tells the page ({ type: "restarted" }).

import { apply, bezels, keys, margins, place } from "./body.js";
import { KEYS, PaperPawDevice } from "./paperpaw-web.js";

const shell = document.getElementById("device");
const canvas = document.getElementById("screen");
const context = canvas.getContext("2d");
const row = document.getElementById("buttons");
const started = performance.now();

let device = null;
let info = null;
/// The keys drawn on the body, by button; the computer's keys bound to
/// the body's, by key.
let drawn = new Map();
let bound = new Map();
/// How the device was last started, and the data plugins on its card by
/// id (their files): what a restart after a trap starts again.
let last = null;
let plugins = {};
let timer = null;
let drawing = false;
/// The window sets the size of the screen (see `followWindow`), not the
/// device's panel.
let free = false;
/// The recorded answers and whether the page fetches the rest: kept for
/// every device made after (a restart after a trap too).
let network = { fixtures: [], mode: "fixtures" };

const loading = PaperPawDevice.load(new URL("paperpaw-web.wasm", import.meta.url));

function now() {
  return performance.now() - started;
}

/// Runs `f` on the device; a trap restarts it (see above).
function guarded(f) {
  try {
    return f();
  } catch (error) {
    if (error instanceof WebAssembly.RuntimeError) {
      restart(String(error.message ?? error));
    }
    throw error;
  }
}

/// A fresh instance, started again as last time.
async function restart(reason) {
  console.error("paperpaw: the device stopped and starts again:", reason);
  clearTimeout(timer);
  const broken = device;
  device = null;
  if (!broken || !last) return;
  // Out of the device's memory (its heap budget refused an allocation
  // with no way around it): the device would have stopped there too.
  const refused = broken.refused();
  if (refused > 0) tell({ type: "out-of-memory", bytes: refused, device: info?.id });
  try {
    device = await broken.fresh();
    boot(last, plugins);
    tell({ type: "restarted", reason });
  } catch (error) {
    console.error("paperpaw:", error);
  }
}

/// Tells the page around the frame.
/// The origin of the page that embeds the frame: the first that wrote to
/// it. Messages from others are passed over, and what the frame tells
/// (plugins' requests among it) goes to it alone.
let embedder = null;

/// Served on this computer: only then may plugins reach the network
/// through the frame (a local setup; see docs/web-emulator.md).
const LOCAL =
  ["localhost", "127.0.0.1", "[::1]"].includes(location.hostname) || location.hostname.endsWith(".localhost");

function tell(message) {
  if (window.parent === window) return;
  // Before any page wrote: only that the module is ready, or that it
  // could not be loaded, nothing else.
  if (embedder) window.parent.postMessage(message, embedder);
  else if (message.type === "ready" || message.type === "error") window.parent.postMessage(message, "*");
}

/// The body of a device whose profile draws none.
const PLAIN = { bezel: [6, 6, 6, 6], keys: [] };

/// Sizes the canvas on the page, the body around it and the keys on it:
/// each of the device's pixels as a whole number of the screen's own
/// (devicePixelRatio counted), drawn crisp, where it fits at least once;
/// smoothed where it has to be made smaller, as dropping rows and columns
/// would make strokes look bolder or vanish.
function fit() {
  const ratio = window.devicePixelRatio || 1;
  const body = free ? PLAIN : (info?.body ?? PLAIN);
  const [top, right, bottom, left] = margins({ keys: drawn.size && !free ? body.keys : [] });
  const below = row.hidden ? 0 : row.offsetHeight + 20;
  const room = [window.innerWidth - 24 - left - right, window.innerHeight - 24 - top - bottom - below];
  // The body for each of the device's pixels across: its bezels are
  // hundredths of the screen's width.
  const [bt, br, bb, bl] = body.bezel;
  const wide = canvas.width * (1 + (bl + br) / 100);
  const high = canvas.height + (canvas.width * (bt + bb)) / 100;
  const scale = Math.min((room[0] * ratio) / wide, (room[1] * ratio) / high);
  // A free size is made for the room: one screen pixel to a CSS pixel.
  const whole = free ? ratio : scale >= 1 ? Math.floor(scale) : scale;
  const [width, height] = [(canvas.width * whole) / ratio, (canvas.height * whole) / ratio];
  canvas.style.width = `${width}px`;
  canvas.style.height = `${height}px`;
  canvas.style.imageRendering = scale >= 1 ? "pixelated" : "auto";
  const bezel = bezels(body, width);
  shell.style.padding = bezel.map((part) => `${part}px`).join(" ");
  shell.style.margin = [top, right, bottom, left].map((part) => `${part}px`).join(" ");
  place(drawn, width + bezel[1] + bezel[3], height + bezel[0] + bezel[2], bezel);
}

window.addEventListener("resize", () => {
  if (canvas.width) fit();
  if (free) followWindow();
});

/// Where the page's window sets the size of the screen (`free`, see
/// `start`): the screen as large as the room the window leaves, in CSS
/// pixels, bezel of a plain body taken off. Made once the window has kept
/// its size for SETTLE_MS (as the emulator's own window does,
/// shells/desktop/src/resize.rs): every open screen is made anew for it.
const SETTLE_MS = 200;
let settling = null;
function followWindow() {
  clearTimeout(settling);
  settling = setTimeout(() => {
    if (!device || !free) return;
    const [bt, br, bb, bl] = PLAIN.bezel;
    const room = [window.innerWidth - 24, window.innerHeight - 24 - (row.hidden ? 0 : row.offsetHeight + 20)];
    const width = Math.floor(room[0] / (1 + (bl + br) / 100));
    const height = Math.floor(room[1] - (width * (bt + bb)) / 100);
    if (width > 0 && height > 0) {
      guarded(() => device.resize(width, height));
      requestDraw();
    }
  }, SETTLE_MS);
}

/// Draws what the panel shows, and keeps drawing while it changes.
function draw() {
  drawing = false;
  if (!device) return;
  const animating = guarded(() => {
    const [width, height] = device.size();
    if (canvas.width !== width || canvas.height !== height) {
      canvas.width = width;
      canvas.height = height;
      fit();
    }
    // Asked first: a flash ending while the frame is copied still gets
    // the frame after it (its paper) drawn.
    const animating = device.animating();
    context.putImageData(device.frame(), 0, 0);
    return animating;
  });
  if (animating) requestDraw();
}

function requestDraw() {
  if (!drawing) {
    drawing = true;
    requestAnimationFrame(draw);
  }
}

/// Lets the device's time pass, draws, and waits for what is due next.
function step() {
  clearTimeout(timer);
  if (!device) return;
  const next = guarded(() => device.advance(now()));
  requestDraw();
  requests();
  if (next !== null) timer = setTimeout(step, Math.max(next, 1));
}

/// Tells the page every request plugins made, and fetches those it is to.
function requests() {
  for (const seen of guarded(() => device.requests())) tell({ type: "request", ...seen });
  for (const call of guarded(() => device.outbox())) fetchFor(call);
}

/// Bytes written in hexadecimal.
function bytesOf(hex) {
  return Uint8Array.from(hex.match(/../g) ?? [], (pair) => parseInt(pair, 16));
}

/// Fetches a request for the device (real network allowed, see
/// docs/web-emulator.md), within the page's own rules (CORS, its
/// Content-Security-Policy): what it may not reach fails, status 0.
async function fetchFor(call) {
  let status = 0;
  let headers = "";
  let body = new Uint8Array();
  const stop = new AbortController();
  const timeout = setTimeout(() => stop.abort(), call.timeout_ms);
  let url = "";
  try {
    const answer = await fetch(call.url, {
      method: call.method,
      headers: call.headers,
      body: call.body === null ? undefined : bytesOf(call.body),
      signal: stop.signal,
      // The page's own cookies and logins are not the plugin's.
      credentials: "omit",
    });
    status = answer.status;
    headers = [...answer.headers].map(([name, value]) => `${name}: ${value}`).join("\n");
    url = answer.url;
    body = new Uint8Array(await answer.arrayBuffer());
  } catch (error) {
    body = new TextEncoder().encode(String(error.message ?? error));
  } finally {
    clearTimeout(timeout);
  }
  if (!device) return;
  try {
    guarded(() => device.fetched(call.id, status, headers, body, url, now()));
  } catch (error) {
    // The device made anew meanwhile (a start, a restart): its request
    // is gone with it.
    if (error instanceof WebAssembly.RuntimeError) throw error;
    return;
  }
  step();
}

/// The device's keys: on its body where its profile places them, else
/// in a row below it (the row it has, else all its buttons); none where
/// the page said so (`buttons: false`). The computer's keys the profile
/// binds press them either way.
function buttons(shown) {
  const body = info.body ?? PLAIN;
  bound = new Map(body.keys.flatMap((key) => key.keys.map((pressed) => [pressed, key.button])));
  for (const key of shell.querySelectorAll(".key")) key.remove();
  drawn = shown ? keys(shell, body, press) : new Map();
  row.replaceChildren();
  row.hidden = !shown || body.keys.length > 0;
  if (row.hidden) return;
  const names = info.row.length ? info.row : info.buttons;
  for (const name of names) {
    const button = document.createElement("button");
    button.textContent = name;
    button.dataset.button = name;
    button.addEventListener("pointerdown", () => press(name, true));
    button.addEventListener("pointerup", () => press(name, false));
    button.addEventListener("pointerleave", (event) => {
      if (event.buttons) press(name, false);
    });
    row.append(button);
  }
}

function press(name, pressed) {
  if (device && guarded(() => device.button(name, pressed, now()))) step();
}

/// Where a pointer event is on the device's screen, in its pixels.
function at(event) {
  const rect = canvas.getBoundingClientRect();
  return [((event.clientX - rect.left) * canvas.width) / rect.width, ((event.clientY - rect.top) * canvas.height) / rect.height];
}

for (const [type, phase] of [
  ["pointerdown", "down"],
  ["pointermove", "move"],
  ["pointerup", "up"],
  ["pointercancel", "cancel"],
]) {
  canvas.addEventListener(type, (event) => {
    if (!device || !info.touch || (phase === "move" && !event.buttons)) return;
    if (phase === "down") canvas.setPointerCapture(event.pointerId);
    guarded(() => device.pointer(phase, ...at(event), now()));
    step();
  });
}

for (const type of ["keydown", "keyup"]) {
  window.addEventListener(type, (event) => {
    // The browser's own shortcuts stay its own.
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    // A key of the device reached by the keyboard: Enter and Space press
    // it (one clicked does not keep them).
    const focused = event.target instanceof Element ? event.target.closest("[data-button]") : null;
    const own =
      focused?.matches(":focus-visible") && (event.key === "Enter" || event.key === " ") ? focused.dataset.button : null;
    // Letters as they are bound, Caps Lock or Shift or not.
    const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
    const name = own ?? bound.get(key) ?? KEYS[key];
    if (!name || !device) return;
    event.preventDefault();
    // Held: pressed once, as the device's buttons are.
    if (event.repeat) return;
    press(name, type === "keydown");
  });
}

/// Makes the device of `how` on `device`, puts `given` (files by plugin
/// id) on its card and starts it.
function boot(how, given) {
  device.create(how);
  device.setFixtures(network.fixtures);
  device.setNetwork(network.mode);
  plugins = {};
  for (const [id, files] of Object.entries(given)) put(id, files);
  device.start();
  info = device.device();
  buttons(how.buttons ?? true);
  if (canvas.width) fit();
  step();
  if (free) followWindow();
}

/// Starts a device: the files go to `plugins/<id>/` on its card.
async function start({ profile, language, dark, buttons, look, free: sized = false, plugins: given = [] }) {
  device = await loading;
  apply(look);
  free = sized;
  last = { profile, language, dark, buttons };
  boot(last, Object.fromEntries(given.map((plugin) => [plugin.id, plugin.files])));
  return { device: info };
}

function put(id, files) {
  for (const [path, bytes] of Object.entries(files)) device.putFile(id, path, bytes);
  plugins[id] = files;
}

/// Replaces a data plugin's files, whatever of it was on the card, and
/// starts the device again.
function files({ id, files: given }) {
  device.removePlugin(id);
  delete plugins[id];
  put(id, given);
  device.start();
  step();
  return {};
}

async function input({ event }) {
  if (event.kind === "button" && event.hold) {
    // A long press: held `hold` milliseconds, then let go.
    press(event.name, true);
    await new Promise((resolve) => setTimeout(resolve, event.hold));
    press(event.name, false);
  } else if (event.kind === "button") {
    press(event.name, event.pressed);
  } else if (event.kind === "key") {
    const name = KEYS[event.key];
    if (name) press(name, event.pressed ?? true);
  } else if (event.kind === "pointer") {
    guarded(() => device.pointer(event.phase, event.x, event.y, now()));
    step();
  }
  return {};
}

/// The panel as a PNG, at the device's own resolution; of another device,
/// night mode or not, or the sleep screen where asked, what is shown left
/// as it is.
async function screenshot({ profile, dark, asleep }) {
  const apart = profile !== undefined || dark !== undefined || asleep;
  const image = apart ? device.snapshot({ profile, dark, asleep }) : device.shown();
  const offscreen = new OffscreenCanvas(image.width, image.height);
  offscreen.getContext("2d").putImageData(image, 0, 0);
  const blob = await offscreen.convertToBlob({ type: "image/png" });
  return { png: new Uint8Array(await blob.arrayBuffer()) };
}

function check({ commands, plugins }) {
  return { report: device.check(commands ?? [], plugins ?? []) };
}

/// Text as the remote's lines carry it: hexadecimal, `-` for none.
function hex(text) {
  const bytes = new TextEncoder().encode(String(text ?? ""));
  return bytes.length ? Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("") : "-";
}

/// Parameters as the remote's lines carry them: `key=hex` words.
function pairs(params) {
  return Object.entries(params ?? {}).map(([key, value]) => `${key}=${hex(value)}`);
}

/// The remote control's command, an x4-remote line (see
/// docs/web-emulator.md); its answer.
function command({ line }) {
  const answer = guarded(() => device.command(line, now()));
  step();
  return { answer };
}

/// The same, built from a message's fields.
const line = (words) => command({ line: words.join(" ") });
const commands = {
  command,
  describe: () => command({ line: "describe" }).answer,
  run: ({ action }) => line(["run", action]),
  slot: ({ slot, params }) => line(["slot", slot, ...pairs(params)]),
  show: ({ screen, params }) => line(["show", screen, ...pairs(params)]),
  open: ({ path }) => line(["open", hex(path)]),
  set: ({ setting, value }) => line(["set", setting, hex(value)]),
  emit: ({ event, params }) => line(["emit", event, ...pairs(params)]),
  /// Text into the text field shown, as if typed; confirmed with `enter`.
  type: ({ text, enter }) => line([enter ? "type-enter" : "type", hex(text)]),
  /// Waits until the screen is `text` (its id) or shows it, at most
  /// `timeout` ms of the page's time, the device running meanwhile.
  until: async ({ text, timeout = 30000 }) => {
    const end = now() + timeout;
    for (;;) {
      step();
      if (guarded(() => device.shows(text))) return {};
      if (now() >= end) throw new Error(`not shown in ${timeout} ms: ${text}`);
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
  },
  expect: ({ text }) => {
    if (!guarded(() => device.shows(text))) throw new Error(`not shown: ${text}`);
    return {};
  },
};

/// Recorded answers for plugins' requests, kept for devices made after.
function fixtures({ fixtures: given = [] }) {
  network.fixtures = given;
  device?.setFixtures(given);
  return {};
}

/// Whether the page fetches what no recorded answer answers.
function networkMode({ mode }) {
  if (mode !== "fixtures" && mode !== "real") throw new Error(`no network mode ${mode}`);
  if (mode === "real" && !LOCAL) {
    throw new Error("the real network only where the frame is served on this computer (localhost)");
  }
  network.mode = mode;
  device?.setNetwork(mode);
  return {};
}

const HANDLERS = { start, files, input, screenshot, check, fixtures, network: networkMode, ...commands };

window.addEventListener("message", async (event) => {
  if (event.source !== window.parent || event.source === window) return;
  // The page that wrote first is the one the frame serves.
  embedder ??= event.origin;
  if (event.origin !== embedder) return;
  const message = event.data ?? {};
  const handler = HANDLERS[message.type];
  // `request` pairs the answer with the message (`id` names a plugin).
  const reply = (answer) => event.source.postMessage({ type: message.type, request: message.request, ...answer }, event.origin);
  if (!handler) return reply({ error: `unknown message type ${message.type}` });
  const before = ["start", "fixtures", "network"];
  if (!before.includes(message.type) && !device) return reply({ error: "no device started yet" });
  try {
    reply(await handler(message));
  } catch (error) {
    if (error instanceof WebAssembly.RuntimeError) restart(String(error.message ?? error));
    reply({ error: String(error.message ?? error) });
  }
});

// Ready for messages once the module is loaded.
loading.then(
  (loaded) => tell({ type: "ready", version: loaded.version() }),
  (error) => {
    console.error("paperpaw:", error);
    tell({ type: "error", error: String(error.message ?? error) });
  },
);

// Opened on its own (not in a frame): a device to try, from the address
// (`frame.html?profile=boox-go-7&language=de&dark`; `&free` makes the screen
// as large as the window leaves room for).
if (window.parent === window) {
  const query = new URLSearchParams(location.search);
  start({
    profile: query.get("profile") ?? "xteink-x4",
    language: query.get("language") ?? "en",
    dark: query.has("dark"),
    free: query.has("free"),
  }).catch((error) => console.error("paperpaw:", error));
}
