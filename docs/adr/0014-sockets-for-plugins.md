# 0014. Sockets for plugins

- Status: Accepted; the X4 now has sockets too
  (`shells/xteink/src/sockets.rs`); TLS for them in the emulator and
  the Android app is rustls with ring (`crates/paperpaw-serve/src/tls`,
  owner decision 52)
- Date: 2026-10-16

## Context

Plugins reach the network through HTTP (`Command::Http`) and serve HTTP to
it (`Command::Serve`). Some services speak neither: Calibre's wireless
device connection, the most common way to send books to a reader, is a
TCP connection the device opens to Calibre, found by a UDP broadcast,
with JSON messages and books as raw bytes between them. Others will
follow (Syncthing-like tools, other companion apps).

The kernel is sans-I/O ([ADR 0003](0003-sans-io-kernel.md)): it never
touches the network itself. Books sent over a connection can be larger
than the memory of small devices, so they must go to storage as they
come. Connections cost battery on e-ink devices and must not outlive
what the user is doing.

## Decision

- **Commands for plugins**, in the manner of servers:
  - `Command::Connect { socket, host, port, events }` opens a TCP
    connection. Everything about it comes as the event `events`
    (`socket_params`): connected, received, saved, closed, failed.
  - `Command::SocketSend { socket, data }` sends bytes.
  - `Command::SocketRead { socket, read }` asks for what comes next:
    `SocketRead::Text { max }`, up to `max` bytes as text (for protocol
    messages, which are text), or `SocketRead::File { path, len }`,
    exactly `len` bytes written to `path` in storage (for books). One read
    at a time per socket; nothing arrives unasked, so a plugin reads at
    its own pace.
  - `Command::Disconnect(socket)` closes it.
  - `Command::broadcast(id, ports, data, wait_ms, events)` sends a UDP
    datagram to the local network on each of `ports` and passes the
    answers that come within `wait_ms` on as events, then that it is
    done.
- **The kernel** keeps the sockets of each plugin, numbers them for the
  shell (`Io::Socket`, `Kernel::take_io`) and turns what the
  shell reports (`SocketEvent`, `Kernel::socket_event`) into the
  plugin's events. A file read is asked of the shell in parts of 64 KiB
  and each part appended to the file, so a book never needs to fit in
  memory. Without a network every command fails at once with an event.
- **Sockets end with their screen**, like servers: closing the screen
  that was showing when a socket was opened closes it. *Amended
  2026-11 (#81, owner decision 52):* a socket that background work opens
  (a work's event handlers, a job's step; ADR 0025) is that run's
  instead and closes when the run ends: the job done or failed, the work
  run again, taken back or refused, or its plugin gone. Its secure
  sockets end when idle (`idle_ms(0)` refused, at most 30 minutes).
- **Shells with the standard library** (the emulator, Android) do it in
  `paperpaw-serve`: a thread per connection for reading, one for writing,
  bounded channels between them and the shell, so a fast network waits
  for a slow storage. Shells without a network (the X4 for now) report
  no network, so the commands fail.

## Consequences

- A plugin can speak any protocol over TCP, find services by UDP
  broadcast and receive files of any size, testable without a network:
  the kernel's part with events made up in tests, the shell's part over
  local connections.
- Text reads are UTF-8 (invalid bytes replaced): binary protocols read
  into files. That keeps events as they are (text parameters).
- The kernel holds at most one read's part (64 KiB) per socket in memory.
