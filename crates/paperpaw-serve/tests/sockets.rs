//! Plugins' sockets over loopback: reads bounded by what the kernel asks
//! for, data in order across many reads, large sends, calls for sockets
//! that are not open, sockets closed by the kernel staying quiet, and
//! broadcasts with nothing to send.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]
// Tests panic on purpose when something is wrong.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener};
use std::thread;
use std::time::{Duration, Instant};

use paperpaw_core::{SocketCall, SocketEvent};
use paperpaw_serve::Sockets;

/// The next event, waiting up to five seconds.
fn wait(sockets: &mut Sockets) -> SocketEvent {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(event) = sockets.take_event() {
            return event;
        }
        assert!(Instant::now() < until, "no event");
        thread::sleep(Duration::from_millis(2));
    }
}

/// Asserts that no event comes for a while.
fn quiet(sockets: &mut Sockets, millis: u64) {
    let until = Instant::now() + Duration::from_millis(millis);
    while Instant::now() < until {
        if let Some(event) = sockets.take_event() {
            panic!("unexpected {event:?}");
        }
        thread::sleep(Duration::from_millis(5));
    }
}

/// A listener on a free loopback port.
fn listener() -> (TcpListener, u16) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    (listener, port)
}

fn connect(sockets: &mut Sockets, socket: u32, port: u16) {
    sockets.apply(SocketCall::Connect {
        socket,
        host: String::from("127.0.0.1"),
        port,
        tls: None,
    });
    assert_eq!(wait(sockets), SocketEvent::Connected { socket });
}

#[test]
fn calls_for_sockets_that_are_not_open_are_ignored() {
    let mut sockets = Sockets::new(|| {});
    sockets.apply(SocketCall::Send {
        socket: 4,
        data: b"x".to_vec(),
    });
    sockets.apply(SocketCall::Read { socket: 4, max: 9 });
    sockets.apply(SocketCall::Close { socket: 4 });
    assert!(!sockets.any());
    quiet(&mut sockets, 50);
}

#[test]
fn reads_bring_at_most_what_was_asked_for() {
    let (listener, port) = listener();
    let peer = thread::spawn(move || {
        let (mut peer, _) = listener.accept().unwrap();
        peer.write_all(&[7u8; 100]).unwrap();
        // Stay open until the reading is over.
        let mut rest = Vec::new();
        let _ = peer.read_to_end(&mut rest);
    });
    let mut sockets = Sockets::new(|| {});
    connect(&mut sockets, 1, port);
    let mut got = 0;
    while got < 100 {
        sockets.apply(SocketCall::Read { socket: 1, max: 30 });
        let SocketEvent::Data { socket: 1, data } = wait(&mut sockets) else {
            panic!("no data");
        };
        assert!(!data.is_empty() && data.len() <= 30, "{}", data.len());
        assert!(data.iter().all(|&b| b == 7));
        got += data.len();
    }
    assert_eq!(got, 100);
    sockets.apply(SocketCall::Close { socket: 1 });
    peer.join().unwrap();
}

#[test]
fn a_read_asking_for_nothing_takes_one_byte() {
    let (listener, port) = listener();
    let peer = thread::spawn(move || {
        let (mut peer, _) = listener.accept().unwrap();
        peer.write_all(b"ab").unwrap();
        let mut rest = Vec::new();
        let _ = peer.read_to_end(&mut rest);
    });
    let mut sockets = Sockets::new(|| {});
    connect(&mut sockets, 1, port);
    sockets.apply(SocketCall::Read { socket: 1, max: 0 });
    assert_eq!(
        wait(&mut sockets),
        SocketEvent::Data {
            socket: 1,
            data: b"a".to_vec()
        }
    );
    sockets.apply(SocketCall::Close { socket: 1 });
    peer.join().unwrap();
}

#[test]
fn data_comes_in_order_and_ends_empty_when_the_peer_closes() {
    let sent: Vec<u8> = (0..300_000u32).map(|i| (i % 253) as u8).collect();
    let (listener, port) = listener();
    let peer = thread::spawn({
        let sent = sent.clone();
        move || {
            let (mut peer, _) = listener.accept().unwrap();
            for part in sent.chunks(7_777) {
                peer.write_all(part).unwrap();
            }
        }
    });
    let mut sockets = Sockets::new(|| {});
    connect(&mut sockets, 3, port);
    let mut received = Vec::new();
    loop {
        sockets.apply(SocketCall::Read {
            socket: 3,
            max: 65_536,
        });
        match wait(&mut sockets) {
            SocketEvent::Data { socket: 3, data } if data.is_empty() => break,
            SocketEvent::Data { socket: 3, data } => received.extend(data),
            other => panic!("{other:?}"),
        }
    }
    peer.join().unwrap();
    assert!(received == sent, "all bytes, in order");
}

#[test]
fn large_sends_arrive_whole_and_in_order() {
    const LEN: usize = 5 * 100_000;
    let (listener, port) = listener();
    let peer = thread::spawn(move || {
        let (mut peer, _) = listener.accept().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut got = vec![0; LEN];
        peer.read_exact(&mut got).unwrap();
        got
    });
    let mut sockets = Sockets::new(|| {});
    connect(&mut sockets, 5, port);
    let mut expected = Vec::new();
    for i in 0..5u8 {
        let data = vec![i; LEN / 5];
        expected.extend_from_slice(&data);
        sockets.apply(SocketCall::Send { socket: 5, data });
    }
    assert!(peer.join().unwrap() == expected);
    sockets.apply(SocketCall::Close { socket: 5 });
}

#[test]
fn what_was_sent_before_closing_still_arrives() {
    let (listener, port) = listener();
    let peer = thread::spawn(move || {
        let (mut peer, _) = listener.accept().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut got = Vec::new();
        let _ = peer.read_to_end(&mut got);
        got
    });
    let mut sockets = Sockets::new(|| {});
    connect(&mut sockets, 6, port);
    let data = vec![b'q'; 1_000_000];
    sockets.apply(SocketCall::Send {
        socket: 6,
        data: data.clone(),
    });
    sockets.apply(SocketCall::Close { socket: 6 });
    let got = peer.join().unwrap();
    assert_eq!(got.len(), data.len(), "a send followed by a close is lost");
}

#[test]
fn a_socket_closed_by_the_kernel_reports_nothing_more() {
    let (listener, port) = listener();
    let peer = thread::spawn(move || {
        let (mut peer, _) = listener.accept().unwrap();
        thread::sleep(Duration::from_millis(100));
        let _ = peer.write_all(b"too late");
    });
    let mut sockets = Sockets::new(|| {});
    connect(&mut sockets, 2, port);
    sockets.apply(SocketCall::Read { socket: 2, max: 64 });
    sockets.apply(SocketCall::Close { socket: 2 });
    assert!(!sockets.any());
    peer.join().unwrap();
    quiet(&mut sockets, 200);
}

#[test]
fn broadcasts_to_no_ports_are_done_at_once() {
    let mut sockets = Sockets::new(|| {});
    sockets.apply(SocketCall::Broadcast {
        broadcast: 9,
        ports: Vec::new(),
        data: b"hello".to_vec(),
        wait_ms: 0,
    });
    assert!(sockets.any(), "until it is done");
    assert_eq!(wait(&mut sockets), SocketEvent::Done { broadcast: 9 });
    assert!(!sockets.any());
}

#[test]
fn many_broadcasts_are_counted_until_each_is_done() {
    let mut sockets = Sockets::new(|| {});
    for broadcast in 0..3 {
        sockets.apply(SocketCall::Broadcast {
            broadcast,
            ports: Vec::new(),
            data: Vec::new(),
            wait_ms: 10,
        });
    }
    let mut done = Vec::new();
    while done.len() < 3 {
        assert!(sockets.any());
        match wait(&mut sockets) {
            SocketEvent::Done { broadcast } => done.push(broadcast),
            other => panic!("{other:?}"),
        }
    }
    done.sort_unstable();
    assert_eq!(done, [0, 1, 2]);
    assert!(!sockets.any());
}

/// The shell's half of the STARTTLS rule (`SocketCall::StartTls`): a server
/// that sends more right after its go-ahead (an attacker's injection, to
/// pass as secure) fails the switch, `NotReady`, and the socket closes.
/// The kernel cannot see bytes the shell holds unread: only the shell can
/// check this.
#[test]
fn bytes_sent_past_a_starttls_go_ahead_fail_the_switch() {
    use paperpaw_core::TlsSetup;
    use paperpaw_core::sdk::{SocketError, Trust};
    let (listener, port) = listener();
    let server = thread::spawn(move || {
        let (mut peer, _) = listener.accept().unwrap();
        peer.write_all(b"220 Go ahead\r\n* injected\r\n").unwrap();
        // Until the client gives up.
        let _ = peer.read(&mut [0u8; 64]);
    });
    let mut sockets = Sockets::new(|| {});
    connect(&mut sockets, 1, port);
    let go_ahead = b"220 Go ahead\r\n";
    sockets.apply(SocketCall::Read {
        socket: 1,
        max: go_ahead.len() as u32,
    });
    let SocketEvent::Data { data, .. } = wait(&mut sockets) else {
        panic!("no go-ahead");
    };
    assert!(go_ahead.starts_with(&data));
    // Let the injected line arrive behind it, unread.
    thread::sleep(Duration::from_millis(100));
    let tls = TlsSetup {
        server_name: String::from("127.0.0.1"),
        trust: Trust::Authorities,
        timeout_ms: 2_000,
        idle_ms: 0,
    };
    sockets.apply(SocketCall::StartTls { socket: 1, tls });
    assert!(matches!(
        wait(&mut sockets),
        SocketEvent::Failed {
            socket: 1,
            error: SocketError::NotReady,
            ..
        }
    ));
    assert!(!sockets.any());
    server.join().unwrap();
}
